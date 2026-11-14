//! Disk, decoding, and audio schedule rebuilds run here (D-48, D-55).

use crate::{
    chrome::Owner,
    editor::model::{Layer, Wave},
    export_view::{ExportReview, LineDiff},
    theme,
};
use eframe::egui::Context;
use hs_core::{
    ChannelId, ChannelProps, Edit, Project, SampleSource,
    project::{ExportSettings, Monitor},
};
use hs_session::{
    Clock, Levels, OnMapsetChange, Session, Settings, Transport, import,
    library::{self, LibraryFolder},
    separation::{self, Request, SeparatedStem},
    settings::AppSettings,
    songs::SongIndex,
};
use osu_format::{Hitsound, SampleSet};
use rfd::FileDialog;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

pub enum Job {
    /// A new project; asks for the mapset folder.
    Create,
    /// A new project for this mapset folder, chosen in the mapset picker (D-82).
    CreateFrom(PathBuf),
    /// A new project from a mania hitsound layer difficulty (D-66).
    ImportHitsounds,
    /// Point the open project at another mapset folder (D-67); asks for the folder.
    ChangeMapset,
    /// Point the open project at this mapset folder, chosen in the picker (D-67, D-82).
    SetMapset(PathBuf),
    Open,
    OpenRecent(PathBuf),
    /// Open a project whose mapset folder moved, with the mapset at `mapset`, or at a
    /// folder the user is asked for (D-82).
    OpenRelocated {
        file: PathBuf,
        mapset: Option<PathBuf>,
    },
    Close,
    Save,
    Refresh {
        automatic: bool,
    },
    ChooseSkin,
    ClearSkin,
    /// The osu! Songs folder the mapset picker lists (D-82).
    ChooseSongs,
    /// Back to the Songs folder of the standard osu! install.
    ClearSongs,
    /// Where new projects go without asking (D-82).
    ChooseProjectsDir,
    ClearProjectsDir,
    AddLibrary,
    RemoveLibrary(PathBuf),
    ForgetRecent(PathBuf),
    ReduceMotion(Option<bool>),
    /// Turn the startup update check on or off (D-84).
    UpdateChecks(bool),
    /// Keep the song's pitch when slowed (D-90).
    KeepPitch(bool),
    /// Save the changed keyboard shortcuts (D-92).
    Keybinds(std::collections::BTreeMap<String, Vec<String>>),
    UiScale(Option<f32>),
    /// Save and apply the listening levels (D-77).
    Levels(Levels),
    ImportStem,
    RemoveStem,
    /// Choose the user's drums model file for separation (D-71).
    ChooseModel,
    ClearModel,
    /// Prepare a drum separation of the song (D-70); it runs on its own thread.
    Separate,
    /// Make a finished separation the project's stem, as one undo step (D-72).
    ApplySeparation(Box<SeparatedStem>),
    Fallback(Option<u32>),
    History {
        redo: bool,
    },
    Monitor(Monitor),
    SelectLayer(Option<String>),
    Edit(Edit),
    AddChannel,
    UseSample {
        channel: Option<ChannelId>,
        path: Option<PathBuf>,
    },
    AuditionFile(PathBuf),
    AuditionChannel(ChannelId),
    ScanLibrary,
    BuildExport(ExportSettings),
    ReadDiff(String),
    WriteExport,
}

impl Job {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Create | Self::CreateFrom(_) => "Creating project",
            Self::ImportHitsounds => "Importing hitsound diff",
            Self::ChangeMapset | Self::SetMapset(_) => "Changing mapset",
            Self::Open | Self::OpenRecent(_) | Self::OpenRelocated { .. } => "Opening project",
            Self::Close => "Closing project",
            Self::Save => "Saving project",
            Self::Refresh { .. } => "Refreshing mapset",
            Self::ChooseSkin | Self::ClearSkin => "Updating skin folder",
            Self::ChooseSongs | Self::ClearSongs => "Updating Songs folder",
            Self::ChooseProjectsDir | Self::ClearProjectsDir => "Updating projects folder",
            Self::AddLibrary | Self::RemoveLibrary(_) => "Updating library folders",
            Self::ForgetRecent(_)
            | Self::ReduceMotion(_)
            | Self::UpdateChecks(_)
            | Self::KeepPitch(_)
            | Self::Keybinds(_)
            | Self::UiScale(_)
            | Self::Levels(_) => "Saving settings",
            Self::ImportStem | Self::RemoveStem | Self::ApplySeparation(_) => "Updating drum stem",
            Self::ChooseModel => "Checking the drums model",
            Self::ClearModel => "Saving settings",
            Self::Separate => "Preparing drum separation",
            Self::Fallback(_) => "Updating default fallback",
            Self::History { redo: false } => "Undoing change",
            Self::History { redo: true } => "Redoing change",
            Self::Monitor(_) => "Switching monitor source",
            Self::SelectLayer(_) => "Switching layer",
            Self::Edit(_) => "Updating timeline",
            Self::AddChannel => "Adding channel",
            Self::UseSample { .. } => "Loading sample",
            Self::AuditionFile(_) | Self::AuditionChannel(_) => "Auditioning sample",
            Self::ScanLibrary => "Scanning sample folders",
            Self::BuildExport(_) => "Building export preview",
            Self::ReadDiff(_) => "Comparing .osu lines",
            Self::WriteExport => "Writing export",
        }
    }

    fn changes_project_view(&self) -> bool {
        !matches!(
            self,
            Self::ReduceMotion(_)
                | Self::UpdateChecks(_)
                | Self::KeepPitch(_)
                | Self::Keybinds(_)
                | Self::UiScale(_)
                | Self::Levels(_)
                | Self::ForgetRecent(_)
                | Self::AddLibrary
                | Self::RemoveLibrary(_)
                | Self::ChooseSongs
                | Self::ClearSongs
                | Self::ChooseProjectsDir
                | Self::ClearProjectsDir
                | Self::AuditionFile(_)
                | Self::AuditionChannel(_)
                | Self::ScanLibrary
                | Self::ReadDiff(_)
                | Self::ChooseModel
                | Self::ClearModel
                | Self::Separate
        )
    }
}

