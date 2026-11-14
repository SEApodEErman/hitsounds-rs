//! Importing a mania hitsound layer difficulty from disk (D-03, D-66).
//!
//! [`read`] parses the `.osu` and resolves its samples against the folder it sits in.
//! [`find_mapset`] picks the mapset the hitsounds are for. [`Session::import_hitsound_diff`]
//! copies the samples into the project and adds the layers as channels in one undo step.

use crate::mapset::Mapset;
use crate::session::Session;
use hs_core::import::{ImportedDiff, LayerSample, import_hitsound_diff, layer_names};
use hs_core::{ChannelId, ChannelProps, Edit, SampleSource};
use osu_format::OsuDocument;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A parsed hitsound layer difficulty and the folder its samples come from.
pub struct HitsoundDiff {
    /// The folder the `.osu` is in. Sample files are read from here.
    pub dir: PathBuf,
    /// `[Metadata] Version`, or the file name.
    pub label: String,
    pub imported: ImportedDiff,
}

/// What an import added, for the notice the UI shows.
#[derive(Debug, Default, PartialEq)]
pub struct ImportSummary {
    pub channels: usize,
    pub hits: usize,
    pub volume_points: usize,
    pub samples: usize,
    /// Sample files that could not be copied or decoded. Their channels play the skin.
    pub failed: Vec<String>,
}

pub fn read(path: &Path) -> Result<HitsoundDiff, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc = OsuDocument::parse(&text);
    if doc.format_version().is_none() {
        return Err("This file has no osu! file format header.".into());
    }
    let dir = path
        .parent()
        .ok_or("The difficulty has no parent folder.")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    // Windows file names are case-insensitive, and so is osu!'s lookup there.
    let mut files: HashMap<String, String> = HashMap::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().is_ok_and(|t| t.is_file())
            && let Ok(name) = entry.file_name().into_string()
        {
            files.insert(name.to_lowercase(), name);
        }
    }
    let imported = import_hitsound_diff(&doc, |name| files.get(&name.to_lowercase()).cloned())
        .map_err(|e| e.to_string())?;
    let label = doc
        .value("Metadata", "Version")
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .or_else(|| path.file_stem().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "hitsound diff".into());
    Ok(HitsoundDiff {
        dir,
        label,
        imported,
    })
}

fn has_targets(dir: &Path) -> bool {
    Mapset::read(dir).is_ok_and(|m| m.reference().is_some())
}

/// The leading beatmap set number of a Songs folder name (`2629496 Artist - Title`).
fn set_number(dir: &Path) -> Option<&str> {
    let name = dir.file_name()?.to_str()?;
    let digits = name.split(' ').next()?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

/// The mapset a hitsound diff is for: its own folder when that has standard or catch
/// difficulties, otherwise the one sibling folder with the same set number that does
/// (a copy made for hitsounding, such as `123 Artist - Title - hs`). `None` means the
/// user has to choose.
pub fn find_mapset(dir: &Path) -> Option<PathBuf> {
    if has_targets(dir) {
        return Some(dir.to_path_buf());
    }
    let number = set_number(dir)?;
    let mut found = std::fs::read_dir(dir.parent()?)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path != dir && path.is_dir() && set_number(path) == Some(number) && has_targets(path)
        });
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

