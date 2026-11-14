//! One open project, its undo history, and the editor's cached mapset and playback.

use crate::mapset::{Mapset, is_filename, is_osu};
use crate::playback::{Playback, Waveform};
use crate::separation::{self, SeparatedStem};
use hs_audio::{Command, decode_file};
use hs_core::packing::Source;
use hs_core::project::Monitor;
use hs_core::projection::{Redline, project};
use hs_core::{DiffTimeline, Edit, History, Ms, Project, Projection, SampleSource, StemOrigin};
use hs_export::{ExportPlan, ExportReport, PreviewDiff, SampleLibrary};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Called from the file watcher's thread with the mapset folder whenever a `.osu` in it
/// changes (D-20). The UI shell refreshes the session in response.
pub type OnMapsetChange = Arc<dyn Fn(&Path) + Send + Sync>;

pub struct Session {
    pub project: Project,
    saved: Project,
    file: PathBuf,
    history: History,
    mapset: Mapset,
    active: Option<String>,
    library: SampleLibrary,
    pub playback: Playback,
    pending_export: Option<ExportPlan>,
    watcher: Option<RecommendedWatcher>,
    watch_warning: Option<String>,
    on_change: OnMapsetChange,
    /// The app-wide skin folder (D-40), absolute.
    skin_dir: Option<PathBuf>,
    /// The song file's stamp and SHA-256, hashed only while the stem was separated in
    /// the app, to notice a changed song (D-73).
    song_hash: Option<(FileStamp, String)>,
}

/// Enough of a file's metadata to notice it was replaced.
#[derive(Debug, Clone, PartialEq)]
struct FileStamp {
    path: PathBuf,
    len: u64,
    modified: Option<std::time::SystemTime>,
}

impl FileStamp {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            path: path.to_path_buf(),
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

#[derive(Serialize)]
pub struct Workspace<'a> {
    project: &'a Project,
    file: &'a Path,
    pub active_diff: Option<&'a str>,
    pub available_diffs: Vec<&'a str>,
    pub redlines: &'a [Redline],
    pub timeline: Option<&'a DiffTimeline>,
    pub projection: Option<Projection>,
    pub events: Option<Vec<(u32, Ms)>>,
    pub length_ms: f64,
    waveform_version: u32,
    pub stem_ready: bool,
    /// The stem was separated from a different version of the song (D-73).
    pub stem_outdated: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub dirty: bool,
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct CompletedExport {
    pub report: ExportReport,
    pub warning: Option<String>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn directory(path: &Path) -> Result<PathBuf, String> {
    let resolved = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !resolved.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }
    Ok(resolved)
}

/// A project whose mapset folder is gone, as [`moved_mapset`] reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct MovedMapset {
    pub name: String,
    /// The folder the project names, which no longer exists.
    pub mapset_dir: PathBuf,
    /// The project's difficulty files, to recognise the mapset elsewhere.
    pub diff_files: Vec<String>,
}

/// `Some` when the project at `path` reads but its mapset folder is missing, so the
/// shell can look for the mapset before opening it (D-82). Any other problem is left
/// for [`Session::open`] to report.
pub fn moved_mapset(path: &Path) -> Option<MovedMapset> {
    let file = path.canonicalize().ok()?;
    let project = Project::load(&file).ok()?;
    let mapset_dir = file.parent()?.join(&project.mapset_dir);
    (!mapset_dir.is_dir()).then(|| MovedMapset {
        name: project.name.clone(),
        mapset_dir: project.mapset_dir.clone(),
        diff_files: project.diffs.iter().map(|d| d.file.clone()).collect(),
    })
}

fn outside_mapset(dir: &Path, mapset: &Path) -> Result<(), String> {
    if dir.starts_with(mapset) {
        return Err("Choose a project location outside the mapset folder.".into());
    }
    Ok(())
}

