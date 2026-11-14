//! The project file (`project.json`), stored outside the mapset folder (D-27).

use crate::{Channel, ChannelId, DiffProfile};
use osu_format::SampleSet;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("project json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("project format version {found} is newer than supported ({FORMAT_VERSION})")]
    TooNew { found: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Monitor {
    #[default]
    Original,
    Stems,
}

/// Monitoring audio. The mapset's `AudioFilename` is never modified (D-24).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AudioSettings {
    /// What plays under the hitsounds: the mapset's song, or the stem.
    #[serde(default)]
    pub monitor: Monitor,
    /// One stem file (e.g. the drum stem), same length and offset as the song (D-21, D-22).
    #[serde(default)]
    pub stem: Option<PathBuf>,
    /// Set only for a stem hitsounds-rs separated itself (D-73); an imported stem has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stem_origin: Option<StemOrigin>,
}

/// Where the project's stem came from (D-73), so a later run for the same song and
/// model reuses it and a changed song is noticed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StemOrigin {
    /// Separated in the app with the user's model file (D-71).
    Separated {
        /// SHA-256 of the ONNX model file, lowercase hex.
        model_sha256: String,
        /// SHA-256 of the song file it was separated from, lowercase hex.
        song_sha256: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    /// Write hitsounds straight into each difficulty, with backups (D-19).
    #[default]
    Direct,
    /// Emit a hitsound-only `.osu` per difficulty instead.
    HitsoundOnly,
}

/// What the always-playing hitnormal does at an addition-only moment (D-31, D-78, D-91).
/// It applies to preview and export alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdditionOnlyNormal {
    /// The skin hitnormal of the sample set the difficulty's greenlines give the object
    /// (`[General] SampleSet` when the greenline says 0), as an untouched object would
    /// play (D-91). `skin_default` was the old default and is read as this.
    #[default]
    #[serde(alias = "skin_default")]
    Difficulty,
    /// Skin default hitnormal of the top addition's sample set (Mapping Tools behaviour,
    /// D-31).
    AdditionsSet,
    /// The skin's `normal-hitnormal`, whatever set the additions use (D-78).
    SkinNormal,
    /// The skin's `soft-hitnormal` (D-78).
    SkinSoft,
    /// The skin's `drum-hitnormal` (D-78).
    SkinDrum,
    /// A 44-byte silent file.
    Silent,
}

impl AdditionOnlyNormal {
    pub const ALL: [Self; 6] = [
        Self::Difficulty,
        Self::AdditionsSet,
        Self::SkinSoft,
        Self::SkinNormal,
        Self::SkinDrum,
        Self::Silent,
    ];

    /// The hitnormal's own sample set when it doesn't follow the additions (D-78, D-91).
    /// `point_set` is the set the difficulty gives the object; `Auto` when there is no
    /// difficulty (the base layer), which falls back to the additions' set.
    pub fn hitnormal_set(self, point_set: SampleSet) -> Option<SampleSet> {
        match self {
            Self::Difficulty => (point_set != SampleSet::Auto).then_some(point_set),
            Self::SkinNormal => Some(SampleSet::Normal),
            Self::SkinSoft => Some(SampleSet::Soft),
            Self::SkinDrum => Some(SampleSet::Drum),
            Self::AdditionsSet | Self::Silent => None,
        }
    }
}

/// The file type of exported hitsound samples (D-69), as in Mapping Tools' Hitsound
/// Studio. A 44-byte blank is always WAV, which the ranking criteria require.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleFormat {
    /// Copy an unchanged single WAV or OGG source file as it is. A source that must be
    /// re-encoded keeps its kind: OGG stays OGG, anything else becomes 16-bit WAV.
    /// Mixed samples treat this as 16-bit WAV.
    Source,
    /// 16-bit PCM WAV.
    #[default]
    Wav16,
    /// 32-bit float WAV. osu! loads samples as 16-bit, so this only costs space.
    WavFloat,
    /// Ogg Vorbis.
    Ogg,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    #[serde(default)]
    pub write_mode: WriteMode,
    /// First custom index handed out (Mapping Tools default: 1).
    #[serde(default = "one")]
    pub first_custom_index: u32,
    /// First index for generated slidertick/sliderslide samples (Hitsound Copier default: 100).
    #[serde(default = "hundred")]
    pub body_sample_first_index: u32,
    /// Events closer than this are one moment (Mapping Tools `ZipLayersLeniency`: 15).
    #[serde(default = "fifteen")]
    pub grouping_leniency_ms: f64,
    #[serde(default)]
    pub addition_only_normal: AdditionOnlyNormal,
    /// Let a re-export add new custom indices to the saved schema (Mapping Tools
    /// "allow growth"). When off, the saved schema must already fit every hit.
    #[serde(default = "yes")]
    pub allow_growth: bool,
    /// Apply the ranking-criteria auto-fixes to rendered samples (D-12): trim leading
    /// silence, pad to 25 ms.
    #[serde(default = "yes")]
    pub auto_fix: bool,
    /// Replace hitsound sample files in the mapset that hitsounds-rs did not write. They
    /// are moved into the project's backup folder first. Off: their index numbers are
    /// avoided instead (D-38).
    #[serde(default)]
    pub take_over_samples: bool,
    /// Write a 44-byte `sliderslide` for every (set, index) a slider plays under, so
    /// custom indices don't bring back the skin's slide sound (D-39).
    #[serde(default)]
    pub mute_slider_slides: bool,
    /// Same for `slidertick`, for sliders that have ticks (D-39).
    #[serde(default)]
    pub mute_slider_ticks: bool,
    /// File type of a sample with one source (D-69).
    #[serde(default)]
    pub single_format: SampleFormat,
    /// File type of a sample that mixes several sources (D-69).
    #[serde(default)]
    pub mixed_format: SampleFormat,
    /// Remove greenlines that change nothing from written difficulties, including the
    /// mapper's own (D-80).
    #[serde(default = "yes")]
    pub remove_redundant_greenlines: bool,
}