pub struct Snapshot {
    pub settings: AppSettings,
    pub skin_ready: bool,
    /// The drums model setting points at a file (D-71).
    pub model_ready: bool,
    /// `onnxruntime.dll` is next to the app (D-72).
    pub runtime_ready: bool,
    pub recent_exists: Vec<bool>,
    /// The Songs folder the mapset picker lists: the setting, else the one found in
    /// the standard osu! install (D-82). `None` when neither is a folder.
    pub songs_dir: Option<PathBuf>,
    /// `songs_dir` was found, not chosen.
    pub songs_detected: bool,
    /// The projects folder setting is a folder.
    pub projects_ready: bool,
    pub warning: Option<String>,
    pub active: Option<Arc<ProjectView>>,
    pub library: Option<Arc<Vec<LibraryFolder>>>,
}

pub struct ProjectView {
    pub file: PathBuf,
    pub project: Project,
    pub available_diffs: Vec<String>,
    pub length_ms: f64,
    pub stem_ready: bool,
    /// The separated stem was made from an older version of the song (D-73).
    pub stem_outdated: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub dirty: bool,
    pub warnings: Vec<String>,
    pub waveform: Arc<Wave>,
    pub layer: Layer,
    pub mapset_changed: Arc<AtomicBool>,
}

impl ProjectView {
    fn new(session: &Session, mapset_changed: Arc<AtomicBool>, waveform: Arc<Wave>) -> Self {
        let view = session.workspace();
        let layer = Layer::from_session(session, &view);
        Self {
            file: session.file().to_path_buf(),
            project: session.project.clone(),
            available_diffs: view
                .available_diffs
                .into_iter()
                .map(str::to_owned)
                .collect(),
            length_ms: view.length_ms,
            stem_ready: view.stem_ready,
            stem_outdated: view.stem_outdated,
            can_undo: view.can_undo,
            can_redo: view.can_redo,
            dirty: view.dirty,
            warnings: view.warnings,
            waveform,
            layer,
            mapset_changed,
        }
    }
}

pub struct Reply {
    pub snapshot: Snapshot,
    pub result: Result<Option<Outcome>, String>,
}

pub enum Outcome {
    Notice(String),
    SelectChannel(ChannelId),
    ExportPreview(Box<ExportReview>),
    ExportDiff(LineDiff),
    Exported(hs_session::CompletedExport),
    /// Start this separation on its own thread.
    Separate(Box<Request>),
    /// A project was just created; the shell may offer to separate its drums.
    Created(String),
    /// This project's mapset folder is gone and the cached Songs list has no match;
    /// the shell asks where it is now (D-82).
    MapsetMoved {
        file: PathBuf,
        moved: hs_session::MovedMapset,
    },
}

pub struct Worker {
    session: Arc<Mutex<Option<Session>>>,
    pub jobs: Sender<Job>,
    pub replies: Receiver<Reply>,
}

