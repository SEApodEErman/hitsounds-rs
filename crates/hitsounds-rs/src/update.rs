//! Update checks from GitHub Releases (D-84). The check and the download run on their
//! own thread and never block the UI. A downloaded update is applied after the window
//! has closed (`apply_on_exit`): the installed app runs the new setup silently, the
//! portable app swaps its files in place.

use eframe::egui::Context;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// `owner/name` of the repository whose Releases are checked. The release workflow sets
/// it at build time (D-85); builds from source have none and never update themselves.
pub const REPO: Option<&str> = option_env!("HITSOUNDS_RS_UPDATE_REPO");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const EXE: &str = "hitsounds-rs.exe";
/// Where a portable update is unpacked, next to the exe so the swap is a rename.
const STAGING: &str = ".hitsounds-rs-update";
/// A release asset larger than this is not an update of this app.
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

/// How this copy of the app was installed, which decides the asset it downloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    /// Installed by the Inno Setup installer (its uninstaller is next to the exe).
    Installed,
    /// Unzipped from the portable archive.
    Portable,
}

impl Install {
    fn detect(dir: &Path) -> Self {
        if dir.join("unins000.exe").is_file() {
            Self::Installed
        } else {
            Self::Portable
        }
    }

    /// The release asset for `version`, named by the release workflow (D-85).
    fn asset_name(self, version: &Version) -> String {
        match self {
            Self::Installed => format!("hitsounds-rs-{version}-setup.exe"),
            Self::Portable => format!("hitsounds-rs-{version}-portable.zip"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Ready {
    pub version: String,
    /// The release page, for its notes.
    pub notes: String,
    file: PathBuf,
    install: Install,
}

#[derive(Clone, Debug, Default)]
pub enum Status {
    #[default]
    Idle,
    /// Updates can't work for this copy, and why.
    Unavailable(&'static str),
    Checking,
    UpToDate,
    Downloading {
        version: String,
        done: u64,
        total: u64,
    },
    Ready(Ready),
    Failed(String),
}

impl Status {
    pub fn busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Downloading { .. })
    }

    pub fn ready(&self) -> Option<&Ready> {
        match self {
            Self::Ready(ready) => Some(ready),
            _ => None,
        }
    }
}

static STATUS: Mutex<Status> = Mutex::new(Status::Idle);
static RUNNING: AtomicBool = AtomicBool::new(false);
static RELAUNCH: AtomicBool = AtomicBool::new(false);
/// Mirrors the app setting, so turning checks off also drops a downloaded update.
static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn status() -> Status {
    STATUS.lock().map(|s| s.clone()).unwrap_or_default()
}

fn set(status: Status) {
    if let Ok(mut current) = STATUS.lock() {
        *current = status;
    }
}

/// Start the app again after the update is applied ("Restart to update").
pub fn set_relaunch(relaunch: bool) {
    RELAUNCH.store(relaunch, Ordering::Relaxed);
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Checks for a newer release after `delay` and downloads it, on its own thread.
/// Does nothing while a check runs or once an update is ready.
pub fn check(ctx: &Context, delay: Duration) {
    let Some(repo) = REPO else {
        set(Status::Unavailable(
            "Builds made from source don't update themselves.",
        ));
        return;
    };
    let Some(dir) = exe_dir() else {
        set(Status::Unavailable(
            "hitsounds-rs could not find its own folder.",
        ));
        return;
    };
    let install = Install::detect(&dir);
    if install == Install::Portable && !writable(&dir) {
        set(Status::Unavailable(
            "hitsounds-rs can't write to its folder, so get new versions from the releases page.",
        ));
        return;
    }
    let Some(cache) = crate::worker::config_folder().map(|d| d.join("updates")) else {
        set(Status::Unavailable(
            "No app config folder is available for downloads.",
        ));
        return;
    };
    if status().ready().is_some() || RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    set(Status::Checking);
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("update check".into())
        .spawn(move || {
            std::thread::sleep(delay);
            set(Status::Checking);
            ctx.request_repaint();
            let result = run(repo, install, &cache, &ctx);
            set(match result {
                Ok(Some(ready)) => Status::Ready(ready),
                Ok(None) => Status::UpToDate,
                Err(error) => Status::Failed(error),
            });
            RUNNING.store(false, Ordering::Release);
            ctx.request_repaint();
        });
    if spawned.is_err() {
        RUNNING.store(false, Ordering::Release);
        set(Status::Failed("The update check could not start.".into()));
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
    /// `sha256:<hex>`, which GitHub computes when the asset is uploaded.
    #[serde(default)]
    digest: Option<String>,
}

/// The newest release above `current` that has an asset for `install`. Prereleases
/// count only while `current` is one, so alpha testers move on to the stable
/// release but a stable install never gets an alpha.
fn newest<'a>(
    releases: &'a [Release],
    current: &Version,
    install: Install,
) -> Option<(Version, &'a Release, &'a Asset)> {
    releases
        .iter()
        .filter(|r| !r.draft && (!r.prerelease || !current.pre.is_empty()))
        .filter_map(|r| {
            let version =
                Version::parse(r.tag_name.strip_prefix('v').unwrap_or(&r.tag_name)).ok()?;
            if version <= *current || (!version.pre.is_empty() && current.pre.is_empty()) {
                return None;
            }
            let name = install.asset_name(&version);
            let asset = r.assets.iter().find(|a| a.name == name)?;
            Some((version, r, asset))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!("hitsounds-rs/{VERSION}"))
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(15 * 60)))
        .build()
        .into()
}

fn net(error: ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(403 | 429) => {
            "GitHub is limiting requests from this network. Try again later.".into()
        }
        ureq::Error::StatusCode(404) => "The releases page could not be found.".into(),
        error => format!("Could not reach GitHub ({error})."),
    }
}

fn run(repo: &str, install: Install, cache: &Path, ctx: &Context) -> Result<Option<Ready>, String> {
    let current = Version::parse(VERSION).map_err(|e| e.to_string())?;
    let agent = agent();
    let releases: Vec<Release> = agent
        .get(format!(
            "https://api.github.com/repos/{repo}/releases?per_page=30"
        ))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(net)?
        .body_mut()
        .read_json()
        .map_err(net)?;
    let Some((version, release, asset)) = newest(&releases, &current, install) else {
        let _ = fs::remove_dir_all(cache);
        return Ok(None);
    };
    // Downloads only ever come from this repository's release files.
    let prefix = format!("https://github.com/{repo}/releases/download/");
    if !asset.browser_download_url.starts_with(&prefix) {
        return Err("The release file is not on GitHub Releases.".into());
    }
    if asset.size > MAX_DOWNLOAD {
        return Err("The release file is unexpectedly large.".into());
    }
    let expected = match asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
    {
        Some(hex) => hex.to_ascii_lowercase(),
        None => listed_sha256(&agent, release, &asset.name, &prefix)?,
    };
    let dir = cache.join(version.to_string());
    remove_others(cache, &dir);
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create the download folder: {e}"))?;
    let file = dir.join(install.asset_name(&version));
    let have = file.is_file() && sha256_file(&file).is_ok_and(|hex| hex == expected);
    if !have {
        download(&agent, asset, &version, &file, &expected, ctx)?;
    }
    Ok(Some(Ready {
        version: version.to_string(),
        notes: release.html_url.clone(),
        file,
        install,
    }))
}

/// For assets without a GitHub digest: the line for `name` in the release's
/// `SHA256SUMS.txt`, which the release workflow uploads (D-85).
fn listed_sha256(
    agent: &ureq::Agent,
    release: &Release,
    name: &str,
    prefix: &str,
) -> Result<String, String> {
    let missing = || "The release has no checksum for its files.".to_string();
    let sums = release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS.txt" && a.browser_download_url.starts_with(prefix))
        .ok_or_else(missing)?;
    let text = agent
        .get(&sums.browser_download_url)
        .call()
        .map_err(net)?
        .body_mut()
        .read_to_string()
        .map_err(net)?;
    parse_sums(&text, name).ok_or_else(missing)
}

/// `sha256sum` format: `<hex>  <name>` (or `<hex> *<name>`).
fn parse_sums(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (hex, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        (file == name && hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hex.to_ascii_lowercase())
    })
}

fn download(
    agent: &ureq::Agent,
    asset: &Asset,
    version: &Version,
    file: &Path,
    expected: &str,
    ctx: &Context,
) -> Result<(), String> {
    let progress = |done| {
        set(Status::Downloading {
            version: version.to_string(),
            done,
            total: asset.size,
        });
        ctx.request_repaint();
    };
    progress(0);
    let part = file.with_extension("part");
    let failed = |e: io::Error| format!("The download failed: {e}");
    let mut response = agent.get(&asset.browser_download_url).call().map_err(net)?;
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .reader();
    let mut out = io::BufWriter::new(fs::File::create(&part).map_err(failed)?);
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    let (mut done, mut shown) = (0u64, 0u64);
    loop {
        let n = reader.read(&mut buf).map_err(failed)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(failed)?;
        done += n as u64;
        if done - shown >= 512 * 1024 {
            shown = done;
            progress(done);
        }
    }
    out.flush().map_err(failed)?;
    drop(out);
    let actual = hex(&hasher.finalize());
    if done != asset.size || actual != expected {
        let _ = fs::remove_file(&part);
        return Err("The downloaded file did not match its checksum, so it was discarded.".into());
    }
    fs::rename(&part, file).map_err(failed)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

/// Downloads of other versions are no longer needed.
fn remove_others(cache: &Path, keep: &Path) {
    if let Ok(entries) = fs::read_dir(cache) {
        for entry in entries.flatten() {
            if entry.path() != keep {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".hitsounds-rs-write-test");
    let ok = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

/// Removes what a portable update left behind: the old files it renamed and its
/// staging folder. Runs at startup, when the old exe is no longer running.
pub fn tidy() {
    let Some(dir) = exe_dir() else { return };
    let _ = fs::remove_dir_all(dir.join(STAGING));
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "old") && path.is_file() {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// Applies a downloaded update once the window has closed. A failure is reported
/// and the update is tried again at the next exit.
pub fn apply_on_exit() {
    let Some(ready) = status().ready().cloned() else {
        return;
    };
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let relaunch = RELAUNCH.load(Ordering::Relaxed);
    if other_instances() > 0 {
        if relaunch {
            message(&format!(
                "hitsounds-rs {} will install when every hitsounds-rs window is closed.",
                ready.version
            ));
        }
        return;
    }
    let result = match ready.install {
        Install::Installed => run_installer(&ready.file, relaunch).map_err(|e| e.to_string()),
        Install::Portable => swap_portable(&ready.file, relaunch),
    };
    if let Err(error) = result {
        message(&format!(
            "hitsounds-rs {} could not be installed: {error}\n\nhitsounds-rs will try again the next time it closes.",
            ready.version
        ));
    }
}

fn message(text: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("hitsounds-rs update")
        .set_description(text)
        .show();
}

/// The installer is per user (no UAC prompt) and closes this process if it is still
/// exiting (D-85).
fn run_installer(setup: &Path, relaunch: bool) -> io::Result<()> {
    let mut command = Command::new(setup);
    command.args([
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART",
        "/CLOSEAPPLICATIONS",
        "/SP-",
    ]);
    command.arg(format!("/LOG={}", setup.with_extension("log").display()));
    if relaunch {
        command.arg("/relaunch=1");
    }
    command.spawn().map(drop)
}

/// Unpacks the portable archive next to the exe and swaps each file in by renaming
/// the current one to `.old` (Windows allows renaming a running exe or loaded DLL).
/// Any failure puts back the files already swapped.
fn swap_portable(archive: &Path, relaunch: bool) -> Result<(), String> {
    let dir = exe_dir().ok_or("hitsounds-rs could not find its own folder.")?;
    let staging = dir.join(STAGING);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let names = unpack(archive, &staging).inspect_err(|_| {
        let _ = fs::remove_dir_all(&staging);
    })?;
    if !names.iter().any(|n| n == EXE) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("the archive has no {EXE}."));
    }
    let mut swapped: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let result = names.iter().try_for_each(|name| -> io::Result<()> {
        let target = dir.join(name);
        let old = dir.join(format!("{name}.old"));
        let _ = fs::remove_file(&old);
        let kept = if target.exists() {
            fs::rename(&target, &old)?;
            Some(old)
        } else {
            None
        };
        swapped.push((target.clone(), kept));
        fs::rename(staging.join(name), &target)
    });
    if let Err(error) = result {
        for (target, old) in swapped.into_iter().rev() {
            let _ = fs::remove_file(&target);
            if let Some(old) = old {
                let _ = fs::rename(old, &target);
            }
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(error.to_string());
    }
    let _ = fs::remove_dir_all(&staging);
    if relaunch {
        Command::new(dir.join(EXE))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Extracts the archive's files into `into`. The portable archive is flat, so any
/// entry in a folder (or with an unsafe path) is refused.
fn unpack(archive: &Path, into: &Path) -> Result<Vec<String>, String> {
    let bad = |e: zip::result::ZipError| format!("the archive is unreadable ({e}).");
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(bad)?;
    let mut names = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(bad)?;
        if entry.is_dir() {
            return Err("the archive has folders, which the portable build never has.".into());
        }
        let name = entry
            .enclosed_name()
            .filter(|p| p.components().count() == 1)
            .and_then(|p| p.to_str().map(str::to_owned))
            .ok_or("the archive has a file with an unsafe path.")?;
        let mut out = fs::File::create(into.join(&name)).map_err(|e| e.to_string())?;
        io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        names.push(name);
    }
    Ok(names)
}

/// Other running copies of hitsounds-rs.exe. The installer would close them, which
/// could lose their unsaved work, so the update waits for them.
fn other_instances() -> usize {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };
    let me = std::process::id();
    let mut count = 0;
    // SAFETY: the snapshot handle is checked and closed, and `entry` has its size set.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return 0;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if entry.th32ProcessID != me && name.eq_ignore_ascii_case(EXE) {
                count += 1;
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    count
}

/// Opens a release page in the default browser.
pub fn open_notes(url: &str) {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    if !url.starts_with("https://github.com/") {
        return;
    }
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(url));
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// What App settings says about updates.
pub fn describe(status: &Status) -> String {
    match status {
        Status::Idle => "Not checked yet.".into(),
        Status::Unavailable(why) => (*why).into(),
        Status::Checking => "Checking for updates…".into(),
        Status::UpToDate => format!("hitsounds-rs {VERSION} is the latest version."),
        Status::Downloading {
            version,
            done,
            total,
        } => {
            let percent = (*done as f64 / (*total).max(1) as f64 * 100.0).min(100.0);
            format!("Downloading hitsounds-rs {version}… {percent:.0} %")
        }
        Status::Ready(ready) => format!(
            "hitsounds-rs {} is downloaded and installs when you close the app.",
            ready.version
        ),
        Status::Failed(error) => format!("The last check failed. {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: format!("https://github.com/o/r/releases/tag/{tag}"),
            draft: false,
            prerelease,
            assets: assets
                .iter()
                .map(|name| Asset {
                    name: (*name).into(),
                    size: 1,
                    browser_download_url: format!(
                        "https://github.com/o/r/releases/download/{tag}/{name}"
                    ),
                    digest: None,
                })
                .collect(),
        }
    }

    fn both(version: &str) -> [String; 2] {
        [
            format!("hitsounds-rs-{version}-setup.exe"),
            format!("hitsounds-rs-{version}-portable.zip"),
        ]
    }

    fn pick(releases: &[Release], current: &str, install: Install) -> Option<String> {
        newest(releases, &Version::parse(current).unwrap(), install).map(|(v, _, _)| v.to_string())
    }

    #[test]
    fn an_alpha_moves_to_newer_alphas_and_the_stable_release() {
        let a2 = both("0.1.0-alpha.2");
        let stable = both("0.1.0");
        let releases = [
            release("v0.1.0-alpha.2", true, &[&a2[0], &a2[1]]),
            release("v0.1.0", false, &[&stable[0], &stable[1]]),
        ];
        assert_eq!(
            pick(&releases[..1], "0.1.0-alpha.1", Install::Installed).as_deref(),
            Some("0.1.0-alpha.2")
        );
        assert_eq!(
            pick(&releases, "0.1.0-alpha.1", Install::Portable).as_deref(),
            Some("0.1.0")
        );
    }

    #[test]
    fn an_alpha_moves_on_to_the_beta() {
        let b1 = both("0.1.0-beta.1");
        let releases = [release("v0.1.0-beta.1", true, &[&b1[0], &b1[1]])];
        for install in [Install::Installed, Install::Portable] {
            assert_eq!(
                pick(&releases, "0.1.0-alpha.2", install).as_deref(),
                Some("0.1.0-beta.1")
            );
        }
        assert_eq!(pick(&releases, "0.1.0-beta.1", Install::Installed), None);
    }

    #[test]
    fn a_stable_install_never_gets_a_prerelease() {
        let a = both("0.2.0-alpha.1");
        let releases = [release("v0.2.0-alpha.1", true, &[&a[0], &a[1]])];
        assert_eq!(pick(&releases, "0.1.0", Install::Installed), None);
        // Even one that was not marked as a prerelease on GitHub.
        let releases = [release("v0.2.0-alpha.1", false, &[&a[0], &a[1]])];
        assert_eq!(pick(&releases, "0.1.0", Install::Installed), None);
    }

    #[test]
    fn drafts_older_versions_and_releases_without_the_asset_are_skipped() {
        let new = both("0.3.0");
        let mut draft = release("v0.4.0", false, &[]);
        draft.draft = true;
        let releases = [
            draft,
            release("v0.0.9", false, &[]),
            release("v0.3.0", false, &[&new[0]]),
            release("not-a-version", false, &[]),
        ];
        assert_eq!(
            pick(&releases, "0.2.0", Install::Installed).as_deref(),
            Some("0.3.0")
        );
        assert_eq!(pick(&releases, "0.2.0", Install::Portable), None);
        assert_eq!(pick(&releases, "0.3.0", Install::Installed), None);
    }

    #[test]
    fn sums_are_read_in_sha256sum_format() {
        let hex = "ab".repeat(32);
        let text = format!(
            "{}  other.zip\n{} *hitsounds-rs-0.1.0-setup.exe\n",
            "cd".repeat(32),
            hex.to_uppercase()
        );
        assert_eq!(parse_sums(&text, "hitsounds-rs-0.1.0-setup.exe"), Some(hex));
        assert_eq!(parse_sums(&text, "missing.zip"), None);
        assert_eq!(parse_sums("short  a.zip", "a.zip"), None);
    }

    #[test]
    fn the_portable_archive_must_be_flat() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("hitsounds-rs.exe", options).unwrap();
        zip.write_all(b"exe").unwrap();
        zip.start_file("../evil.dll", options).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        let into = dir.path().join("out");
        fs::create_dir(&into).unwrap();
        assert!(unpack(&archive, &into).is_err());
        assert!(!dir.path().join("evil.dll").exists());
    }

    #[test]
    fn a_flat_archive_unpacks() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for name in ["hitsounds-rs.exe", "onnxruntime.dll"] {
            zip.start_file(name, options).unwrap();
            zip.write_all(name.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let into = dir.path().join("out");
        fs::create_dir(&into).unwrap();
        assert_eq!(
            unpack(&archive, &into).unwrap(),
            ["hitsounds-rs.exe", "onnxruntime.dll"]
        );
        assert_eq!(
            fs::read(into.join("onnxruntime.dll")).unwrap(),
            b"onnxruntime.dll"
        );
    }
}