fn one() -> u32 {
    1
}
fn hundred() -> u32 {
    100
}
fn fifteen() -> f64 {
    15.0
}
fn yes() -> bool {
    true
}
impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            write_mode: WriteMode::Direct,
            first_custom_index: 1,
            body_sample_first_index: 100,
            grouping_leniency_ms: 15.0,
            addition_only_normal: AdditionOnlyNormal::Difficulty,
            allow_growth: true,
            auto_fix: true,
            take_over_samples: false,
            mute_slider_slides: false,
            mute_slider_ticks: false,
            single_format: SampleFormat::Wav16,
            mixed_format: SampleFormat::Wav16,
            remove_redundant_greenlines: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub format_version: u32,
    pub name: String,
    /// The osu!stable mapset folder (`.../osu!/Songs/<set>`).
    pub mapset_dir: PathBuf,
    /// Format-1 projects stored the skin folder here. It is now an app-wide setting
    /// (D-40), so this is read once for migration and never written back.
    #[serde(rename = "skin_dir", default, skip_serializing)]
    pub legacy_skin_dir: Option<PathBuf>,
    #[serde(default)]
    pub audio: AudioSettings,
    /// Order = mixing priority: index 0 is the top channel and wins clashes (D-09).
    #[serde(default)]
    pub channels: Vec<Channel>,
    /// Channel used for objects no event lands on (e.g. a soft-hitnormal channel).
    #[serde(default)]
    pub fallback: Option<ChannelId>,
    /// Master volume envelope shared by every difficulty (D-44).
    #[serde(default)]
    pub volume: crate::VolumeEnvelope,
    #[serde(default)]
    pub diffs: Vec<DiffProfile>,
    #[serde(default)]
    pub export: ExportSettings,
    #[serde(default)]
    pub next_channel_id: ChannelId,
    /// The custom-index schema of the last export. Re-exports keep these numbers so
    /// sample files don't churn (findings 5, `SampleSchema`).
    #[serde(default)]
    pub schema: Vec<crate::packing::CustomIndex>,
}

