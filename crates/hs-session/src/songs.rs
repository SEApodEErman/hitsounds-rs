//! The osu! Songs folder as a list of mapsets, for the mapset picker (D-82).
//!
//! A scan takes each mapset's metadata from osu!'s own `osu!.db` when it lists the
//! folder, else from the header of each `.osu` file (`[General]` and `[Metadata]`),
//! and keeps the result in a small cache file (`songs.json` in the app's config
//! folder). A later scan lists each mapset folder again, but reads its headers only
//! when the folder's `.osu` files changed (name, size or modified time), so opening
//! the picker again is quick even for a large Songs folder.

use crate::osudb;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Bump when the cached fields change; an older cache is then read from scratch.
const CACHE_VERSION: u32 = 1;
/// A header longer than this is not an osu! header; stop reading.
const HEADER_LIMIT: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffEntry {
    /// The `.osu` file name.
    pub file: String,
    /// `[Metadata] Version`.
    pub version: String,
    /// `[General] Mode`: 0 standard, 1 taiko, 2 catch, 3 mania.
    pub mode: u8,
}

impl DiffEntry {
    /// hitsounds-rs edits standard and catch difficulties (D-04).
    pub fn editable(&self) -> bool {
        matches!(self.mode, 0 | 2)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapsetEntry {
    /// The folder relative to the Songs folder, with `/` between parts.
    pub folder: String,
    /// Changes when a `.osu` file in the folder is added, removed or rewritten.
    pub fingerprint: u64,
    /// The newest `.osu` modified time, in Unix milliseconds.
    pub modified: u64,
    pub artist: String,
    pub title: String,
    pub creator: String,
    /// `BeatmapSetID`, when the difficulties give one above 0.
    pub set_id: Option<u32>,
    pub diffs: Vec<DiffEntry>,
    /// Lowercase text searched besides the fields above: Unicode artist and title,
    /// source and tags.
    pub keywords: String,
}

impl MapsetEntry {
    /// Has a standard or catch difficulty, so a project can use it (D-04, D-67).
    pub fn editable(&self) -> bool {
        self.diffs.iter().any(DiffEntry::editable)
    }

    /// "Artist - Title", or the folder name when the header has neither.
    pub fn display_name(&self) -> String {
        match (self.artist.is_empty(), self.title.is_empty()) {
            (false, false) => format!("{} - {}", self.artist, self.title),
            (true, false) => self.title.clone(),
            (false, true) => self.artist.clone(),
            (true, true) => self.folder.rsplit('/').next().unwrap_or(&self.folder).to_owned(),
        }
    }

    /// Every term (lowercase) appears in a field: artist, title, creator, a
    /// difficulty name, the folder, the set ID or the keywords.
    pub fn matches(&self, terms: &[String]) -> bool {
        terms.iter().all(|term| {
            let has = |text: &str| text.to_lowercase().contains(term.as_str());
            has(&self.artist)
                || has(&self.title)
                || has(&self.creator)
                || has(&self.folder)
                || self.keywords.contains(term.as_str())
                || self.diffs.iter().any(|d| has(&d.version))
                || self.set_id.is_some_and(|id| id.to_string() == *term)
        })
    }
}

/// Splits a search box into lowercase terms.
pub fn search_terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SongIndex {
    version: u32,
    /// The Songs folder that was scanned.
    pub root: PathBuf,
    /// In folder order.
    pub mapsets: Vec<MapsetEntry>,
    /// Folders whose `.osu` files had no readable osu! header.
    #[serde(default)]
    pub unreadable: usize,
}

impl SongIndex {
    /// Reads the cache, or `None` when it is missing, unreadable or from an older
    /// version of the app.
    pub fn load(cache: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(cache).ok()?;
        let index: Self = serde_json::from_str(&text).ok()?;
        (index.version == CACHE_VERSION).then_some(index)
    }

    pub fn save(&self, cache: &Path) -> Result<(), String> {
        let write = || -> std::io::Result<()> {
            let dir = cache.parent().unwrap_or(Path::new("."));
            std::fs::create_dir_all(dir)?;
            let json = serde_json::to_vec(self).map_err(std::io::Error::other)?;
            let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
            tmp.write_all(&json)?;
            tmp.persist(cache).map_err(|e| e.error)?;
            Ok(())
        };
        write().map_err(|e| format!("The mapset list could not be saved: {e}"))
    }

    pub fn path_of(&self, entry: &MapsetEntry) -> PathBuf {
        entry.folder.split('/').fold(self.root.clone(), |path, part| path.join(part))
    }

    /// Where a project's mapset is now, when `old` no longer exists: the folder with
    /// the same name, else the one holding the most of the project's difficulty
    /// files (at least one). `None` when nothing fits.
    pub fn relocate(&self, old: &Path, diff_files: &[String]) -> Option<PathBuf> {
        let name = old.file_name()?.to_string_lossy().to_lowercase();
        let same_name = self.mapsets.iter().find(|m| {
            m.editable() && m.folder.rsplit('/').next().is_some_and(|f| f.to_lowercase() == name)
        });
        let best = same_name.or_else(|| {
            self.mapsets
                .iter()
                .filter(|m| m.editable())
                .map(|m| {
                    let found = diff_files
                        .iter()
                        .filter(|f| m.diffs.iter().any(|d| d.file.eq_ignore_ascii_case(f)))
                        .count();
                    (found, m)
                })
                .filter(|(found, _)| *found > 0)
                .max_by_key(|(found, _)| *found)
                .map(|(_, m)| m)
        })?;
        let path = self.path_of(best);
        path.is_dir().then_some(path)
    }
}

/// Progress of a running scan. `total` is the number of folders to look at.
#[derive(Debug, Default)]
pub struct ScanProgress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

/// Lists `root` and reads every mapset folder that changed since `previous`.
///
/// A changed folder takes its metadata from osu!'s own `osu!.db` next to the Songs
/// folder when that lists every `.osu` file in it, else from the files' headers.
/// Folders are handled on several threads: opening thousands of small files is slow
/// one at a time, mostly in waiting, not reading.
pub fn scan(
    root: &Path,
    previous: Option<&SongIndex>,
    progress: &ScanProgress,
) -> Result<SongIndex, String> {
    let entries = std::fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let mut folders: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    folders.sort_by_key(|(name, _)| name.to_lowercase());
    progress.total.store(folders.len(), Ordering::Relaxed);
    let cached: HashMap<&str, &MapsetEntry> = previous
        .filter(|p| p.root == root)
        .map(|p| p.mapsets.iter().map(|m| (m.folder.as_str(), m)).collect())
        .unwrap_or_default();
    let db_path = root.parent().map(|install| install.join("osu!.db"));
    let db: OnceLock<Option<OsuDb>> = OnceLock::new();
    let source = Source {
        cached: &cached,
        db: &|| {
            db.get_or_init(|| {
                let bytes = std::fs::read(db_path.as_ref()?).ok()?;
                osudb::read(&bytes).ok()
            })
            .as_ref()
        },
    };

    let results: Vec<Mutex<Vec<MapsetEntry>>> = folders.iter().map(|_| Mutex::default()).collect();
    let unreadable = AtomicUsize::new(0);
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(4, 16);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= folders.len() || progress.cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let (name, path) = &folders[i];
                    let mut found = Vec::new();
                    let mut bad = 0;
                    let osu = osu_files(path);
                    if osu.is_empty() {
                        // osu! also reads mapsets one folder further down, as packs unpack.
                        for (folder, path) in subfolders(name, path) {
                            let osu = osu_files(&path);
                            add(&mut found, &mut bad, folder, &path, &osu, &source);
                        }
                    } else {
                        add(&mut found, &mut bad, name.clone(), path, &osu, &source);
                    }
                    *results[i].lock().expect("one writer per folder") = found;
                    unreadable.fetch_add(bad, Ordering::Relaxed);
                    progress.done.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    if progress.cancel.load(Ordering::Relaxed) {
        return Err("The scan was cancelled.".into());
    }
    Ok(SongIndex {
        version: CACHE_VERSION,
        root: root.to_path_buf(),
        mapsets: results
            .into_iter()
            .flat_map(|m| m.into_inner().expect("workers finished"))
            .collect(),
        unreadable: unreadable.into_inner(),
    })
}

type OsuDb = HashMap<String, Vec<osudb::DbBeatmap>>;

/// Where a folder's metadata can come from besides its files.
struct Source<'a> {
    cached: &'a HashMap<&'a str, &'a MapsetEntry>,
    /// `osu!.db`, read the first time a folder needs it.
    db: &'a (dyn Fn() -> Option<&'a OsuDb> + Sync),
}