impl Session {
    /// Adds every layer of `diff` as a channel, in column order, and replaces the base
    /// volume envelope with the diff's greenline volume when it has any. Samples are
    /// copied into the project's `samples/` (D-45 reuses identical copies). One undo step.
    pub fn import_hitsound_diff(
        &mut self,
        diff: &HitsoundDiff,
        color: impl Fn(ChannelId) -> String,
    ) -> Result<ImportSummary, String> {
        let mut summary = ImportSummary::default();
        let mut copies: HashMap<&str, Option<PathBuf>> = HashMap::new();
        for layer in &diff.imported.layers {
            if let LayerSample::File(name) = &layer.sample
                && !copies.contains_key(name.as_str())
            {
                let copy = match self.import_audio(&diff.dir.join(name), false) {
                    Ok(path) => {
                        summary.samples += 1;
                        Some(path)
                    }
                    Err(_) => {
                        summary.failed.push(name.clone());
                        None
                    }
                };
                copies.insert(name, copy);
            }
        }
        let first = self.project.next_channel_id;
        let mut edits: Vec<Edit> = diff
            .imported
            .layers
            .iter()
            .zip(layer_names(&diff.imported.layers))
            .zip(first..)
            .map(|((layer, name), id)| {
                let sample = match &layer.sample {
                    LayerSample::File(file) => copies
                        .get(file.as_str())
                        .cloned()
                        .flatten()
                        .map_or(SampleSource::SkinDefault, |path| SampleSource::File {
                            path,
                        }),
                    LayerSample::Skin => SampleSource::SkinDefault,
                };
                summary.hits += layer.events.len();
                Edit::AddChannel {
                    props: ChannelProps {
                        name,
                        color: color(id),
                        sample_set: layer.sample_set,
                        hitsound: layer.hitsound,
                        sample,
                        volume: layer.volume,
                    },
                    index: None,
                    events: layer.events.clone(),
                }
            })
            .collect();
        summary.channels = edits.len();
        if !diff.imported.volume.is_empty() {
            summary.volume_points = diff.imported.volume.points.len();
            edits.push(Edit::SetVolume {
                diff: None,
                points: Some(diff.imported.volume.points.clone()),
            });
        }
        self.edit(&Edit::Batch { edits })?;
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const STANDARD: &str = "osu file format v14\n\n[General]\nAudioFilename: audio.mp3\nMode: 0\n\n[Metadata]\nTitle:Song\nArtist:Artist\nVersion:Hard\n\n[TimingPoints]\n0,500,4,2,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
    const LAYERS: &str = "osu file format v14\n\n[General]\nSampleSet: Soft\nMode: 3\n\n[Metadata]\nVersion:hs layers\n\n[TimingPoints]\n0,500,4,2,0,100,1,0\n1500,-100,4,2,0,60,0,0\n\n[HitObjects]\n256,192,1000,5,0,1:1:0:100:kick\n295,192,1500,1,8,2:2:0:100:snare\n256,192,2000,1,0,1:1:0:100:kick\n497,192,2000,1,2,0:0:0:0:\n";

    fn write(path: &Path, text: &[u8]) {
        std::fs::write(path, text).unwrap();
    }

    /// Songs/1 Artist - Song (real mapset) and Songs/1 Artist - Song - hs (layer copy).
    fn songs() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("Songs").join("1 Artist - Song");
        let copy = root.path().join("Songs").join("1 Artist - Song - hs");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&copy).unwrap();
        write(
            &real.join("Artist - Song (me) [Hard].osu"),
            STANDARD.as_bytes(),
        );
        write(
            &copy.join("Artist - Song (me) [hs layers].osu"),
            LAYERS.as_bytes(),
        );
        let tone = hs_export::wav::encode_pcm16(&hs_audio::AudioBuffer {
            sample_rate: 44_100,
            channels: 1,
            samples: vec![0.25; 2205],
        });
        write(&copy.join("kick.wav"), &tone);
        write(&copy.join("Snare.wav"), &hs_export::wav::blank_wav());
        (
            root,
            real.canonicalize().unwrap(),
            copy.canonicalize().unwrap(),
        )
    }

    #[test]
    fn a_layer_copy_finds_its_sibling_mapset() {
        let (_root, real, copy) = songs();
        assert_eq!(find_mapset(&copy), Some(real.clone()));
        assert_eq!(find_mapset(&real), Some(real));
    }

    #[test]
    fn two_candidate_mapsets_leave_the_choice_to_the_user() {
        let (root, _real, copy) = songs();
        let other = root.path().join("Songs").join("1 Artist - Song (old)");
        std::fs::create_dir_all(&other).unwrap();
        write(
            &other.join("Artist - Song (me) [Easy].osu"),
            STANDARD.as_bytes(),
        );
        assert_eq!(find_mapset(&copy), None);
    }

    #[test]
    fn import_adds_channels_volume_and_copies_samples() {
        let (root, real, copy) = songs();
        let diff = read(&copy.join("Artist - Song (me) [hs layers].osu")).unwrap();
        assert_eq!(diff.label, "hs layers");
        let mut session =
            Session::create(Arc::new(|_| {}), &real, &root.path().join("p"), None).unwrap();
        let summary = session
            .import_hitsound_diff(&diff, |_| "#123456".into())
            .unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                channels: 3,
                hits: 4,
                volume_points: 2,
                samples: 2,
                failed: vec![]
            }
        );
        let channels: Vec<_> = session
            .project
            .channels
            .iter()
            .map(|c| (c.name.as_str(), c.sample.clone(), c.events.clone()))
            .collect();
        assert_eq!(
            channels,
            vec![
                (
                    "kick",
                    SampleSource::File {
                        path: PathBuf::from("samples").join("kick.wav")
                    },
                    vec![1000, 2000]
                ),
                // The snare's name case comes from the folder, and the blank copies as is.
                (
                    "Snare",
                    SampleSource::File {
                        path: PathBuf::from("samples").join("Snare.wav")
                    },
                    vec![1500]
                ),
                (
                    "soft-hitwhistle (skin)",
                    SampleSource::SkinDefault,
                    vec![2000]
                ),
            ]
        );
        assert!(
            session
                .file()
                .parent()
                .unwrap()
                .join("samples")
                .join("kick.wav")
                .is_file()
        );
        assert_eq!(session.project.volume.scale(100, 1499), 100);
        assert_eq!(session.project.volume.scale(100, 1500), 60);
        // One undo step takes the whole import back.
        session.step_history(false);
        assert!(session.project.channels.is_empty());
        assert!(session.project.volume.is_empty());
    }
}