impl Worker {
    pub fn start(ctx: Context, owner: Owner) -> std::io::Result<Self> {
        let (send_job, jobs) = mpsc::channel::<Job>();
        let (replies, receive_reply) = mpsc::channel();
        let session = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&session);
        thread::Builder::new()
            .name("hitsounds-rs-worker".into())
            .spawn(move || {
                let mut state = State::new(ctx, owner);
                if !state.reply(&replies, &None, Ok(None)) {
                    return;
                }
                for job in jobs {
                    let mut session = slot.lock().expect("worker owns session mutations");
                    if job.changes_project_view() {
                        state.project_view = None;
                    }
                    let result = state.run(job, &mut session);
                    if !state.reply(&replies, &session, result) {
                        break;
                    }
                }
            })?;
        Ok(Self {
            session,
            jobs: send_job,
            replies: receive_reply,
        })
    }

    pub fn clock(&self) -> Option<Clock> {
        Some(self.session.try_lock().ok()?.as_ref()?.playback.clock())
    }

    pub fn transport(&self, action: Transport) -> Result<(), String> {
        if let Ok(session) = self.session.try_lock()
            && let Some(session) = session.as_ref()
        {
            return session.transport(action);
        }
        Err("Playback is unavailable while the project is busy.".into())
    }

    /// Applies levels at once while a fader is dragged; `Job::Levels` saves them
    /// (D-77). A busy worker just skips this frame's update.
    pub fn preview_levels(&self, levels: &Levels) {
        if let Ok(session) = self.session.try_lock()
            && let Some(session) = session.as_ref()
        {
            let _ = session.set_levels(levels);
        }
    }
}

struct State {
    ctx: Context,
    owner: Owner,
    settings: Settings,
    mapset_changed: Arc<AtomicBool>,
    waveform: Option<(u32, Monitor, Arc<Wave>)>,
    project_view: Option<Arc<ProjectView>>,
    library: Option<Arc<Vec<LibraryFolder>>>,
    /// The Songs folder of the standard osu! install, looked up once (D-82).
    detected_songs: Option<PathBuf>,
}

/// The Songs folder of the osu!stable install (D-82): the standard place, else
/// wherever the installer registered `osu!.exe` for `.osz` files.
fn detect_songs_dir() -> Option<PathBuf> {
    hs_session::songs::detect_songs_dir().or_else(|| {
        let install = registered_install()?;
        hs_session::songs::songs_dir_of(&install)
    })
}

/// The folder of the `osu!.exe` that opens `.osz` files, from
/// `HKEY_CLASSES_ROOT\osustable.File.osz\shell\open\command`.
fn registered_install() -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::{HKEY_CLASSES_ROOT, RRF_RT_REG_SZ, RegGetValueW};
    let key: Vec<u16> = r"osustable.File.osz\shell\open\command"
        .encode_utf16()
        .chain([0])
        .collect();
    let mut buffer = vec![0u16; 1024];
    let mut bytes = (buffer.len() * 2) as u32;
    // SAFETY: the key is NUL-terminated, the buffer holds `bytes` bytes, and a null
    // value name reads the key's default value.
    let status = unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            key.as_ptr(),
            std::ptr::null(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != 0 {
        return None;
    }
    let len = (bytes as usize / 2).saturating_sub(1).min(buffer.len());
    install_from_command(&String::from_utf16_lossy(&buffer[..len]))
}

/// `"D:\Games\osu!\osu!.exe" "%1"` → `D:\Games\osu!`.
fn install_from_command(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    let exe = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => command.split(" \"").next()?.trim(),
    };
    let exe = Path::new(exe);
    exe.file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("osu!.exe")
        .then(|| exe.parent().map(Path::to_path_buf))
        .flatten()
}

/// The cached list of the Songs folder (D-82).
pub fn songs_cache() -> Option<PathBuf> {
    config_folder().map(|dir| dir.join("songs.json"))
}

/// The ONNX Runtime the app ships next to its exe (D-72). `HS_ORT_DYLIB` overrides it
/// for development.
pub fn runtime_path() -> PathBuf {
    std::env::var_os("HS_ORT_DYLIB")
        .map(PathBuf::from)
        .or_else(|| {
            let exe = std::env::current_exe().ok()?;
            Some(exe.parent()?.join("onnxruntime.dll"))
        })
        .unwrap_or_else(|| PathBuf::from("onnxruntime.dll"))
}