fn subfolders(name: &str, path: &Path) -> Vec<(String, PathBuf)> {
    let Ok(children) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut inner: Vec<(String, PathBuf)> = children
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (format!("{name}/{}", e.file_name().to_string_lossy()), e.path()))
        .collect();
    inner.sort_by_key(|(name, _)| name.to_lowercase());
    inner
}

fn add(
    found: &mut Vec<MapsetEntry>,
    unreadable: &mut usize,
    folder: String,
    path: &Path,
    osu: &[(String, u64, u64)],
    source: &Source<'_>,
) {
    if osu.is_empty() {
        return;
    }
    let fingerprint = fingerprint(osu);
    if let Some(entry) = source.cached.get(folder.as_str()).filter(|m| m.fingerprint == fingerprint) {
        found.push((*entry).clone());
        return;
    }
    let headers = from_db(&folder, osu, source).unwrap_or_else(|| {
        osu.iter()
            .filter_map(|(name, _, _)| Some((name.clone(), read_header(&path.join(name))?)))
            .collect()
    });
    match mapset_entry(folder, osu, fingerprint, &headers) {
        Some(entry) => found.push(entry),
        None => *unreadable += 1,
    }
}

/// The headers of every `.osu` file in `osu` from `osu!.db`, or `None` when it
/// misses one of them (a difficulty osu! hasn't imported yet).
fn from_db(folder: &str, osu: &[(String, u64, u64)], source: &Source<'_>) -> Option<Vec<(String, Header)>> {
    let maps = (source.db)()?.get(&folder.replace('/', "\\").to_lowercase())?;
    osu.iter()
        .map(|(name, _, _)| {
            let m = maps.iter().find(|m| m.file.eq_ignore_ascii_case(name))?;
            Some((
                name.clone(),
                Header {
                    mode: m.mode,
                    title: m.title.clone(),
                    title_unicode: m.title_unicode.clone(),
                    artist: m.artist.clone(),
                    artist_unicode: m.artist_unicode.clone(),
                    creator: m.creator.clone(),
                    version: m.version.clone(),
                    source: m.source.clone(),
                    tags: m.tags.clone(),
                    set_id: m.set_id,
                },
            ))
        })
        .collect()
}

