//! Read-only mapset discovery and parsed views for the editor.

use hs_core::{DiffProfile, DiffTimeline, Project};
use osu_format::{HitObject, OsuDocument, SampleSet, TimingPoint};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

pub struct Difficulty {
    pub label: String,
    pub title: Option<String>,
    pub audio: Option<PathBuf>,
    pub timeline: DiffTimeline,
    pub objects: Vec<HitObject>,
    pub points: Vec<TimingPoint>,
    pub general_set: SampleSet,
}

impl Difficulty {
    fn read(path: &Path) -> Result<Option<Self>, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let doc = OsuDocument::parse(&text);
        if doc.format_version().is_none() {
            return Err("Missing osu! file format header.".into());
        }
        if !matches!(doc.value("General", "Mode"), None | Some("0") | Some("2")) {
            return Ok(None);
        }
        let timeline = DiffTimeline::from_document(&doc).map_err(|e| e.to_string())?;
        Ok(Some(Self {
            label: doc
                .value("Metadata", "Version")
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|n| n.to_str())
                        .unwrap_or("Difficulty")
                })
                .to_owned(),
            title: doc
                .value("Metadata", "Title")
                .filter(|v| !v.is_empty())
                .map(|title| {
                    doc.value("Metadata", "Artist")
                        .filter(|v| !v.is_empty())
                        .map_or_else(|| title.to_owned(), |artist| format!("{artist} - {title}"))
                }),
            audio: doc.value("General", "AudioFilename").and_then(|name| {
                let relative = Path::new(name.trim_matches('"'));
                if !relative
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)))
                {
                    return None;
                }
                let dir = path.parent()?;
                let audio = dir.join(relative).canonicalize().ok()?;
                (audio.is_file() && audio.starts_with(dir)).then_some(audio)
            }),
            timeline,
            objects: doc.hit_objects().map_err(|e| e.to_string())?,
            points: doc.timing_points().map_err(|e| e.to_string())?,
            general_set: hs_core::import::general_sample_set(&doc),
        }))
    }
}

pub struct Mapset {
    pub diffs: BTreeMap<String, Difficulty>,
    pub warnings: Vec<String>,
}

pub fn is_osu(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("osu"))
}

pub fn is_filename(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(part)) if part == name)
        && components.next().is_none()
}

impl Mapset {
    pub fn read(dir: &Path) -> Result<Self, String> {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("Cannot open the mapset folder {}: {e}", dir.display()))?;
        let mut out = Self {
            diffs: BTreeMap::new(),
            warnings: Vec::new(),
        };
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if !is_osu(&path) || !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                continue;
            }
            let Ok(file) = entry.file_name().into_string() else {
                continue;
            };
            // D-95: a base hitsound difficulty is export output, not one to hitsound.
            if hs_export::export::is_base_file(&file) {
                continue;
            }
            match Difficulty::read(&path) {
                Ok(Some(diff)) => {
                    out.diffs.insert(file, diff);
                }
                Ok(None) => out.warnings.push(format!(
                    "Skipped {file} (only standard and catch are supported)."
                )),
                Err(e) => out.warnings.push(format!("Could not read {file}: {e}")),
            }
        }
        if let Some(reference) = out.reference() {
            let warnings: Vec<_> = out
                .diffs
                .values()
                .skip(1)
                .filter_map(|diff| {
                    if diff.timeline.redlines != reference.timeline.redlines {
                        Some(format!(
                            "{} has different timing. The base layer uses {}'s redlines.",
                            diff.label, reference.label
                        ))
                    } else if diff.audio != reference.audio {
                        Some(format!(
                            "{} uses a different audio file. Monitoring uses {}'s song.",
                            diff.label, reference.label
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            out.warnings.extend(warnings);
        }
        out.warnings.sort();
        Ok(out)
    }

    pub fn reference(&self) -> Option<&Difficulty> {
        self.diffs.values().next()
    }

    /// Add discoveries without throwing away the settings of temporarily missing files.
    pub fn update_profiles(&self, project: &mut Project) {
        for (file, diff) in &self.diffs {
            if let Some(profile) = project.diff_mut(file) {
                profile.label.clone_from(&diff.label);
            } else {
                project.diffs.push(DiffProfile::new(file, &diff.label));
            }
        }
    }
}
