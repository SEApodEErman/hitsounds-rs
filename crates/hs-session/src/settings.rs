//! App-wide settings (`settings.json` in the OS config dir): the skin sample folder
//! (D-40), the recent-projects list (D-41), the sample library folders (D-45) and the
//! drums model for separation (D-71), the listening levels (D-77), and the osu!
//! Songs folder and projects folder (D-82), whether to check for updates (D-84), and
//! whether slowed playback keeps the song's pitch (D-90). Nothing here belongs to a
//! project.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_RECENT: usize = 12;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecentProject {
    /// The `project.json` path.
    pub file: PathBuf,
    pub name: String,
    pub mapset_dir: PathBuf,
    /// Unix time in milliseconds.
    pub opened_at: u64,
}

/// Listening levels, as osu!'s master, music and effect volume (D-77). They only
/// change what you hear in the app and never touch export (like D-14a). The volumes
/// are saved; the two mutes last for the session, so the app never opens silent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Levels {
    /// 0..=1, scales everything.
    pub master: f32,
    /// 0..=1, the song or drum stem.
    pub music: f32,
    /// 0..=1, hitsounds and sample auditions.
    pub effects: f32,
    #[serde(skip)]
    pub music_muted: bool,
    /// Mute all channels at once without touching their own mute and solo.
    #[serde(skip)]
    pub hits_muted: bool,
}

impl Default for Levels {
    /// Music at 60 % under full-scale hits, which is how preview always sounded.
    fn default() -> Self {
        Self {
            master: 1.0,
            music: 0.6,
            effects: 1.0,
            music_muted: false,
            hits_muted: false,
        }
    }
}

impl Levels {
    /// Linear gains for the song, scheduled hits and auditions.
    pub fn gains(&self) -> (f32, f32, f32) {
        let level = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 };
        let master = level(self.master);
        let effects = master * level(self.effects);
        let music = if self.music_muted { 0.0 } else { master * level(self.music) };
        let hits = if self.hits_muted { 0.0 } else { effects };
        (music, hits, effects)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    /// Skin folder that supplies default samples for preview and export mixing (D-30, D-40).
    #[serde(default)]
    pub skin_dir: Option<PathBuf>,
    /// Most recent first.
    #[serde(default)]
    pub recent: Vec<RecentProject>,
    /// Folders browsed by the sample library (D-45).
    #[serde(default)]
    pub library: Vec<PathBuf>,
    /// None follows the Windows animation preference (D-54, D-55).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce_motion: Option<bool>,
    /// Interface scale on top of the Windows display scale (D-68). None = 100 %.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_scale: Option<f32>,
    /// The user's ONNX export of the HT-Demucs FT drums model (D-71).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separation_model: Option<PathBuf>,
    /// Listening levels (D-77).
    #[serde(default)]
    pub levels: Levels,
    /// The osu! Songs folder the mapset picker lists (D-82). None = the one found in
    /// the standard osu! install, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub songs_dir: Option<PathBuf>,
    /// Where new projects are created without asking (D-82). None = ask each time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projects_dir: Option<PathBuf>,
    /// Check GitHub Releases for a newer version at startup (D-84). None = on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_checks: Option<bool>,
    /// Keep the song's pitch below 1x speed (D-90). None = on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_pitch: Option<bool>,
}

impl AppSettings {
    /// D-84: update checks are on unless turned off.
    pub fn checks_for_updates(&self) -> bool {
        self.update_checks.unwrap_or(true)
    }

    /// D-90: slowed playback keeps the pitch unless turned off.
    pub fn keeps_pitch(&self) -> bool {
        self.keep_pitch.unwrap_or(true)
    }
}

/// What the UI receives: the settings plus whether each recent project still exists.
#[derive(Serialize)]
pub struct SettingsView<'a> {
    skin_dir: Option<&'a Path>,
    recent: Vec<RecentView<'a>>,
    library: &'a [PathBuf],
}

#[derive(Serialize)]
struct RecentView<'a> {
    #[serde(flatten)]
    project: &'a RecentProject,
    exists: bool,
}

pub struct Settings {
    path: Option<PathBuf>,
    pub value: AppSettings,
    /// Set when the settings file could not be read or written.
    pub warning: Option<String>,
}

impl Settings {
    pub fn load(path: Option<PathBuf>) -> Self {
        let mut warning = None;
        let contents = path.as_deref().map(std::fs::read_to_string);
        let value = match contents {
            Some(Ok(text)) => serde_json::from_str(&text).unwrap_or_else(|e| {
                warning = Some(format!("App settings were unreadable and have been reset: {e}"));
                AppSettings::default()
            }),
            Some(Err(e)) if e.kind() != std::io::ErrorKind::NotFound => {
                warning = Some(format!("App settings could not be read: {e}"));
                AppSettings::default()
            }
            _ => AppSettings::default(),
        };
        Self { path, value, warning }
    }