impl Project {
    pub fn new(name: impl Into<String>, mapset_dir: impl Into<PathBuf>) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            name: name.into(),
            mapset_dir: mapset_dir.into(),
            legacy_skin_dir: None,
            audio: AudioSettings::default(),
            channels: Vec::new(),
            fallback: None,
            volume: crate::VolumeEnvelope::default(),
            diffs: Vec::new(),
            export: ExportSettings::default(),
            next_channel_id: 1,
            schema: Vec::new(),
        }
    }

    pub fn add_channel(&mut self, mut channel: Channel) -> ChannelId {
        channel.id = self.next_channel_id;
        self.next_channel_id += 1;
        let id = channel.id;
        self.channels.push(channel);
        id
    }

    pub fn channel(&self, id: ChannelId) -> Option<&Channel> {
        self.channels.iter().find(|c| c.id == id)
    }

    pub fn channel_mut(&mut self, id: ChannelId) -> Option<&mut Channel> {
        self.channels.iter_mut().find(|c| c.id == id)
    }

    pub fn diff(&self, file: &str) -> Option<&DiffProfile> {
        self.diffs.iter().find(|d| d.file == file)
    }

    pub fn diff_mut(&mut self, file: &str) -> Option<&mut DiffProfile> {
        self.diffs.iter_mut().find(|d| d.file == file)
    }

    /// Whether a channel is heard while monitoring: not muted, and soloed if any
    /// channel is soloed. Monitoring never affects export (D-14a).
    pub fn audible(&self, id: ChannelId) -> bool {
        let any_solo = self.channels.iter().any(|c| c.solo);
        self.channel(id).is_some_and(|c| !c.muted && (!any_solo || c.solo))
    }

    /// The volume envelope a difficulty plays with: its own override, or the base one (D-44).
    pub fn volume_for<'a>(&'a self, diff: Option<&'a DiffProfile>) -> &'a crate::VolumeEnvelope {
        diff.and_then(|d| d.volume.as_ref()).unwrap_or(&self.volume)
    }

    /// Mixing priority (0 = highest).
    pub fn priority(&self, id: ChannelId) -> Option<usize> {
        self.channels.iter().position(|c| c.id == id)
    }

    /// All base events as `(channel, time)`.
    pub fn base_events(&self) -> impl Iterator<Item = (ChannelId, crate::Ms)> + '_ {
        self.channels.iter().flat_map(|c| c.events.iter().map(move |&t| (c.id, t)))
    }

    pub fn load(path: &Path) -> Result<Self, ProjectError> {
        let text = std::fs::read_to_string(path)?;
        let mut project: Self = serde_json::from_str(&text)?;
        if project.format_version > FORMAT_VERSION {
            return Err(ProjectError::TooNew { found: project.format_version });
        }
        let last_id = project.channels.iter().map(|c| c.id).max().unwrap_or_default();
        project.next_channel_id = project.next_channel_id.max(last_id.saturating_add(1)).max(1);
        Ok(project)
    }

    /// Writes atomically (temp file + rename) so a crash never leaves a half project.
    pub fn save(&self, path: &Path) -> Result<(), ProjectError> {
        let json = serde_json::to_string_pretty(self)?;
        let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(json.as_bytes())?;
        tmp.as_file().sync_all()?;
        tmp.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use osu_format::{Hitsound, SampleSet};

    #[test]
    fn json_round_trip() {
        let mut p = Project::new("test", "C:/osu!/Songs/123 Artist - Title");
        let mut c = Channel::new(0, "Kick", SampleSet::Drum, Hitsound::Normal);
        c.add_event(1000);
        c.add_event(500);
        let id = p.add_channel(c);
        p.fallback = Some(id);
        p.diffs.push(DiffProfile::new("Artist - Title (me) [Hard].osu", "Hard"));
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.channels[0].events, vec![500, 1000]);
    }

    #[test]
    fn addition_only_normal_keeps_its_old_names() {
        // D-91: the old default becomes the new one, the difficulty's set.
        let old: ExportSettings = serde_json::from_str(r#"{"addition_only_normal":"skin_default"}"#).unwrap();
        assert_eq!(old.addition_only_normal, AdditionOnlyNormal::Difficulty);
        let missing: ExportSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.addition_only_normal, AdditionOnlyNormal::Difficulty);
        let json = serde_json::to_value(ExportSettings::default()).unwrap();
        assert_eq!(json["addition_only_normal"], "difficulty");
        let additions: ExportSettings = serde_json::from_str(r#"{"addition_only_normal":"additions_set"}"#).unwrap();
        assert_eq!(additions.addition_only_normal, AdditionOnlyNormal::AdditionsSet);
        let silent: ExportSettings = serde_json::from_str(r#"{"addition_only_normal":"silent"}"#).unwrap();
        assert_eq!(silent.addition_only_normal, AdditionOnlyNormal::Silent);
        let soft = ExportSettings { addition_only_normal: AdditionOnlyNormal::SkinSoft, ..ExportSettings::default() };
        let json = serde_json::to_value(&soft).unwrap();
        assert_eq!(json["addition_only_normal"], "skin_soft");
        assert_eq!(serde_json::from_value::<ExportSettings>(json).unwrap(), soft);
    }

    #[test]
    fn legacy_skin_dir_is_read_but_not_written() {
        let json = r#"{"format_version":1,"name":"t","mapset_dir":"m","skin_dir":"C:/skins/default"}"#;
        let p: Project = serde_json::from_str(json).unwrap();
        assert_eq!(p.legacy_skin_dir.as_deref(), Some(Path::new("C:/skins/default")));
        let out = serde_json::to_string(&p).unwrap();
        assert!(!out.contains("skin_dir"), "{out}");
    }

    #[test]
    fn stem_origin_is_optional_and_tagged() {
        let old = r#"{"format_version":1,"name":"t","mapset_dir":"m","audio":{"stem":"stems/a.ogg"}}"#;
        let mut p: Project = serde_json::from_str(old).unwrap();
        assert_eq!(p.audio.stem_origin, None);
        assert!(!serde_json::to_string(&p).unwrap().contains("stem_origin"));
        p.audio.stem_origin = Some(StemOrigin::Separated {
            model_sha256: "aa".into(),
            song_sha256: "bb".into(),
        });
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(
            json["audio"]["stem_origin"],
            serde_json::json!({"kind": "separated", "model_sha256": "aa", "song_sha256": "bb"})
        );
        assert_eq!(serde_json::from_value::<Project>(json).unwrap(), p);
    }
}