pub fn config_folder() -> Option<PathBuf> {
    std::env::var_os("HITSOUNDS_RS_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir().map(|d| d.join("dev.hitsounds-rs.app")))
}

impl State {
    fn new(ctx: Context, owner: Owner) -> Self {
        // App-wide settings: skin folder, recent projects, library (D-40, D-41, D-45).
        let settings = Settings::load(config_folder().map(|d| d.join("settings.json")));
        Self {
            ctx,
            owner,
            settings,
            mapset_changed: Arc::default(),
            waveform: None,
            project_view: None,
            library: None,
            detected_songs: detect_songs_dir(),
        }
    }

    /// The Songs folder in use and whether it was found rather than chosen.
    fn songs_dir(&self) -> (Option<PathBuf>, bool) {
        match self.settings.value.songs_dir.as_deref() {
            Some(dir) => (dir.is_dir().then(|| dir.to_path_buf()), false),
            None => (self.detected_songs.clone(), true),
        }
    }

    fn reply(
        &mut self,
        replies: &Sender<Reply>,
        session: &Option<Session>,
        result: Result<Option<Outcome>, String>,
    ) -> bool {
        if let Some(session) = session
            && self.project_view.is_none()
        {
            let version = session.playback.waveform_version;
            let monitor = session.project.audio.monitor;
            if !self
                .waveform
                .as_ref()
                .is_some_and(|(v, m, _)| *v == version && *m == monitor)
            {
                self.waveform = Some((version, monitor, Arc::new(Wave::new(session.waveform()))));
            }
            let wave = self
                .waveform
                .as_ref()
                .expect("waveform cache is populated")
                .2
                .clone();
            self.project_view = Some(Arc::new(ProjectView::new(
                session,
                self.mapset_changed.clone(),
                wave,
            )));
        }
        let settings = self.settings.value.clone();
        let (songs_dir, songs_detected) = self.songs_dir();
        let snapshot = Snapshot {
            songs_dir,
            songs_detected,
            projects_ready: settings.projects_dir.as_deref().is_some_and(Path::is_dir),
            skin_ready: settings.skin_dir.as_deref().is_some_and(Path::is_dir),
            model_ready: settings
                .separation_model
                .as_deref()
                .is_some_and(Path::is_file),
            runtime_ready: runtime_path().is_file(),
            recent_exists: settings.recent.iter().map(|p| p.file.is_file()).collect(),
            settings,
            warning: self.settings.warning.clone(),
            active: self.project_view.clone(),
            library: self.library.clone(),
        };
        let sent = replies.send(Reply { snapshot, result }).is_ok();
        self.ctx.request_repaint();
        sent
    }

    fn dialog(&self, title: &str) -> FileDialog {
        FileDialog::new().set_title(title).set_parent(&self.owner)
    }

    fn watcher(&self, changed: Arc<AtomicBool>) -> OnMapsetChange {
        let ctx = self.ctx.clone();
        Arc::new(move |_| {
            changed.store(true, Ordering::Relaxed);
            ctx.request_repaint();
        })
    }

    fn set_project(
        &mut self,
        slot: &mut Option<Session>,
        session: Session,
        changed: Arc<AtomicBool>,
    ) {
        self.settings
            .remember_project(session.file(), &session.project, session.skin_dir());
        self.mapset_changed = changed;
        self.waveform = None;
        // Each session opens its own audio engine, so it starts at the app's levels.
        let _ = session.set_levels(&self.settings.value.levels);
        let _ = session.set_keep_pitch(self.settings.value.keeps_pitch());
        *slot = Some(session);
    }

    /// Creates a project for `mapset`: in the projects folder when one is set (D-82),
    /// else where the user says. `None` = cancelled.
    fn new_session(&self, mapset: &Path) -> Result<Option<(Session, Arc<AtomicBool>)>, String> {
        let name = mapset
            .file_name()
            .map_or_else(|| "New project".into(), |p| p.to_string_lossy());
        let projects = self
            .settings
            .value
            .projects_dir
            .as_deref()
            .filter(|dir| dir.is_dir() && !dir.starts_with(mapset));
        let file = if let Some(dir) = projects {
            free_project_path(dir, &name)
        } else {
            let mut dialog = self
                .dialog("Create the project outside the mapset folder")
                .add_filter("hitsounds-rs project", &["hsproj"])
                .set_file_name(format!("{name}.hsproj"));
            // Start where the last project went, so new ones collect in one place.
            let last = self
                .settings
                .value
                .recent
                .iter()
                .find_map(|r| r.file.parent()?.parent().filter(|d| d.is_dir()));
            if let Some(dir) = last.map(Path::to_path_buf).or_else(dirs::document_dir) {
                dialog = dialog.set_directory(dir);
            }
            let Some(file) = dialog.save_file() else {
                return Ok(None);
            };
            file
        };
        let changed = Arc::default();
        let session = Session::create(
            self.watcher(Arc::clone(&changed)),
            mapset,
            &file,
            self.settings.value.skin_dir.clone(),
        )?;
        Ok(Some((session, changed)))
    }

    /// Opens a project. When its mapset folder is gone, the cached Songs list is asked
    /// where it went (D-82); if it doesn't know, the shell asks the user.
    fn open(&mut self, slot: &mut Option<Session>, path: &Path) -> Result<Option<Outcome>, String> {
        if let Some(moved) = hs_session::moved_mapset(path) {
            let found = songs_cache()
                .and_then(|cache| SongIndex::load(&cache))
                .filter(|index| Some(&index.root) == self.songs_dir().0.as_ref())
                .and_then(|index| index.relocate(&moved.mapset_dir, &moved.diff_files));
            let Some(found) = found else {
                return Ok(Some(Outcome::MapsetMoved {
                    file: path.to_path_buf(),
                    moved,
                }));
            };
            self.open_relocated(slot, path, &found)?;
            return Ok(Some(Outcome::Notice(format!(
                "The mapset moved, so the project now uses {}. Save to keep it.",
                found.display()
            ))));
        }
        let changed = Arc::default();
        let session = Session::open(
            self.watcher(Arc::clone(&changed)),
            path,
            self.settings.value.skin_dir.clone(),
        )?;
        self.set_project(slot, session, changed);
        Ok(None)
    }

    fn open_relocated(
        &mut self,
        slot: &mut Option<Session>,
        path: &Path,
        mapset: &Path,
    ) -> Result<(), String> {
        let changed = Arc::default();
        let session = Session::open_relocated(
            self.watcher(Arc::clone(&changed)),
            path,
            self.settings.value.skin_dir.clone(),
            mapset,
        )?;
        self.set_project(slot, session, changed);
        Ok(())
    }

    fn create(
        &mut self,
        slot: &mut Option<Session>,
        mapset: &Path,
    ) -> Result<Option<Outcome>, String> {
        let Some((session, changed)) = self.new_session(mapset)? else {
            return Ok(None);
        };
        let file = session.file().to_path_buf();
        self.set_project(slot, session, changed);
        let folder = file
            .parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Some(Outcome::Created(format!("Project created: {folder}"))))
    }

    fn change_settings(
        &mut self,
        edit: impl FnOnce(&mut Settings) -> Result<(), String>,
    ) -> Result<(), String> {
        let before = self.settings.value.clone();
        match edit(&mut self.settings) {
            Ok(()) => {
                self.settings.warning = None;
                Ok(())
            }
            Err(error) => {
                self.settings.value = before;
                Err(error)
            }
        }
    }

    fn skin(&mut self, slot: &mut Option<Session>, path: Option<PathBuf>) -> Result<(), String> {
        self.change_settings(|settings| settings.set_skin_dir(path))?;
        if let Some(session) = slot {
            session.set_skin_dir(self.settings.value.skin_dir.clone());
        }
        Ok(())
    }

    fn run(&mut self, job: Job, slot: &mut Option<Session>) -> Result<Option<Outcome>, String> {
        let message = match job {
            Job::Create => {
                let mut dialog = self.dialog("Choose the osu! mapset folder");
                if let Some(songs) = self.songs_dir().0 {
                    dialog = dialog.set_directory(songs);
                }
                let Some(mapset) = dialog.pick_folder() else {
                    return Ok(None);
                };
                return self.create(slot, &mapset);
            }
            Job::CreateFrom(mapset) => return self.create(slot, &mapset),
            Job::ImportHitsounds => {
                let mut dialog = self
                    .dialog("Choose the mania hitsound difficulty to import")
                    .add_filter("osu! difficulty", &["osu"]);
                if let Some(songs) = self.songs_dir().0 {
                    dialog = dialog.set_directory(songs);
                }
                let Some(osu) = dialog.pick_file() else {
                    return Ok(None);
                };
                let diff = import::read(&osu)?;
                // D-66: the .osu's own folder, or its one sibling copy of the same set.
                let mapset = match import::find_mapset(&diff.dir) {
                    Some(dir) => dir,
                    None => {
                        let mut dialog = self.dialog(
                            "No standard or catch difficulties next to it. Choose the mapset these hitsounds are for",
                        );
                        if let Some(songs) = diff.dir.parent() {
                            dialog = dialog.set_directory(songs);
                        }
                        let Some(dir) = dialog.pick_folder() else {
                            return Ok(None);
                        };
                        dir
                    }
                };
                let Some((mut session, changed)) = self.new_session(&mapset)? else {
                    return Ok(None);
                };
                let imported =
                    session.import_hitsound_diff(&diff, |id| theme::channel_default(id).into());
                let saved = session.save();
                let target = session.project.mapset_dir.clone();
                self.set_project(slot, session, changed);
                let summary = imported?;
                saved?;
                return Ok(Some(Outcome::Created(import_notice(
                    &diff, &summary, &target,
                ))));
            }
            Job::ChangeMapset => {
                let session = active(slot)?;
                let mut dialog = self.dialog("Choose the mapset folder for this project");
                if let Some(parent) = session.project.mapset_dir.parent() {
                    dialog = dialog.set_directory(parent);
                }
                let Some(dir) = dialog.pick_folder() else {
                    return Ok(None);
                };
                session.set_mapset(&dir)?;
                self.settings.touch_recent(
                    session.file(),
                    &session.project.name,
                    &session.project.mapset_dir,
                );
                "Mapset changed"
            }
            Job::SetMapset(dir) => {
                let session = active(slot)?;
                session.set_mapset(&dir)?;
                self.settings.touch_recent(
                    session.file(),
                    &session.project.name,
                    &session.project.mapset_dir,
                );
                "Mapset changed"
            }
            Job::Open => {
                let Some(file) = self
                    .dialog("Open a hitsounds-rs project")
                    .add_filter("Project file", &["json"])
                    .pick_file()
                else {
                    return Ok(None);
                };
                return Ok(self
                    .open(slot, &file)?
                    .or(Some(Outcome::Notice("Project opened".into()))));
            }
            Job::OpenRecent(file) => {
                return Ok(self
                    .open(slot, &file)?
                    .or(Some(Outcome::Notice("Project opened".into()))));
            }
            Job::OpenRelocated { file, mapset } => {
                let mapset = match mapset {
                    Some(dir) => dir,
                    None => {
                        let mut dialog = self.dialog("Choose where this project's mapset is now");
                        if let Some(songs) = self.songs_dir().0 {
                            dialog = dialog.set_directory(songs);
                        }
                        let Some(dir) = dialog.pick_folder() else {
                            return Ok(None);
                        };
                        dir
                    }
                };
                self.open_relocated(slot, &file, &mapset)?;
                "Project opened at the new mapset folder. Save to keep it"
            }
            Job::Close => {
                *slot = None;
                self.waveform = None;
                "Project closed"
            }
            Job::Save => {
                active(slot)?.save()?;
                "Project saved"
            }
            Job::Refresh { automatic } => {
                active(slot)?.refresh()?;
                if automatic {
                    return Ok(None);
                }
                "Mapset refreshed"
            }
            Job::ChooseSkin => {
                let Some(path) = self
                    .dialog("Choose the osu! skin sample folder")
                    .pick_folder()
                else {
                    return Ok(None);
                };
                self.skin(slot, Some(readable_dir(path)?))?;
                "Skin folder set"
            }
            Job::ClearSkin => {
                self.skin(slot, None)?;
                "Skin folder cleared"
            }
            Job::ChooseSongs => {
                let mut dialog = self.dialog("Choose the osu! Songs folder");
                if let Some(dir) = self.songs_dir().0 {
                    dialog = dialog.set_directory(dir);
                }
                let Some(path) = dialog.pick_folder() else {
                    return Ok(None);
                };
                let path = readable_dir(path)?;
                self.change_settings(|settings| settings.set_songs_dir(Some(path)))?;
                "Songs folder set"
            }
            Job::ClearSongs => {
                self.detected_songs = detect_songs_dir();
                self.change_settings(|settings| settings.set_songs_dir(None))?;
                if self.detected_songs.is_some() {
                    "Using the Songs folder of your osu! install"
                } else {
                    "Songs folder cleared"
                }
            }
            Job::ChooseProjectsDir => {
                let mut dialog = self.dialog("Choose where new projects go");
                if let Some(dir) = self
                    .settings
                    .value
                    .projects_dir
                    .clone()
                    .or_else(dirs::document_dir)
                {
                    dialog = dialog.set_directory(dir);
                }
                let Some(path) = dialog.pick_folder() else {
                    return Ok(None);
                };
                let path = readable_dir(path)?;
                if self
                    .songs_dir()
                    .0
                    .is_some_and(|songs| path.starts_with(songs))
                {
                    return Err("Choose a projects folder outside the Songs folder, so project files never end up in a mapset.".into());
                }
                self.change_settings(|settings| settings.set_projects_dir(Some(path)))?;
                "Projects folder set"
            }
            Job::ClearProjectsDir => {
                self.change_settings(|settings| settings.set_projects_dir(None))?;
                "New projects will ask where to go"
            }
            Job::AddLibrary => {
                let Some(path) = self.dialog("Add a sample library folder").pick_folder() else {
                    return Ok(None);
                };
                let path = readable_dir(path)?;
                self.change_settings(|settings| settings.add_library(&path))?;
                self.library = None;
                "Library folder added"
            }
            Job::RemoveLibrary(path) => {
                self.change_settings(|settings| settings.remove_library(&path))?;
                self.library = None;
                "Library folder removed"
            }
            Job::ForgetRecent(path) => {
                self.change_settings(|settings| settings.forget_recent(&path))?;
                "Removed from recent projects"
            }
            Job::ReduceMotion(value) => {
                self.change_settings(|settings| settings.set_reduce_motion(value))?;
                return Ok(None);
            }
            Job::UpdateChecks(on) => {
                self.change_settings(|settings| settings.set_update_checks(on))?;
                return Ok(None);
            }
            Job::KeepPitch(on) => {
                // Apply first: a failed save still changes what you hear.
                if let Some(session) = slot {
                    session.set_keep_pitch(on)?;
                }
                self.change_settings(|settings| settings.set_keep_pitch(on))?;
                return Ok(None);
            }
            Job::Keybinds(keybinds) => {
                self.change_settings(|settings| settings.set_keybinds(keybinds))?;
                return Ok(None);
            }
            Job::UiScale(value) => {
                self.change_settings(|settings| settings.set_ui_scale(value))?;
                return Ok(None);
            }
            Job::Levels(levels) => {
                // Apply first: a failed save still changes what you hear.
                if let Some(session) = slot {
                    session.set_levels(&levels)?;
                }
                self.change_settings(|settings| settings.set_levels(levels))?;
                return Ok(None);
            }
            Job::ImportStem => {
                let Some(path) = self
                    .dialog("Choose the aligned drum stem")
                    .add_filter("Audio", &["wav", "ogg", "mp3", "flac"])
                    .pick_file()
                else {
                    return Ok(None);
                };
                active(slot)?.import_audio(&path, true)?;
                "Drum stem imported"
            }
            Job::RemoveStem => {
                active(slot)?.edit(&Edit::SetStem {
                    path: None,
                    origin: None,
                })?;
                "Drum stem cleared"
            }
            Job::ChooseModel => {
                let mut dialog = self
                    .dialog("Choose the drums model made by tools/demucs-export")
                    .add_filter("ONNX model", &["onnx"]);
                if let Some(dir) = self
                    .settings
                    .value
                    .separation_model
                    .as_deref()
                    .and_then(Path::parent)
                {
                    dialog = dialog.set_directory(dir);
                }
                let Some(path) = dialog.pick_file() else {
                    return Ok(None);
                };
                separation::check_model(&path, &runtime_path())?;
                self.change_settings(|settings| settings.set_separation_model(Some(path)))?;
                "Drums model ready"
            }
            Job::ClearModel => {
                self.change_settings(|settings| settings.set_separation_model(None))?;
                "Drums model cleared"
            }
            Job::Separate => {
                let model = self.settings.value.separation_model.clone().ok_or(
                    "Choose the drums model in App settings first. tools/demucs-export makes it.",
                )?;
                let request = active(slot)?.separation_request(&model, &runtime_path())?;
                return Ok(Some(Outcome::Separate(Box::new(request))));
            }
            Job::ApplySeparation(stem) => {
                active(slot)?.apply_separation(&stem)?;
                let mut text = if stem.reused {
                    "Drum stem restored from the earlier separation of this song".to_owned()
                } else {
                    "Drum stem separated".to_owned()
                };
                if let Some(db) = stem.scaled_db {
                    text.push_str(&format!(". Turned down {:.1} dB so it doesn't clip", -db));
                }
                return Ok(Some(Outcome::Notice(text)));
            }
            Job::Fallback(channel) => {
                active(slot)?.edit(&Edit::SetFallback { channel })?;
                return Ok(None);
            }
            Job::History { redo } => {
                active(slot)?.step_history(redo);
                return Ok(None);
            }
            Job::Monitor(monitor) => {
                active(slot)?.set_monitor(monitor)?;
                return Ok(None);
            }
            Job::SelectLayer(file) => {
                active(slot)?.select_diff(file)?;
                return Ok(None);
            }
            Job::Edit(edit) => {
                active(slot)?.edit(&edit)?;
                return Ok(None);
            }
            Job::AddChannel => {
                let session = active(slot)?;
                let id = session.project.next_channel_id;
                add_channel(session, format!("Channel {id}"), SampleSource::SkinDefault)?;
                return Ok(Some(Outcome::SelectChannel(id)));
            }
            Job::UseSample { channel, path } => {
                let file = match path {
                    Some(path) => path,
                    None => {
                        let Some(path) = self
                            .dialog("Choose a channel sample")
                            .add_filter("Audio", &["wav", "ogg", "mp3", "flac"])
                            .pick_file()
                        else {
                            return Ok(None);
                        };
                        path
                    }
                };
                let session = active(slot)?;
                let target = channel
                    .map(|id| {
                        session
                            .project
                            .channel(id)
                            .map(|c| (id, ChannelProps::from(c)))
                            .ok_or("This channel no longer exists.")
                    })
                    .transpose()?;
                // D-45: only copy a library file when it is assigned to a channel.
                let path = session.import_audio(&file, false)?;
                let sample = SampleSource::File { path };
                let id = if let Some((id, mut props)) = target {
                    props.sample = sample;
                    session.edit(&Edit::SetChannel { id, props })?;
                    id
                } else {
                    let id = session.project.next_channel_id;
                    let name = file.file_stem().map_or_else(
                        || format!("Channel {id}"),
                        |name| name.to_string_lossy().into_owned(),
                    );
                    add_channel(session, name, sample)?;
                    id
                };
                return Ok(Some(Outcome::SelectChannel(id)));
            }
            Job::AuditionFile(path) => {
                active(slot)?.audition_file(&path)?;
                return Ok(None);
            }
            Job::AuditionChannel(id) => {
                active(slot)?.audition(id)?;
                return Ok(None);
            }
            Job::ScanLibrary => {
                self.library = Some(Arc::new(
                    self.settings
                        .value
                        .library
                        .iter()
                        .map(|path| library::scan(path))
                        .collect(),
                ));
                return Ok(None);
            }
            Job::BuildExport(mut export) => {
                let session = active(slot)?;
                // Project settings owns this one (D-78); the export page only shows it.
                export.addition_only_normal = session.project.export.addition_only_normal;
                session.edit(&Edit::SetExport { export })?;
                return Ok(Some(Outcome::ExportPreview(Box::new(ExportReview::new(
                    session.plan_export(),
                )))));
            }
            Job::ReadDiff(file) => {
                let diff = active(slot)?
                    .pending_export()?
                    .diffs
                    .iter()
                    .find(|diff| diff.file == file)
                    .ok_or("This difficulty is outside the export preview.")?;
                return Ok(Some(Outcome::ExportDiff(LineDiff::new(diff))));
            }
            Job::WriteExport => {
                let result = active(slot)?.export()?;
                return Ok(Some(Outcome::Exported(result)));
            }
        };
        Ok(Some(Outcome::Notice(message.into())))
    }
}