/// `.osu` files in `dir`: name, size and modified time (Unix ms).
fn osu_files(dir: &Path) -> Vec<(String, u64, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|e| crate::mapset::is_osu(&e.path()))
        .filter_map(|e| {
            let meta = e.metadata().ok().filter(std::fs::Metadata::is_file)?;
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
            Some((e.file_name().to_string_lossy().into_owned(), meta.len(), modified))
        })
        .collect();
    files.sort();
    files
}

/// FNV-1a over the file list, so the cache stays valid across builds of the app.
fn fingerprint(osu: &[(String, u64, u64)]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for (name, size, modified) in osu {
        feed(name.as_bytes());
        feed(&[0]);
        feed(&size.to_le_bytes());
        feed(&modified.to_le_bytes());
    }
    hash
}

fn mapset_entry(
    folder: String,
    osu: &[(String, u64, u64)],
    fingerprint: u64,
    headers: &[(String, Header)],
) -> Option<MapsetEntry> {
    // The standard or catch difficulties name the set, as the editor will use them.
    let first = headers
        .iter()
        .find(|(_, h)| matches!(h.mode, 0 | 2))
        .or(headers.first())?;
    let pick = |field: fn(&Header) -> &str| {
        let value = field(&first.1);
        if value.is_empty() {
            headers.iter().map(|(_, h)| field(h)).find(|v| !v.is_empty()).unwrap_or("")
        } else {
            value
        }
        .to_owned()
    };
    let mut keywords: Vec<String> = Vec::new();
    for (_, h) in headers {
        for value in [&h.artist_unicode, &h.title_unicode, &h.source, &h.tags] {
            let value = value.to_lowercase();
            if !value.is_empty() && !keywords.contains(&value) {
                keywords.push(value);
            }
        }
    }
    let mut diffs: Vec<DiffEntry> = headers
        .iter()
        .map(|(file, h)| DiffEntry {
            file: file.clone(),
            version: h.version.clone(),
            mode: h.mode,
        })
        .collect();
    diffs.sort_by(|a, b| a.mode.cmp(&b.mode).then_with(|| a.version.to_lowercase().cmp(&b.version.to_lowercase())));
    Some(MapsetEntry {
        set_id: headers.iter().find_map(|(_, h)| h.set_id),
        artist: pick(|h| &h.artist),
        title: pick(|h| &h.title),
        creator: pick(|h| &h.creator),
        modified: osu.iter().map(|f| f.2).max().unwrap_or(0),
        folder,
        fingerprint,
        diffs,
        keywords: keywords.join(" "),
    })
}