impl Session {
    pub fn create(
        on_change: OnMapsetChange,
        mapset_dir: &Path,
        path: &Path,
        skin_dir: Option<PathBuf>,
    ) -> Result<Self, String> {
        let mapset_dir = directory(mapset_dir)?;
        let mapset = read_targets(&mapset_dir)?;
        let reference = mapset.reference().expect("read_targets found a difficulty");
        let parent = directory(path.parent().ok_or("Choose a project location.")?)?;
        outside_mapset(&parent, &mapset_dir)?;
        let mut folder = path
            .file_name()
            .ok_or("Choose a project name.")?
            .to_os_string();
        if !path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("hsproj"))
        {
            folder.push(".hsproj");
        }
        let project_dir = parent.join(folder);
        let file = project_dir.join("project.json");
        let name = reference.title.clone().unwrap_or_else(|| {
            mapset_dir
                .file_name()
                .map_or_else(|| "Untitled".into(), |n| n.to_string_lossy().into_owned())
        });
        let mut project = Project::new(name, mapset_dir);
        mapset.update_profiles(&mut project);
        std::fs::create_dir(&project_dir).map_err(|e| format!("{}: {e}", project_dir.display()))?;
        if let Err(e) = project.save(&file) {
            let _ = std::fs::remove_dir(&project_dir);
            return Err(err(e));
        }
        Ok(Self::new(on_change, file, project, mapset, skin_dir))
    }

    /// Opens a project. With no app-wide skin folder yet, a format-1 project's own
    /// `skin_dir` is used instead; the caller adopts it as the app setting (D-40).
    pub fn open(
        on_change: OnMapsetChange,
        path: &Path,
        skin_dir: Option<PathBuf>,
    ) -> Result<Self, String> {
        Self::open_at(on_change, path, skin_dir, None)
    }

    /// Opens a project whose mapset folder moved, with the mapset at `mapset` (D-82).
    /// The new folder is an unsaved change, so saving the project keeps it.
    pub fn open_relocated(
        on_change: OnMapsetChange,
        path: &Path,
        skin_dir: Option<PathBuf>,
        mapset: &Path,
    ) -> Result<Self, String> {
        Self::open_at(on_change, path, skin_dir, Some(mapset))
    }

    fn open_at(
        on_change: OnMapsetChange,
        path: &Path,
        skin_dir: Option<PathBuf>,
        relocated: Option<&Path>,
    ) -> Result<Self, String> {
        let file = path.canonicalize().map_err(err)?;
        let mut project = Project::load(&file).map_err(err)?;
        let root = file.parent().ok_or("Project file has no parent folder.")?;
        let legacy = project.legacy_skin_dir.take().map(|p| root.join(p));
        let skin_dir = skin_dir.or(legacy);
        let stored_mapset = project.mapset_dir.clone();
        project.mapset_dir = match relocated {
            Some(dir) => {
                let dir = directory(dir)?;
                read_targets(&dir)?;
                dir
            }
            None => directory(&root.join(&project.mapset_dir))?,
        };
        outside_mapset(root, &project.mapset_dir)?;
        if let Some(invalid) = project
            .diffs
            .iter()
            .find(|d| !is_filename(&d.file) || !is_osu(Path::new(&d.file)))
        {
            return Err(format!(
                "Invalid difficulty filename in project: {}",
                invalid.file
            ));
        }
        let mapset = Mapset::read(&project.mapset_dir)?;
        let mut session = Self::new(on_change, file, project, mapset, skin_dir);
        if relocated.is_some() {
            session.saved.mapset_dir = stored_mapset;
        }
        Ok(session)
    }

    fn new(
        on_change: OnMapsetChange,
        file: PathBuf,
        project: Project,
        mapset: Mapset,
        skin_dir: Option<PathBuf>,
    ) -> Self {
        let library = SampleLibrary::new(file.parent().expect("project file has a parent"), None);
        let mut session = Self {
            saved: project.clone(),
            project,
            file,
            history: History::default(),
            mapset,
            active: None,
            library,
            playback: Playback::new(),
            pending_export: None,
            watcher: None,
            watch_warning: None,
            on_change,
            skin_dir,
            song_hash: None,
        };
        session.mapset.update_profiles(&mut session.project);
        session.watch();
        session.sync_playback();
        session
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn skin_dir(&self) -> Option<&Path> {
        self.skin_dir.as_deref()
    }

    pub fn set_skin_dir(&mut self, dir: Option<PathBuf>) {
        if self.skin_dir != dir {
            self.skin_dir = dir;
            self.pending_export = None;
            self.sync_playback();
        }
    }

    fn root(&self) -> &Path {
        self.file.parent().expect("project file has a parent")
    }

    fn watch(&mut self) {
        self.watcher = None;
        self.watch_warning = None;
        let on_change = Arc::clone(&self.on_change);
        let dir = self.project.mapset_dir.clone();
        let watch = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let relevant = event.as_ref().map_or(true, |e| {
                !matches!(e.kind, EventKind::Access(_))
                    && (e.need_rescan()
                        || e.paths
                            .iter()
                            .any(|p| is_osu(p) || crate::library::is_audio(p)))
            });
            if relevant {
                on_change(&dir);
            }
        })
        .and_then(|mut watcher| {
            watcher.watch(&self.project.mapset_dir, RecursiveMode::NonRecursive)?;
            Ok(watcher)
        });
        match watch {
            Ok(watcher) => self.watcher = Some(watcher),
            Err(e) => {
                self.watch_warning = Some(format!(
                    "Live sync unavailable: {e}. Use Refresh mapset to rescan."
                ))
            }
        }
    }

    pub fn refresh(&mut self) -> Result<(), String> {
        let mapset = Mapset::read(&self.project.mapset_dir)?;
        self.load_mapset(mapset);
        Ok(())
    }

    fn load_mapset(&mut self, mapset: Mapset) {
        self.pending_export = None;
        self.mapset = mapset;
        self.mapset.update_profiles(&mut self.project);
        if self
            .active
            .as_deref()
            .is_some_and(|f| self.project.diff(f).is_none())
        {
            self.active = None;
        }
        self.library.clear();
        self.playback.clear_samples();
        self.sync_playback();
    }

    /// Points the project at another mapset folder (D-67), e.g. the real mapset after
    /// working from a copy. Difficulties the new folder lacks lose their profile unless
    /// it holds settings or overrides. Channels, hits and the envelope are untouched.
    /// Undo moves back to the previous folder.
    pub fn set_mapset(&mut self, dir: &Path) -> Result<(), String> {
        let dir = directory(dir)?;
        if dir == self.project.mapset_dir {
            return Ok(());
        }
        outside_mapset(self.root(), &dir)?;
        let mapset = read_targets(&dir)?;
        let before = self.project.clone();
        self.project.mapset_dir = dir;
        self.project.diffs.retain(|d| {
            mapset.diffs.contains_key(&d.file) || *d != hs_core::DiffProfile::new(&d.file, &d.label)
        });
        self.history.record(before);
        self.load_mapset(mapset);
        self.watch();
        Ok(())
    }

    pub fn workspace(&self) -> Workspace<'_> {
        let profile = self
            .active
            .as_deref()
            .and_then(|file| self.project.diff(file));
        let difficulty = self
            .active
            .as_deref()
            .and_then(|file| self.mapset.diffs.get(file));
        let reference = self.mapset.reference();
        let timing = difficulty.or(reference);
        let last_event = self
            .project
            .base_events()
            .chain(
                profile
                    .into_iter()
                    .flat_map(|d| d.apply_overrides(self.project.base_events())),
            )
            .map(|(_, t)| t)
            .max()
            .unwrap_or_default();
        let last_object = timing
            .and_then(|d| d.timeline.edges.last())
            .map_or(0, |e| e.time);
        let mut warnings = self.mapset.warnings.clone();
        warnings.extend(self.watch_warning.iter().cloned());
        warnings.extend(self.playback.warnings());
        let stem_outdated = self.stem_outdated();
        if stem_outdated {
            warnings.push("The song changed since the drum stem was separated from it. Separate it again in Project settings.".into());
        }
        for d in &self.project.diffs {
            if !self.mapset.diffs.contains_key(&d.file) {
                warnings.push(format!("{} is missing or unreadable. Refresh after restoring it, or disable its export.", d.label));
            }
        }
        Workspace {
            project: &self.project,
            file: &self.file,
            active_diff: self.active.as_deref(),
            available_diffs: self.mapset.diffs.keys().map(String::as_str).collect(),
            redlines: timing.map_or(&[], |d| d.timeline.redlines.as_slice()),
            timeline: difficulty.map(|d| &d.timeline),
            projection: profile
                .zip(difficulty)
                .map(|(p, d)| project(&self.project, p, &d.timeline)),
            events: profile.map(|p| p.apply_overrides(self.project.base_events())),
            length_ms: self
                .playback
                .length_ms()
                .max(f64::from(last_event.max(last_object)) + 1000.0),
            waveform_version: self.playback.waveform_version,
            stem_ready: self.playback.stem_ready(),
            stem_outdated,
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
            dirty: self.project != self.saved,
            warnings,
        }
    }

    pub fn waveform(&self) -> Waveform<'_> {
        self.playback.waveform(self.project.audio.monitor)
    }

    /// Used for layer badges and review outside the active difficulty (D-49, D-50).
    pub fn projection_for(&self, file: &str) -> Option<Projection> {
        let profile = self.project.diff(file)?;
        let difficulty = self.mapset.diffs.get(file)?;
        Some(project(&self.project, profile, &difficulty.timeline))
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.project.save(&self.file).map_err(err)?;
        self.saved.clone_from(&self.project);
        Ok(())
    }

    pub fn select_diff(&mut self, file: Option<String>) -> Result<(), String> {
        if file
            .as_deref()
            .is_some_and(|f| self.project.diff(f).is_none())
        {
            return Err("This difficulty is outside the project.".into());
        }
        self.active = file;
        self.sync_preview();
        Ok(())
    }

    pub fn edit(&mut self, edit: &Edit) -> Result<(), String> {
        let before = self.project.clone();
        self.project.apply(edit).map_err(err)?;
        if self.project != before {
            if edit.undoable() {
                self.history.record(before);
                self.pending_export = None;
            }
            self.sync_playback();
        }
        Ok(())
    }

    pub fn step_history(&mut self, redo: bool) {
        let before = self.project.clone();
        let changed = if redo {
            self.history.redo(&mut self.project)
        } else {
            self.history.undo(&mut self.project)
        };
        if changed {
            // Disk discoveries, exports, and monitor controls live outside edit history.
            self.project.audio.monitor = before.audio.monitor;
            self.project.schema = before.schema;
            self.project.next_channel_id = self.project.next_channel_id.max(before.next_channel_id);
            for channel in &mut self.project.channels {
                if let Some(current) = before.channels.iter().find(|c| c.id == channel.id) {
                    channel.muted = current.muted;
                    channel.solo = current.solo;
                }
            }
            if self.project.mapset_dir != before.mapset_dir {
                // D-67: the mapset folder is part of the history, so follow it on disk.
                match Mapset::read(&self.project.mapset_dir) {
                    Ok(mapset) => {
                        self.load_mapset(mapset);
                        self.watch();
                        return;
                    }
                    Err(_) => self.project.mapset_dir = before.mapset_dir,
                }
            }
            self.mapset.update_profiles(&mut self.project);
            self.pending_export = None;
            self.sync_playback();
        }
    }

    fn sync_playback(&mut self) {
        if self.library.skin_dir() != self.skin_dir.as_deref() {
            self.library.set_skin_dir(self.skin_dir.clone());
            self.playback.clear_samples();
        }
        self.hash_song();
        let original = self.mapset.reference().and_then(|d| d.audio.as_deref());
        let stem_matches_song = self
            .stem_song_hashes(original)
            .is_some_and(|(stem_hash, song_hash)| stem_hash == song_hash);
        let stem = self
            .project
            .audio
            .stem
            .as_ref()
            .map(|p| self.root().join(p));
        self.playback.sync_project_tracks(
            original,
            stem.as_deref(),
            self.project.audio.monitor,
            stem_matches_song,
        );
        self.sync_preview();
    }

    /// The song the stem is separated from and monitored against.
    fn song(&self) -> Option<&Path> {
        self.mapset.reference().and_then(|d| d.audio.as_deref())
    }

    /// Re-hashes the song when the stem is a separated one and the song file changed.
    fn hash_song(&mut self) {
        if self.project.audio.stem_origin.is_none() {
            return;
        }
        let stamp = self.song().and_then(FileStamp::of);
        if self.song_hash.as_ref().map(|(s, _)| s) == stamp.as_ref() {
            return;
        }
        self.song_hash = stamp.and_then(|stamp| {
            let hash = separation::sha256_file(&stamp.path).ok()?;
            Some((stamp, hash))
        });
    }

    fn stem_song_hashes(&self, song: Option<&Path>) -> Option<(&str, &str)> {
        let StemOrigin::Separated { song_sha256, .. } = self.project.audio.stem_origin.as_ref()?;
        let (stamp, hash) = self.song_hash.as_ref()?;
        (stamp.path == song?).then_some((song_sha256.as_str(), hash.as_str()))
    }

    fn stem_outdated(&self) -> bool {
        self.stem_song_hashes(self.song())
            .is_some_and(|(stem_hash, song_hash)| stem_hash != song_hash)
    }

    /// What a drum separation needs (D-70). `model` is the user's ONNX export (D-71),
    /// `runtime` the bundled `onnxruntime.dll`. Run the request on its own thread
    /// ([`separation::Separation`]) and pass its result to [`Self::apply_separation`].
    pub fn separation_request(
        &self,
        model: &Path,
        runtime: &Path,
    ) -> Result<separation::Request, String> {
        let song = self
            .song()
            .ok_or("The reference difficulty has no readable song (AudioFilename) to separate.")?;
        if !model.is_file() {
            return Err(format!(
                "The drums model {} is missing. Choose it again in Settings.",
                model.display()
            ));
        }
        if !runtime.is_file() {
            return Err(format!(
                "ONNX Runtime is missing ({}). Reinstall hitsounds-rs to separate stems.",
                runtime.display()
            ));
        }
        let known = std::iter::once(&self.project)
            .chain(std::iter::once(&self.saved))
            .chain(self.history.states())
            .filter_map(|p| Some((p.audio.stem.clone()?, p.audio.stem_origin.clone()?)))
            .collect();
        Ok(separation::Request {
            root: self.root().to_path_buf(),
            song: song.to_path_buf(),
            model: model.to_path_buf(),
            runtime: runtime.to_path_buf(),
            known,
        })
    }

    /// Makes a finished separation the project's stem, as one undo step (D-72).
    pub fn apply_separation(&mut self, stem: &SeparatedStem) -> Result<(), String> {
        if stem.root != self.root() {
            return Err("The separated stem belongs to another project.".into());
        }
        if self.song() != Some(stem.song.as_path()) {
            return Err("The song changed while separating. Separate it again.".into());
        }
        self.edit(&Edit::SetStem {
            path: Some(stem.path.clone()),
            origin: Some(stem.origin.clone()),
        })
    }

    fn sync_preview(&mut self) {
        let diff = self.active.as_ref().and_then(|f| {
            let profile = self.project.diff(f)?;
            let d = self.mapset.diffs.get(f)?;
            Some(PreviewDiff {
                profile,
                timeline: &d.timeline,
                objects: &d.objects,
                points: &d.points,
                general_set: d.general_set,
            })
        });
        // The base layer borrows the greenline sets of the first difficulty, enabled
        // ones first, so its lone additions sound as they will in export (D-91).
        let base_sets = self
            .project
            .diffs
            .iter()
            .filter(|d| d.enabled)
            .chain(&self.project.diffs)
            .find_map(|d| self.mapset.diffs.get(&d.file))
            .map(|d| &d.timeline);
        let mut preview =
            hs_export::preview_in_project(&self.project, diff.as_ref(), base_sets, self.root());
        if self.active.is_some() && diff.is_none() {
            preview = Ok(Vec::new());
        }
        self.playback
            .set_schedule(preview, &mut self.library, self.project.export.auto_fix);
    }

    pub fn set_monitor(&mut self, monitor: Monitor) -> Result<(), String> {
        if monitor == Monitor::Stems && !self.playback.stem_ready() {
            return Err("Separate or import a drum stem in Project settings first.".into());
        }
        self.project.audio.monitor = monitor;
        self.playback.monitor(monitor);
        Ok(())
    }

    pub fn audition(&mut self, id: u32) -> Result<(), String> {
        let channel = self
            .project
            .channel(id)
            .ok_or("This channel no longer exists.")?;
        let source = match &channel.sample {
            SampleSource::SkinDefault => Source::Skin {
                set: channel.sample_set,
                sound: channel.hitsound,
            },
            SampleSource::File { path } => Source::File { path: path.clone() },
        };
        let audio = self.library.load(&source).map_err(err)?;
        self.playback.audition(&audio, channel.volume)
    }

    pub fn audition_file(&self, path: &Path) -> Result<(), String> {
        if !crate::library::is_audio(path) {
            return Err("Choose a WAV, OGG, MP3 or FLAC file.".into());
        }
        let audio = decode_file(path).map_err(err)?;
        self.playback.audition(&audio, 1.0)
    }

    pub fn import_audio(&mut self, path: &Path, stem: bool) -> Result<PathBuf, String> {
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "wav" | "ogg" | "mp3" | "flac") {
            return Err("Choose a WAV, OGG, MP3 or FLAC file.".into());
        }
        // Validate before copying so a bad choice never replaces the current stem/sample.
        let audio = decode_file(path).map_err(err)?;
        if audio.frames() == 0 && stem {
            return Err("A stem must contain audio.".into());
        }
        drop(audio);
        let mut source = std::fs::File::open(path).map_err(err)?;
        let dir = if stem { "stems" } else { "samples" };
        std::fs::create_dir_all(self.root().join(dir)).map_err(err)?;
        let name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .ok_or("This file has no name.")?;
        for number in 1.. {
            let filename = if number == 1 {
                format!("{name}.{extension}")
            } else {
                format!("{name} ({number}).{extension}")
            };
            let relative = Path::new(dir).join(filename);
            let target = self.root().join(&relative);
            match std::fs::File::create_new(&target) {
                Ok(mut file) => {
                    let result = std::io::copy(&mut source, &mut file).and_then(|_| file.flush());
                    drop(file);
                    if let Err(e) = result {
                        let _ = std::fs::remove_file(&target);
                        return Err(err(e));
                    }
                    if stem {
                        self.edit(&Edit::SetStem {
                            path: Some(relative.clone()),
                            origin: None,
                        })?;
                    }
                    return Ok(relative);
                }
                // The same file imported again (e.g. from the library) reuses the copy.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if same_contents(path, &target) {
                        if stem {
                            self.edit(&Edit::SetStem {
                                path: Some(relative.clone()),
                                origin: None,
                            })?;
                        }
                        return Ok(relative);
                    }
                }
                Err(e) => return Err(err(e)),
            }
        }
        unreachable!()
    }

    pub fn plan_export(&mut self) -> &ExportPlan {
        self.library.clear();
        self.playback.clear_samples();
        let root = self.root().to_path_buf();
        let plan = hs_export::export::plan(&self.project, &root, &mut self.library);
        self.pending_export.insert(plan)
    }

    pub fn pending_export(&self) -> Result<&ExportPlan, String> {
        self.pending_export
            .as_ref()
            .ok_or_else(|| "The export preview is out of date. Generate it again.".into())
    }

    pub fn export(&mut self) -> Result<CompletedExport, String> {
        self.pending_export()?;
        self.save()?;
        let plan = self
            .pending_export
            .take()
            .ok_or("The export preview is out of date. Generate it again.")?;
        let report = hs_export::execute(&plan, self.root()).map_err(err)?;
        if plan.mode == hs_core::project::WriteMode::Direct {
            self.project.schema = plan.schema;
        }
        let warning = self
            .save()
            .err()
            .map(|e| format!("Export succeeded. The index schema still needs to be saved: {e}"));
        Ok(CompletedExport { report, warning })
    }

    /// Applies the listening levels to playback (D-77). Monitoring only.
    pub fn set_levels(&self, levels: &crate::settings::Levels) -> Result<(), String> {
        let (music, hitsounds, auditions) = levels.gains();
        self.playback.command(Command::SetVolumes { music, hitsounds, auditions })
    }

    pub fn transport(&self, action: Transport) -> Result<(), String> {
        self.playback.command(match action {
            Transport::Play => Command::Play,
            Transport::Pause => Command::Pause,
            Transport::Seek { ms } => Command::Seek(ms),
            Transport::Rate { rate } => Command::SetRate(rate.clamp(0.25, 1.0)),
        })
    }
}