fn import_notice(
    diff: &import::HitsoundDiff,
    summary: &import::ImportSummary,
    mapset: &Path,
) -> String {
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let mut text = format!(
        "Imported {} and {} from {}",
        plural(summary.channels, "channel"),
        plural(summary.hits, "hit"),
        diff.label
    );
    if mapset != diff.dir {
        let name = mapset
            .file_name()
            .map_or_else(|| mapset.to_string_lossy(), |n| n.to_string_lossy());
        text.push_str(&format!(" for {name}"));
    }
    text.push('.');
    if summary.volume_points > 0 {
        text.push_str(" Greenline volume is on the Volume lane.");
    }
    if !summary.failed.is_empty() {
        text.push_str(&format!(
            " Could not read {}, so those channels play the skin sound.",
            summary.failed.join(", ")
        ));
    }
    text
}

fn add_channel(session: &mut Session, name: String, sample: SampleSource) -> Result<(), String> {
    let id = session.project.next_channel_id;
    session.edit(&Edit::AddChannel {
        props: ChannelProps {
            name,
            color: theme::channel_default(id).into(),
            sample_set: SampleSet::Soft,
            hitsound: Hitsound::Normal,
            sample,
            volume: 1.0,
        },
        index: None,
        events: Vec::new(),
    })
}