#[derive(Debug, Default, PartialEq)]
struct Header {
    mode: u8,
    title: String,
    title_unicode: String,
    artist: String,
    artist_unicode: String,
    creator: String,
    version: String,
    source: String,
    tags: String,
    set_id: Option<u32>,
}

/// Reads `[General] Mode` and `[Metadata]` from the top of a `.osu` file, stopping at
/// the first section after them. `None` for a file without the osu! header.
fn read_header(path: &Path) -> Option<Header> {
    let file = std::fs::File::open(path).ok()?;
    parse_header(BufReader::new(file.take(HEADER_LIMIT)))
}

fn parse_header(reader: impl BufRead) -> Option<Header> {
    let mut lines = reader.lines();
    let first = lines.next()?.ok()?;
    if !first.trim_start_matches('\u{feff}').trim_start().starts_with("osu file format") {
        return None;
    }
    let mut header = Header::default();
    let mut section = String::new();
    for line in lines {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_owned();
            // Everything needed comes before these.
            if matches!(section.as_str(), "Difficulty" | "Events" | "TimingPoints" | "HitObjects") {
                break;
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_owned();
        match (section.as_str(), key.trim()) {
            ("General", "Mode") => header.mode = value.parse().unwrap_or(0),
            ("Metadata", "Title") => header.title = value,
            ("Metadata", "TitleUnicode") => header.title_unicode = value,
            ("Metadata", "Artist") => header.artist = value,
            ("Metadata", "ArtistUnicode") => header.artist_unicode = value,
            ("Metadata", "Creator") => header.creator = value,
            ("Metadata", "Version") => header.version = value,
            ("Metadata", "Source") => header.source = value,
            ("Metadata", "Tags") => header.tags = value,
            ("Metadata", "BeatmapSetID") => {
                header.set_id = value.parse().ok().filter(|&id: &u32| id > 0);
            }
            _ => {}
        }
    }
    Some(header)
}

/// The Songs folder of a standard osu!stable install, if there is one:
/// `%LOCALAPPDATA%\osu!`, with the `BeatmapDirectory` of the user's
/// `osu!.<name>.cfg` when it moved the folder.
pub fn detect_songs_dir() -> Option<PathBuf> {
    let install = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("osu!");
    songs_dir_of(&install)
}

/// The Songs folder of the osu! install at `install`.
pub fn songs_dir_of(install: &Path) -> Option<PathBuf> {
    let configured = std::fs::read_dir(install).ok().and_then(|entries| {
        entries.filter_map(Result::ok).find_map(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            // osu!.cfg holds the machine's settings; the user's file has a name in it.
            if !(name.starts_with("osu!.") && name.ends_with(".cfg") && name != "osu!.cfg") {
                return None;
            }
            let text = std::fs::read_to_string(e.path()).ok()?;
            text.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == "BeatmapDirectory")
                    .then(|| value.trim())
                    .filter(|v| !v.is_empty())
                    .map(|v| install.join(v))
            })
        })
    });
    configured
        .into_iter()
        .chain([install.join("Songs")])
        .find(|dir| dir.is_dir())
}

/// A scan on its own thread, so neither the UI nor the session worker waits for it.
pub struct Scan {
    progress: Arc<ScanProgress>,
    result: Arc<Mutex<Option<Result<SongIndex, String>>>>,
    root: PathBuf,
}