/// Reads a mapset that has at least one standard or catch difficulty to hitsound.
fn read_targets(dir: &Path) -> Result<Mapset, String> {
    let mapset = Mapset::read(dir)?;
    if mapset.reference().is_none() {
        return Err(format!(
            "No readable standard or catch difficulties were found in {}. {}",
            dir.display(),
            mapset.warnings.join(" ")
        ));
    }
    Ok(mapset)
}

fn same_contents(a: &Path, b: &Path) -> bool {
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).ok();
    size(a).is_some() && size(a) == size(b) && std::fs::read(a).ok() == std::fs::read(b).ok()
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transport {
    Play,
    Pause,
    Seek { ms: f64 },
    Rate { rate: f64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(version: &str) -> String {
        format!(
            "osu file format v14\n\n[General]\nMode: 0\n\n[Metadata]\nVersion:{version}\n\n[TimingPoints]\n0,500,4,2,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n"
        )
    }

    fn mapset(root: &Path, name: &str, diffs: &[&str]) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for version in diffs {
            std::fs::write(
                dir.join(format!("A - B (me) [{version}].osu")),
                diff(version),
            )
            .unwrap();
        }
        dir.canonicalize().unwrap()
    }

    #[test]
    fn changing_the_mapset_keeps_hits_and_customised_profiles_and_undoes() {
        let root = tempfile::tempdir().unwrap();
        let copy = mapset(root.path(), "1 A - B - hs", &["Hard", "Old"]);
        let real = mapset(root.path(), "1 A - B", &["Hard", "Insane"]);
        let mut session =
            Session::create(Arc::new(|_| {}), &copy, &root.path().join("p"), None).unwrap();
        let old = "A - B (me) [Old].osu".to_owned();
        session
            .edit(&Edit::Batch {
                edits: vec![
                    Edit::AddChannel {
                        props: hs_core::ChannelProps::from(&hs_core::Channel::new(
                            0,
                            "Kick",
                            osu_format::SampleSet::Soft,
                            osu_format::Hitsound::Normal,
                        )),
                        index: None,
                        events: vec![1000],
                    },
                    Edit::AddEvents {
                        diff: Some("A - B (me) [Hard].osu".into()),
                        events: vec![(1, 2000)],
                    },
                ],
            })
            .unwrap();
        session.select_diff(Some(old.clone())).unwrap();

        session.set_mapset(&real).unwrap();
        assert_eq!(session.project.mapset_dir, real);
        let files: Vec<_> = session
            .project
            .diffs
            .iter()
            .map(|d| d.file.as_str())
            .collect();
        // Hard keeps its override, Old had nothing worth keeping, Insane is new.
        assert_eq!(files, ["A - B (me) [Hard].osu", "A - B (me) [Insane].osu"]);
        assert_eq!(
            session
                .project
                .diff("A - B (me) [Hard].osu")
                .unwrap()
                .overrides
                .len(),
            1
        );
        assert_eq!(session.project.channels[0].events, vec![1000]);
        assert_eq!(
            session.workspace().active_diff,
            None,
            "the open layer left with its file"
        );

        session.step_history(false);
        assert_eq!(session.project.mapset_dir, copy);
        assert!(session.project.diff(&old).is_some());
        assert_eq!(session.workspace().available_diffs.len(), 2);
    }

    #[test]
    fn a_separated_stem_is_one_undo_step_and_goes_out_of_date_with_the_song() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("1 A - B");
        std::fs::create_dir_all(&dir).unwrap();
        let osu = diff("Hard").replace("Mode: 0", "AudioFilename: audio.wav\nMode: 0");
        std::fs::write(dir.join("A - B (me) [Hard].osu"), osu).unwrap();
        let song = |frames| {
            hs_export::wav::encode_pcm16(&hs_audio::AudioBuffer::silent(44_100, 2, frames))
        };
        std::fs::write(dir.join("audio.wav"), song(44_100)).unwrap();
        let dir = dir.canonicalize().unwrap();
        let mut session =
            Session::create(Arc::new(|_| {}), &dir, &root.path().join("p"), None).unwrap();
        let project = session.root().to_path_buf();
        let model = root.path().join("drums.onnx");
        std::fs::write(&model, b"model").unwrap();
        let runtime = root.path().join("onnxruntime.dll");
        std::fs::write(&runtime, b"dll").unwrap();

        let request = session.separation_request(&model, &runtime).unwrap();
        assert_eq!(request.song, dir.join("audio.wav"));
        assert!(request.known.is_empty());
        // Stand in for the thread's result: a real OGG at the song's length.
        std::fs::create_dir(project.join("stems")).unwrap();
        let ogg =
            hs_export::encode::encode_vorbis(&hs_audio::AudioBuffer::silent(44_100, 2, 44_100));
        std::fs::write(project.join("stems/drums.ogg"), ogg.unwrap()).unwrap();
        let origin = StemOrigin::Separated {
            model_sha256: separation::sha256_file(&model).unwrap(),
            song_sha256: separation::sha256_file(&request.song).unwrap(),
        };
        let stem = SeparatedStem {
            root: project.clone(),
            song: request.song.clone(),
            path: "stems/drums.ogg".into(),
            origin: origin.clone(),
            reused: false,
            scaled_db: None,
        };
        session.apply_separation(&stem).unwrap();
        assert_eq!(session.project.audio.stem_origin.as_ref(), Some(&origin));
        let view = session.workspace();
        assert!(view.stem_ready && !view.stem_outdated && view.can_undo);

        // The song is replaced in the mapset: the stem no longer matches it.
        std::fs::write(dir.join("audio.wav"), song(44_200)).unwrap();
        session.refresh().unwrap();
        let view = session.workspace();
        assert!(view.stem_outdated);
        assert!(
            view.warnings
                .iter()
                .any(|w| w.contains("Separate it again"))
        );

        // One undo step removes it; the history still offers it for reuse.
        session.step_history(false);
        assert_eq!(session.project.audio.stem, None);
        assert_eq!(session.project.audio.stem_origin, None);
        assert!(!session.workspace().stem_outdated);
        let again = session.separation_request(&model, &runtime).unwrap();
        assert_eq!(again.known, vec![("stems/drums.ogg".into(), origin)]);

        // A stem for another project is refused.
        let elsewhere = SeparatedStem {
            root: root.path().to_path_buf(),
            ..stem
        };
        assert!(session.apply_separation(&elsewhere).is_err());
    }

    #[test]
    fn importing_a_stem_clears_a_separated_origin() {
        let mut project = Project::new("t", "m");
        project.audio.stem_origin = Some(StemOrigin::Separated {
            model_sha256: "a".into(),
            song_sha256: "b".into(),
        });
        project.audio.stem = Some("stems/drums.ogg".into());
        project
            .apply(&Edit::SetStem {
                path: Some("stems/site.ogg".into()),
                origin: None,
            })
            .unwrap();
        assert_eq!(project.audio.stem_origin, None);
    }

    #[test]
    fn a_moved_mapset_is_reported_and_opens_at_its_new_folder_as_a_change() {
        let root = tempfile::tempdir().unwrap();
        let old = mapset(root.path(), "1 A - B", &["Hard"]);
        let mut session =
            Session::create(Arc::new(|_| {}), &old, &root.path().join("p"), None).unwrap();
        session.save().unwrap();
        let file = session.file().to_path_buf();
        drop(session);
        assert_eq!(moved_mapset(&file), None);

        let songs = root.path().join("Songs");
        std::fs::create_dir(&songs).unwrap();
        std::fs::rename(&old, songs.join("1 A - B")).unwrap();
        let moved = moved_mapset(&file).unwrap();
        assert_eq!(moved.diff_files, ["A - B (me) [Hard].osu"]);
        assert!(Session::open(Arc::new(|_| {}), &file, None).is_err());

        let new = songs.join("1 A - B").canonicalize().unwrap();
        let mut session = Session::open_relocated(Arc::new(|_| {}), &file, None, &new).unwrap();
        assert_eq!(session.project.mapset_dir, new);
        assert!(session.workspace().dirty, "the new folder is an unsaved change");
        session.save().unwrap();
        drop(session);
        assert_eq!(moved_mapset(&file), None);
        // A folder without standard or catch difficulties is refused as before.
        let empty = root.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(Session::open_relocated(Arc::new(|_| {}), &file, None, &empty).is_err());
    }

    #[test]
    fn a_mapset_without_targets_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let real = mapset(root.path(), "1 A - B", &["Hard"]);
        let empty = mapset(root.path(), "2 Empty", &[]);
        let mut session =
            Session::create(Arc::new(|_| {}), &real, &root.path().join("p"), None).unwrap();
        assert!(
            session
                .set_mapset(&empty)
                .unwrap_err()
                .contains("No readable standard or catch")
        );
        assert_eq!(session.project.mapset_dir, real);
    }
}