fn active(slot: &mut Option<Session>) -> Result<&mut Session, String> {
    slot.as_mut()
        .ok_or_else(|| "Open or create a project first.".into())
}

/// `<dir>/<name>.hsproj`, or `<name> (2).hsproj` and so on when that is taken. Characters
/// Windows forbids in file names become `_`.
fn free_project_path(dir: &Path, name: &str) -> PathBuf {
    let clean: String = name
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim_end_matches(['.', ' ']);
    let clean = if clean.is_empty() {
        "New project"
    } else {
        clean
    };
    (1..)
        .map(|n| match n {
            1 => dir.join(format!("{clean}.hsproj")),
            n => dir.join(format!("{clean} ({n}).hsproj")),
        })
        .find(|path| !path.exists())
        .expect("some numbered name is free")
}

fn readable_dir(path: PathBuf) -> Result<PathBuf, String> {
    let result = path
        .canonicalize()
        .and_then(|dir| std::fs::read_dir(&dir).map(|_| dir));
    result.map_err(|e| format!("Could not read the folder {}. {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_folder_comes_from_the_osz_open_command() {
        assert_eq!(
            install_from_command(r#""D:\Games\osu!\osu!.exe" "%1""#),
            Some(PathBuf::from(r"D:\Games\osu!"))
        );
        assert_eq!(
            install_from_command(r#"C:\osu!\osu!.exe "%1""#),
            Some(PathBuf::from(r"C:\osu!"))
        );
        assert_eq!(install_from_command(r#""C:\Other\app.exe" "%1""#), None);
    }

    #[test]
    fn project_names_avoid_taken_and_forbidden_names() {
        let dir = tempfile::tempdir().unwrap();
        let first = free_project_path(dir.path(), "A: B?");
        assert_eq!(first, dir.path().join("A_ B_.hsproj"));
        std::fs::create_dir(&first).unwrap();
        assert_eq!(
            free_project_path(dir.path(), "A: B?"),
            dir.path().join("A_ B_ (2).hsproj")
        );
        assert_eq!(
            free_project_path(dir.path(), " . "),
            dir.path().join("New project.hsproj")
        );
    }
}
