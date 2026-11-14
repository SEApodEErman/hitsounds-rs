//! Loading sample sources and rendering merged sample files.

use hs_audio::{AudioBuffer, conform, decode_file};
use hs_core::packing::{Source, SourceMix};
use osu_format::sample::sample_file_stem;
use osu_format::{Hitsound, SampleSet};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum RenderError {
    #[error("no skin folder is set, but the default {0} sound is needed")]
    NoSkinDir(String),
    #[error("the skin folder has no {0} sample (.wav/.ogg/.mp3)")]
    MissingSkinSample(String),
    #[error("{0}")]
    Decode(String),
}

/// Extensions osu! accepts for samples, in lookup order.
pub const SAMPLE_EXTENSIONS: [&str; 3] = ["wav", "ogg", "mp3"];

/// Rendered sample files are written at this rate.
pub const RENDER_RATE: u32 = 44_100;

/// Finds `<dir>/<stem>.<wav|ogg|mp3>`.
pub fn find_sample(dir: &Path, stem: &str) -> Option<PathBuf> {
    SAMPLE_EXTENSIONS.iter().map(|ext| dir.join(format!("{stem}.{ext}"))).find(|p| p.is_file())
}

/// Decodes and caches the audio behind [`Source`]s for one project.
pub struct SampleLibrary {
    project_dir: PathBuf,
    skin_dir: Option<PathBuf>,
    cache: HashMap<Source, Result<Arc<AudioBuffer>, RenderError>>,
}

impl SampleLibrary {
    pub fn new(project_dir: impl Into<PathBuf>, skin_dir: Option<PathBuf>) -> Self {
        Self { project_dir: project_dir.into(), skin_dir, cache: HashMap::new() }
    }

    pub fn skin_dir(&self) -> Option<&Path> {
        self.skin_dir.as_deref()
    }

    /// Changing the skin folder drops every cached skin sample.
    pub fn set_skin_dir(&mut self, skin_dir: Option<PathBuf>) {
        if self.skin_dir != skin_dir {
            self.skin_dir = skin_dir;
            self.cache.retain(|s, _| !matches!(s, Source::Skin { .. } | Source::SkinSlider { .. }));
        }
    }

    /// Forgets everything, e.g. after sample files changed on disk.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// A project-relative (or absolute) sample path as a real path.
    pub fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() { path.to_path_buf() } else { self.project_dir.join(path) }
    }

    /// The skin file for a default sound, if the skin folder has one (D-30).
    pub fn skin_file(&self, set: SampleSet, sound: Hitsound) -> Result<PathBuf, RenderError> {
        let stem = sample_file_stem(set, sound, 1).unwrap_or_else(|| "auto".into());
        let dir = self.skin_dir.as_deref().ok_or_else(|| RenderError::NoSkinDir(stem.clone()))?;
        find_sample(dir, &stem).ok_or(RenderError::MissingSkinSample(stem))
    }

    /// The decoded audio of one source, at its native rate and layout.
    pub fn load(&mut self, source: &Source) -> Result<Arc<AudioBuffer>, RenderError> {
        if let Some(hit) = self.cache.get(source) {
            return hit.clone();
        }
        let loaded = match source {
            Source::Silent => Ok(AudioBuffer::silent(RENDER_RATE, 1, 0)),
            Source::Skin { set, sound } => self
                .skin_file(*set, *sound)
                .and_then(|p| decode_file(&p).map_err(|e| RenderError::Decode(e.to_string()))),
            Source::SkinSlider { set, sound } => {
                let stem = sound.stem(*set, 1).unwrap_or_default();
                let path = self.skin_dir.as_deref()
                    .ok_or_else(|| RenderError::NoSkinDir(stem.clone()))
                    .and_then(|dir| find_sample(dir, &stem).ok_or(RenderError::MissingSkinSample(stem)));
                path.and_then(|p| decode_file(&p).map_err(|e| RenderError::Decode(e.to_string())))
            }
            Source::File { path } => decode_file(&self.resolve_path(path)).map_err(|e| RenderError::Decode(e.to_string())),
        }
        .map(Arc::new);
        self.cache.insert(source.clone(), loaded.clone());
        loaded
    }

    /// Mixes the sources of one sample slot into the audio of its file: every source
    /// at its baked gain, 44.1 kHz, stereo only if a source is stereo.
    pub fn render(&mut self, mixes: &BTreeSet<SourceMix>) -> Result<AudioBuffer, RenderError> {
        let mut inputs: Vec<(Arc<AudioBuffer>, f32)> = Vec::new();
        for mix in mixes {
            if mix.source == Source::Silent {
                continue;
            }
            inputs.push((self.load(&mix.source)?, f32::from(mix.gain_pct) / 100.0));
        }
        let channels = if inputs.iter().any(|(b, _)| b.channels >= 2) { 2 } else { 1 };
        let conformed: Vec<(AudioBuffer, f32)> = inputs
            .iter()
            .filter(|(b, _)| b.frames() > 0)
            .map(|(b, g)| conform(b, RENDER_RATE, channels).map(|c| (c, *g)).map_err(|e| RenderError::Decode(e.to_string())))
            .collect::<Result<_, _>>()?;
        if conformed.is_empty() {
            return Ok(AudioBuffer::silent(RENDER_RATE, 1, 0));
        }
        let refs: Vec<(&AudioBuffer, f32)> = conformed.iter().map(|(b, g)| (b, *g)).collect();
        hs_audio::mix::mix(&refs).map_err(|e| RenderError::Decode(e.to_string()))
    }
}

/// A short human description of a slot's content, e.g. `kick.wav + skin soft-hitnormal 50%`.
pub fn describe(mixes: &BTreeSet<SourceMix>) -> String {
    let parts: Vec<String> = mixes
        .iter()
        .map(|m| {
            let name = match &m.source {
                Source::Silent => "silence".to_owned(),
                Source::Skin { set, sound } => {
                    format!("skin {}", sample_file_stem(*set, *sound, 1).unwrap_or_default())
                }
                Source::SkinSlider { set, sound } => format!("skin {}", sound.stem(*set, 1).unwrap_or_default()),
                Source::File { path } => path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()),
            };
            if m.gain_pct == 100 { name } else { format!("{name} {}%", m.gain_pct) }
        })
        .collect();
    parts.join(" + ")
}