    pub fn view(&self) -> SettingsView<'_> {
        SettingsView {
            skin_dir: self.value.skin_dir.as_deref(),
            recent: self
                .value
                .recent
                .iter()
                .map(|project| RecentView { project, exists: project.file.is_file() })
                .collect(),
            library: &self.value.library,
        }
    }

    pub fn set_skin_dir(&mut self, dir: Option<PathBuf>) -> Result<(), String> {
        self.value.skin_dir = dir.as_deref().map(plain);
        self.save()
    }

    pub fn set_separation_model(&mut self, model: Option<PathBuf>) -> Result<(), String> {
        self.value.separation_model = model.as_deref().map(plain);
        self.save()
    }

    pub fn set_songs_dir(&mut self, dir: Option<PathBuf>) -> Result<(), String> {
        self.value.songs_dir = dir.as_deref().map(plain);
        self.save()
    }

    pub fn set_projects_dir(&mut self, dir: Option<PathBuf>) -> Result<(), String> {
        self.value.projects_dir = dir.as_deref().map(plain);
        self.save()
    }

    /// `true` is the default, so it is stored as `None`.
    pub fn set_update_checks(&mut self, on: bool) -> Result<(), String> {
        self.value.update_checks = (!on).then_some(false);
        self.save()
    }

    pub fn set_keep_pitch(&mut self, on: bool) -> Result<(), String> {
        self.value.keep_pitch = (!on).then_some(false);
        self.save()
    }

    pub fn set_reduce_motion(&mut self, preference: Option<bool>) -> Result<(), String> {
        self.value.reduce_motion = preference;
        self.save()
    }

    /// `None` or 1.0 is the default size. Values are clamped to 50-300 %.
    pub fn set_ui_scale(&mut self, scale: Option<f32>) -> Result<(), String> {
        self.value.ui_scale = scale
            .filter(|s| s.is_finite())
            .map(|s| s.clamp(0.5, 3.0))
            .filter(|s| (s - 1.0).abs() > f32::EPSILON);
        self.save()
    }

    /// Saves the volumes; the mutes are kept in memory only (D-77).
    pub fn set_levels(&mut self, levels: Levels) -> Result<(), String> {
        let saved = self.value.levels;
        self.value.levels = levels;
        if (saved.master, saved.music, saved.effects) == (levels.master, levels.music, levels.effects) {
            return Ok(());
        }
        self.save()
    }

    /// Moves `project` to the top of the recent list.
    pub fn touch_recent(&mut self, file: &Path, name: &str, mapset_dir: &Path) {
        let (file, mapset_dir) = (plain(file), plain(mapset_dir));
        touch(&mut self.value.recent, &file, name, &mapset_dir, now_ms());
        self.save_or_warn();
    }

    /// D-40: adopt a format-1 skin path if this machine has no skin setting yet.
    pub fn remember_project(
        &mut self,
        file: &Path,
        project: &hs_core::Project,
        skin: Option<&Path>,
    ) {
        if self.value.skin_dir.is_none() {
            self.value.skin_dir = skin.map(plain);
        }
        self.touch_recent(file, &project.name, &project.mapset_dir);
    }

    pub fn add_library(&mut self, dir: &Path) -> Result<(), String> {
        let dir = plain(dir);
        if !self.value.library.contains(&dir) {
            self.value.library.push(dir);
        }
        self.save()
    }

    pub fn remove_library(&mut self, dir: &Path) -> Result<(), String> {
        let dir = plain(dir);
        self.value.library.retain(|d| *d != dir);
        self.save()
    }

    pub fn forget_recent(&mut self, file: &Path) -> Result<(), String> {
        let file = plain(file);
        self.value.recent.retain(|r| plain(&r.file) != file);
        self.save()
    }

    /// Recent-list updates are a convenience, so a failed write is only a warning.
    fn save_or_warn(&mut self) {
        self.warning = self.save().err();
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Err("No app config folder is available, so settings can't be saved.".into());
        };
        let write = || -> std::io::Result<()> {
            let dir = path.parent().unwrap_or(Path::new("."));
            std::fs::create_dir_all(dir)?;
            let json = serde_json::to_string_pretty(&self.value).map_err(std::io::Error::other)?;
            let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
            tmp.write_all(json.as_bytes())?;
            tmp.as_file().sync_all()?;
            tmp.persist(path).map_err(|e| e.error)?;
            Ok(())
        };
        write().map_err(|e| format!("App settings could not be saved: {e}"))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Drops the Windows verbatim prefix that `canonicalize` adds (`\\?\C:\...`), so one