impl Scan {
    /// Scans `root`, reusing `previous`, and saves the result to `cache`. `notify`
    /// runs when it finishes (to wake the UI).
    pub fn start(
        root: PathBuf,
        cache: Option<PathBuf>,
        previous: Option<Arc<SongIndex>>,
        notify: impl Fn() + Send + 'static,
    ) -> Self {
        let progress = Arc::new(ScanProgress::default());
        let result = Arc::new(Mutex::new(None));
        let (thread_progress, thread_result, thread_root) =
            (Arc::clone(&progress), Arc::clone(&result), root.clone());
        let spawned = std::thread::Builder::new()
            .name("hitsounds-rs-songs".into())
            .spawn(move || {
                let mut scanned = scan(&thread_root, previous.as_deref(), &thread_progress);
                if let (Ok(index), Some(cache)) = (&scanned, &cache)
                    && previous.as_deref() != Some(index)
                    && let Err(e) = index.save(cache)
                {
                    scanned = Err(e);
                }
                *thread_result.lock().expect("scan result lock") = Some(scanned);
                notify();
            });
        if let Err(e) = spawned {
            *result.lock().expect("scan result lock") =
                Some(Err(format!("The mapset scan could not start: {e}")));
        }
        Self { progress, result, root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Folders looked at and the total, once the folder is listed.
    pub fn progress(&self) -> (usize, usize) {
        (
            self.progress.done.load(Ordering::Relaxed),
            self.progress.total.load(Ordering::Relaxed),
        )
    }

    /// The result, once; `None` while it runs.
    pub fn take(&self) -> Option<Result<SongIndex, String>> {
        self.result.lock().ok()?.take()
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.progress.cancel.store(true, Ordering::Relaxed);
    }
}

/// Unix milliseconds now, for "changed … ago".
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osu(mode: u8, artist: &str, title: &str, version: &str, set_id: i64) -> String {
        format!(
            "osu file format v14\n\n[General]\nAudioFilename: audio.mp3\nMode: {mode}\n\n[Metadata]\nTitle:{title}\nTitleUnicode:{title} (u)\nArtist:{artist}\nCreator:Mapper\nVersion:{version}\nTags:drums synthetic\nBeatmapSetID:{set_id}\n\n[Difficulty]\nHPDrainRate:5\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n"
        )
    }

    fn songs() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let set = dir.path().join("100 Artist - Song");
        std::fs::create_dir(&set).unwrap();
        std::fs::write(set.join("a.osu"), osu(0, "Artist", "Song", "Hard", 100)).unwrap();
        std::fs::write(set.join("b.osu"), osu(3, "Artist", "Song", "Hitsounds", 100)).unwrap();
        let mania = dir.path().join("200 Other - Keys");
        std::fs::create_dir(&mania).unwrap();
        std::fs::write(mania.join("k.osu"), osu(3, "Other", "Keys", "4K", -1)).unwrap();
        let pack = dir.path().join("Pack").join("300 Packed - Map");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("p.osu"), osu(2, "Packed", "Map", "Salad", 300)).unwrap();
        std::fs::create_dir(dir.path().join("empty")).unwrap();
        dir
    }

    #[test]
    fn scan_reads_headers_modes_and_packs() {
        let dir = songs();
        let index = scan(dir.path(), None, &ScanProgress::default()).unwrap();
        let names: Vec<_> = index.mapsets.iter().map(|m| (m.folder.as_str(), m.display_name(), m.editable())).collect();
        assert_eq!(
            names,
            [
                ("100 Artist - Song", "Artist - Song".to_owned(), true),
                ("200 Other - Keys", "Other - Keys".to_owned(), false),
                ("Pack/300 Packed - Map", "Packed - Map".to_owned(), true),
            ]
        );
        let first = &index.mapsets[0];
        assert_eq!(first.set_id, Some(100));
        assert_eq!(index.mapsets[1].set_id, None);
        assert_eq!(first.creator, "Mapper");
        assert_eq!(
            first.diffs.iter().map(|d| (d.version.as_str(), d.mode)).collect::<Vec<_>>(),
            [("Hard", 0), ("Hitsounds", 3)]
        );
        assert_eq!(index.path_of(&index.mapsets[2]), dir.path().join("Pack").join("300 Packed - Map"));
    }

    #[test]
    fn search_matches_every_term_across_fields() {
        let dir = songs();
        let index = scan(dir.path(), None, &ScanProgress::default()).unwrap();
        let hits = |q: &str| {
            let terms = search_terms(q);
            index.mapsets.iter().filter(|m| m.matches(&terms)).map(|m| m.folder.clone()).collect::<Vec<_>>()
        };
        assert_eq!(hits("artist hard"), ["100 Artist - Song"]);
        assert_eq!(hits("SONG (U)"), ["100 Artist - Song"]);
        assert_eq!(hits("300"), ["Pack/300 Packed - Map"]);
        assert_eq!(hits("drums").len(), 3);
        assert!(hits("artist salad").is_empty());
    }

    #[test]
    fn rescan_reuses_unchanged_folders_and_rereads_changed_ones() {
        let dir = songs();
        let first = scan(dir.path(), None, &ScanProgress::default()).unwrap();
        // A cached entry with a matching fingerprint is reused as it is.
        let mut cached = first.clone();
        cached.mapsets[0].title = "From cache".into();
        let again = scan(dir.path(), Some(&cached), &ScanProgress::default()).unwrap();
        assert_eq!(again.mapsets[0].title, "From cache");
        // A new difficulty changes the fingerprint, so the folder is read again.
        std::fs::write(dir.path().join("100 Artist - Song").join("c.osu"), osu(0, "Artist", "Song", "Easy", 100)).unwrap();
        let changed = scan(dir.path(), Some(&cached), &ScanProgress::default()).unwrap();
        assert_eq!(changed.mapsets[0].title, "Song");
        assert_eq!(changed.mapsets[0].diffs.len(), 3);
    }

    #[test]
    fn osu_db_supplies_imported_folders_and_headers_the_rest() {
        let install = tempfile::tempdir().unwrap();
        let root = install.path().join("Songs");
        std::fs::create_dir(&root).unwrap();
        for (folder, file) in [("1 A - B", "a.osu"), ("2 New", "n.osu")] {
            std::fs::create_dir(root.join(folder)).unwrap();
            std::fs::write(root.join(folder).join(file), osu(0, "Header", "Header", "Hard", 9)).unwrap();
        }
        // osu! knows the first folder (under another title, to tell the sources
        // apart), not the second.
        let db = crate::osudb::tests::synthetic(20_250_107, &[("1 A - B", "a.osu", 0, "From db")]);
        std::fs::write(install.path().join("osu!.db"), db).unwrap();
        let index = scan(&root, None, &ScanProgress::default()).unwrap();
        let titles: Vec<_> = index.mapsets.iter().map(|m| (m.folder.as_str(), m.title.as_str())).collect();
        assert_eq!(titles, [("1 A - B", "From db"), ("2 New", "Header")]);
        // A difficulty added since osu! last looked sends the folder to the headers.
        std::fs::write(root.join("1 A - B").join("b.osu"), osu(0, "Header", "Header", "Easy", 9)).unwrap();
        let index = scan(&root, None, &ScanProgress::default()).unwrap();
        assert_eq!(index.mapsets[0].title, "Header");
        // A broken osu!.db is only a slower scan.
        std::fs::write(install.path().join("osu!.db"), b"garbage").unwrap();
        assert_eq!(scan(&root, None, &ScanProgress::default()).unwrap().mapsets.len(), 2);
    }

    #[test]
    fn cache_round_trips_and_ignores_other_versions() {
        let dir = songs();
        let index = scan(dir.path(), None, &ScanProgress::default()).unwrap();
        let cache = dir.path().join("cache").join("songs.json");
        index.save(&cache).unwrap();
        assert_eq!(SongIndex::load(&cache), Some(index.clone()));
        let old = serde_json::to_string(&SongIndex { version: 0, ..index }).unwrap();
        std::fs::write(&cache, old).unwrap();
        assert_eq!(SongIndex::load(&cache), None);
    }

    #[test]
    fn relocate_prefers_the_same_folder_name_then_shared_difficulties() {
        let dir = songs();
        let index = scan(dir.path(), None, &ScanProgress::default()).unwrap();
        let gone = Path::new("D:/old/Songs/100 Artist - Song");
        assert_eq!(index.relocate(gone, &[]), Some(dir.path().join("100 Artist - Song")));
        let renamed = Path::new("D:/old/Songs/Renamed");
        assert_eq!(index.relocate(renamed, &["P.osu".into()]), Some(dir.path().join("Pack").join("300 Packed - Map")));
        // A mania-only set can't hold a project, and nothing shared means no match.
        assert_eq!(index.relocate(Path::new("x/200 Other - Keys"), &["k.osu".into()]), None);
        assert_eq!(index.relocate(renamed, &["missing.osu".into()]), None);
    }

    #[test]
    fn header_parsing_stops_at_difficulty_and_rejects_non_osu_files() {
        let text = "\u{feff}osu file format v7\n[General]\nMode: 2\n[Metadata]\nTitle: T\nBeatmapSetID: 0\n[Difficulty]\n[Metadata]\nTitle: Late\n";
        let header = parse_header(text.as_bytes()).unwrap();
        assert_eq!((header.mode, header.title.as_str(), header.set_id), (2, "T", None));
        assert_eq!(parse_header("[General]\nMode: 0\n".as_bytes()), None);
    }

    #[test]
    fn songs_folder_comes_from_the_user_config_when_moved() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(songs_dir_of(dir.path()), None);
        std::fs::create_dir(dir.path().join("Songs")).unwrap();
        assert_eq!(songs_dir_of(dir.path()), Some(dir.path().join("Songs")));
        let moved = dir.path().join("Elsewhere");
        std::fs::create_dir(&moved).unwrap();
        std::fs::write(dir.path().join("osu!.cfg"), "BeatmapDirectory = Wrong\n").unwrap();
        std::fs::write(dir.path().join("osu!.player.cfg"), format!("Volume = 80\nBeatmapDirectory = {}\n", moved.display())).unwrap();
        assert_eq!(songs_dir_of(dir.path()), Some(moved));
    }

    /// Timings on a real Songs folder: `HS_SONGS_DIR=<folder> cargo test --release -p
    /// hs-session -- --ignored real_songs_folder --nocapture`. Only reads the folder;
    /// the cache goes to a temporary folder.
    #[test]
    #[ignore = "needs a real osu! Songs folder in HS_SONGS_DIR"]
    fn real_songs_folder() {
        use std::time::Instant;
        let root = PathBuf::from(std::env::var_os("HS_SONGS_DIR").expect("set HS_SONGS_DIR"));
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("songs.json");

        let t = Instant::now();
        let first = scan(&root, None, &ScanProgress::default()).unwrap();
        let full = t.elapsed();
        let t = Instant::now();
        first.save(&cache).unwrap();
        let save = t.elapsed();
        let t = Instant::now();
        let loaded = SongIndex::load(&cache).unwrap();
        let load = t.elapsed();
        let t = Instant::now();
        let again = scan(&root, Some(&loaded), &ScanProgress::default()).unwrap();
        let refresh = t.elapsed();
        assert_eq!(again, first, "nothing changed, so the refresh matches");

        // osu!.db must say what the headers say: compare every 40th mapset.
        let mut checked = 0;
        let mut differ = Vec::new();
        for m in first.mapsets.iter().step_by(40) {
            let dir = first.path_of(m);
            let osu = osu_files(&dir);
            let headers: Vec<_> = osu
                .iter()
                .filter_map(|(name, _, _)| Some((name.clone(), read_header(&dir.join(name))?)))
                .collect();
            let from_files = mapset_entry(m.folder.clone(), &osu, m.fingerprint, &headers);
            checked += 1;
            // The set ID may differ: osu!.db keeps the online one, and old files have none.
            let same = from_files.as_ref().is_some_and(|f| MapsetEntry { set_id: m.set_id, ..f.clone() } == *m);
            if !same {
                differ.push((m.clone(), from_files));
            }
        }
        for (db, files) in differ.iter().take(3) {
            println!("differs:\n  index {db:?}\n  files {files:?}");
        }
        println!("{} of {checked} sampled mapsets differ from their .osu headers beyond the set ID", differ.len());

        let diffs: usize = first.mapsets.iter().map(|m| m.diffs.len()).sum();
        let editable = first.mapsets.iter().filter(|m| m.editable()).count();
        let t = Instant::now();
        let mut hits = 0;
        for query in ["a", "camellia", "hard", "xi freedom dive", "zzzz"] {
            let terms = search_terms(query);
            hits += first.mapsets.iter().filter(|m| m.matches(&terms)).count();
        }
        let search = t.elapsed() / 5;
        println!(
            "{} mapsets ({editable} standard/catch), {diffs} difficulties, {} unreadable\n\
             full scan {full:.2?}, refresh with cache {refresh:.2?}\n\
             cache {} KB, save {save:.2?}, load {load:.2?}\n\
             one search over everything {search:.2?} ({hits} hits in 5 queries)",
            first.mapsets.len(),
            first.unreadable,
            std::fs::metadata(&cache).unwrap().len() / 1024,
        );
    }

    #[test]
    fn background_scan_saves_the_cache() {
        let dir = songs();
        let cache = dir.path().join("songs.json");
        let (tx, rx) = std::sync::mpsc::channel();
        let job = Scan::start(dir.path().to_path_buf(), Some(cache.clone()), None, move || {
            let _ = tx.send(());
        });
        rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        let index = job.take().unwrap().unwrap();
        assert_eq!(index.mapsets.len(), 3);
        assert_eq!(job.progress().0, job.progress().1);
        assert_eq!(SongIndex::load(&cache), Some(index));
    }
}