/// project never appears twice under two spellings of its path.
fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

fn touch(recent: &mut Vec<RecentProject>, file: &Path, name: &str, mapset_dir: &Path, at: u64) {
    recent.retain(|r| plain(&r.file) != file);
    recent.insert(
        0,
        RecentProject {
            file: file.to_path_buf(),
            name: name.to_owned(),
            mapset_dir: mapset_dir.to_path_buf(),
            opened_at: at,
        },
    );
    recent.truncate(MAX_RECENT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_moves_to_front_and_caps_length() {
        let mut recent = Vec::new();
        for i in 0..20 {
            touch(&mut recent, Path::new(&format!("p{i}/project.json")), "n", Path::new("m"), i);
        }
        assert_eq!(recent.len(), MAX_RECENT);
        assert_eq!(recent[0].file, Path::new("p19/project.json"));
        touch(&mut recent, Path::new("p15/project.json"), "renamed", Path::new("m"), 99);
        assert_eq!(recent.len(), MAX_RECENT);
        assert_eq!(recent[0].name, "renamed");
        assert_eq!(recent.iter().filter(|r| r.file == Path::new("p15/project.json")).count(), 1);
    }

    #[test]
    fn verbatim_paths_dedupe_with_plain_ones() {
        assert_eq!(plain(Path::new(r"\\?\C:\a\project.json")), Path::new(r"C:\a\project.json"));
        assert_eq!(plain(Path::new(r"\\?\UNC\srv\share\p.json")), Path::new(r"\\srv\share\p.json"));
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings::load(Some(dir.path().join("settings.json")));
        settings.touch_recent(Path::new(r"C:\a\project.json"), "A", Path::new("m"));
        settings.touch_recent(Path::new(r"\\?\C:\a\project.json"), "A", Path::new(r"\\?\C:\m"));
        assert_eq!(settings.value.recent.len(), 1);
        assert_eq!(settings.value.recent[0].mapset_dir, Path::new(r"C:\m"));
        settings.forget_recent(Path::new(r"\\?\C:\a\project.json")).unwrap();
        assert!(settings.value.recent.is_empty());
    }

    #[test]
    fn settings_round_trip_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        let mut settings = Settings::load(Some(path.clone()));
        assert!(settings.warning.is_none());
        settings.set_skin_dir(Some("C:/skins/default".into())).unwrap();
        settings.touch_recent(Path::new("a/project.json"), "A", Path::new("songs/a"));
        settings.set_ui_scale(Some(1.25)).unwrap();
        settings
            .set_separation_model(Some(r"\\?\C:\models\drums.onnx".into()))
            .unwrap();
        assert_eq!(
            settings.value.separation_model.as_deref(),
            Some(Path::new(r"C:\models\drums.onnx"))
        );
        let back = Settings::load(Some(path));
        assert_eq!(back.value, settings.value);
        assert_eq!(back.value.ui_scale, Some(1.25));
    }

    #[test]
    fn levels_keep_volumes_but_not_mutes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut settings = Settings::load(Some(path.clone()));
        let levels = Levels { master: 0.5, music: 0.25, effects: 0.8, music_muted: true, hits_muted: true };
        settings.set_levels(levels).unwrap();
        assert_eq!(settings.value.levels, levels);
        let back = Settings::load(Some(path)).value.levels;
        assert_eq!((back.master, back.music, back.effects), (0.5, 0.25, 0.8));
        assert!(!back.music_muted && !back.hits_muted, "the app never opens muted");
    }

    #[test]
    fn level_gains_multiply_by_master_and_hit_mute_keeps_auditions() {
        let levels = Levels { master: 0.5, music: 0.5, effects: 0.8, ..Levels::default() };
        assert_eq!(levels.gains(), (0.25, 0.4, 0.4));
        let muted = Levels { hits_muted: true, music_muted: true, ..levels };
        assert_eq!(muted.gains(), (0.0, 0.0, 0.4));
        let old: AppSettings = serde_json::from_str(r#"{"recent":[]}"#).unwrap();
        assert_eq!(old.levels, Levels::default());
    }

    #[test]
    fn default_ui_scale_is_not_stored() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings::load(Some(dir.path().join("settings.json")));
        settings.set_ui_scale(Some(1.0)).unwrap();
        assert_eq!(settings.value.ui_scale, None);
        settings.set_ui_scale(Some(9.0)).unwrap();
        assert_eq!(settings.value.ui_scale, Some(3.0));
    }

    #[test]
    fn corrupt_settings_reset_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        let settings = Settings::load(Some(path));
        assert!(settings.warning.is_some());
        assert_eq!(settings.value, AppSettings::default());
    }
}
