//! One-way import of a mania "hitsound layers" difficulty (D-03).
//!
//! This reads the layout Mapping Tools' Hitsound Preview Helper writes: a mania map in
//! which every note is one sample, described by its hitsound bits, its sample set and
//! the `filename` of its `hitSample` (findings 5). Notes that play the same sound
//! become one layer, which the app turns into a channel. Holds are read as their head.
//!
//! Each note is resolved the way osu!stable plays it in mania (findings 10): a `filename` that exists plays alone, otherwise the hitsound bits
//! play from the custom index, falling back to the skin. Greenline volume becomes the
//! master volume envelope (D-44). A note's own volume becomes its layer's channel volume.
//!
//! The module stays pure: the caller supplies `exists`, which says whether a file with a
//! given name is in the mapset folder and returns its name as found there.

use crate::volume::{Curve, VolumeEnvelope, VolumePoint};
use crate::Ms;
use osu_format::timing::{HITSOUND_LOOKAHEAD_MS, active_point};
use osu_format::{Hitsound, OsuDocument, SampleSet, TimingPoint};
use std::collections::BTreeMap;

/// Where a layer's sound comes from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayerSample {
    /// The skin's sound for the layer's set and hitsound (no mapset file plays).
    Skin,
    /// A file in the mapset folder, named as it was found there.
    File(String),
}

/// The notes that play one sound at one volume.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedLayer {
    /// The osu! slot the layer is exported through. Never `Auto`.
    pub sample_set: SampleSet,
    pub hitsound: Hitsound,
    pub sample: LayerSample,
    /// Linear 0..=1 from the note's own volume. A note volume of 0 ("use the timing
    /// point") imports as 1.0, so the greenline envelope alone sets the hit volume.
    pub volume: f32,
    /// Leftmost column the layer was seen in, to keep the mania lane order.
    pub column_x: i32,
    pub events: Vec<Ms>,
}

/// Everything an import adds to a project.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedDiff {
    /// Ordered by column, then by first note.
    pub layers: Vec<ImportedLayer>,
    /// Greenline and redline volume as step points. Empty when every point is at 100 %.
    pub volume: VolumeEnvelope,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("This is not an osu!mania difficulty. Import reads mania hitsound layer difficulties.")]
    NotMania,
    #[error("This difficulty has no notes to import.")]
    Empty,
    #[error(transparent)]
    Parse(#[from] osu_format::ParseError),
}

/// `[General] SampleSet` as a set. `None`/unknown maps to Normal (**unverified** in
/// stable; v14 timing points always carry a set, so it rarely decides anything).
pub fn general_sample_set(doc: &OsuDocument) -> SampleSet {
    match doc.value("General", "SampleSet").map(str::to_ascii_lowercase).as_deref() {
        Some("soft") => SampleSet::Soft,
        Some("drum") => SampleSet::Drum,
        _ => SampleSet::Normal,
    }
}

pub fn is_mania(doc: &OsuDocument) -> bool {
    doc.value("General", "Mode").map(str::trim) == Some("3")
}

/// The mapset file a `filename` field loads, as stable looks it
/// up: the exact name when it has an extension, then the name without its extension
/// as `.wav`, `.ogg` and `.mp3`.
pub fn find_named_sample(name: &str, exists: &impl Fn(&str) -> Option<String>) -> Option<String> {
    // Stable treats the name as extensionless unless its first dot comes after a character.
    let dot = name.find('.').filter(|&first| first > 0).and(name.rfind('.'));
    if dot.is_some()
        && let Some(found) = exists(name)
    {
        return Some(found);
    }
    let stem = dot.map_or(name, |i| &name[..i]);
    ["wav", "ogg", "mp3"].into_iter().find_map(|ext| exists(&format!("{stem}.{ext}")))
}

/// The mapset file a custom index plays for one slot, or `None` when the skin plays.
/// Index 0 never reads the mapset (findings 2).
fn find_index_sample(set: SampleSet, sound: Hitsound, index: u32, exists: &impl Fn(&str) -> Option<String>) -> Option<String> {
    if index == 0 {
        return None;
    }
    let stem = osu_format::sample::sample_file_stem(set, sound, index)?;
    ["wav", "ogg"].into_iter().find_map(|ext| exists(&format!("{stem}.{ext}")))
}

/// Reads the layers and volume changes of a mania hitsound layer difficulty.
pub fn import_hitsound_diff(
    doc: &OsuDocument,
    exists: impl Fn(&str) -> Option<String>,
) -> Result<ImportedDiff, ImportError> {
    if !is_mania(doc) {
        return Err(ImportError::NotMania);
    }
    let points = doc.timing_points()?;
    let objects = doc.hit_objects()?;
    if objects.is_empty() {
        return Err(ImportError::Empty);
    }
    let general = general_sample_set(doc);

    type Key = (SampleSet, Hitsound, LayerSample, u32);
    let mut layers: BTreeMap<Key, ImportedLayer> = BTreeMap::new();
    for o in &objects {
        let point = active_point(&points, f64::from(o.time) + HITSOUND_LOOKAHEAD_MS);
        let point_set = point.map(|p| p.sample_set).filter(|s| *s != SampleSet::Auto).unwrap_or(general);
        let normal = if o.hit_sample.normal_set == SampleSet::Auto { point_set } else { o.hit_sample.normal_set };
        let addition = if o.hit_sample.addition_set == SampleSet::Auto { normal } else { o.hit_sample.addition_set };
        let index = if o.hit_sample.index != 0 { o.hit_sample.index } else { point.map_or(0, |p| p.sample_index) };
        let additions: Vec<Hitsound> = o.hitsound.additions().collect();
        // Mania has no LayeredHitSounds: the hitnormal plays only for its own bit or
        // when no bit is set (findings 10).
        let plays_normal = o.hitsound.has(Hitsound::Normal) || additions.is_empty();

        let named = Some(o.hit_sample.filename.as_str())
            .filter(|name| !name.is_empty())
            .and_then(|name| find_named_sample(name, &exists));
        let sounds: Vec<(SampleSet, Hitsound, LayerSample)> = if let Some(file) = named {
            // A filename that loads plays alone (findings 10). Its slot
            // comes from the bits, which the Preview Helper writes per lane.
            let (set, sound) = if plays_normal { (normal, Hitsound::Normal) } else { (addition, additions[0]) };
            vec![(set, sound, LayerSample::File(file))]
        } else {
            let mut sounds = Vec::new();
            if plays_normal {
                sounds.push((normal, Hitsound::Normal));
            }
            sounds.extend(additions.into_iter().map(|h| (addition, h)));
            sounds
                .into_iter()
                .map(|(set, sound)| {
                    let sample = find_index_sample(set, sound, index, &exists).map_or(LayerSample::Skin, LayerSample::File);
                    (set, sound, sample)
                })
                .collect()
        };
        let volume = match o.hit_sample.volume {
            0 => 100,
            v => v.min(100),
        };
        for (set, sound, sample) in sounds {
            let layer = layers.entry((set, sound, sample.clone(), volume)).or_insert_with(|| ImportedLayer {
                sample_set: set,
                hitsound: sound,
                sample,
                volume: volume as f32 / 100.0,
                column_x: o.x,
                events: Vec::new(),
            });
            layer.column_x = layer.column_x.min(o.x);
            layer.events.push(o.time);
        }
    }
    let mut out: Vec<ImportedLayer> = layers.into_values().collect();
    for layer in &mut out {
        layer.events.sort_unstable();
        layer.events.dedup();
    }
    out.sort_by_key(|l| (l.column_x, l.events.first().copied().unwrap_or(0)));

    let mut times: Vec<Ms> = objects.iter().map(|o| o.time).collect();
    times.sort_unstable();
    Ok(ImportedDiff { layers: out, volume: volume_envelope(&points, &times) })
}

/// Turns timing point volume into step points of the master envelope (D-44).
///
/// osu! reads the greenline at `note + 5 ms`, while the envelope applies from a point's
/// own time. A point is therefore moved back to the first note that already hears it,
/// so every note gets the volume it had in the mania map.
fn volume_envelope(points: &[TimingPoint], notes: &[Ms]) -> VolumeEnvelope {
    // At one instant the last point in file order wins (`active_point`).
    let mut by_time: Vec<(f64, u32)> = Vec::new();
    for p in points {
        match by_time.last_mut() {
            Some(last) if last.0 == p.time => last.1 = p.volume,
            _ => by_time.push((p.time, p.volume)),
        }
    }
    let mut changes: Vec<(f64, u32)> = Vec::new();
    for (time, volume) in by_time {
        if changes.last().is_none_or(|&(_, v)| v != volume) {
            changes.push((time, volume));
        }
    }
    if changes.iter().all(|&(_, v)| v == 100) {
        return VolumeEnvelope::default();
    }
    let mut out: Vec<VolumePoint> = Vec::with_capacity(changes.len());
    for (time, volume) in changes {
        let mut at = time.floor() as Ms;
        let first_hearing = notes.partition_point(|&n| f64::from(n) + HITSOUND_LOOKAHEAD_MS < time);
        if let Some(&note) = notes.get(first_hearing)
            && f64::from(note) < time
        {
            at = note;
        }
        if let Some(previous) = out.last() {
            at = at.max(previous.time + 1);
        }
        out.push(VolumePoint { time: at, volume, curve: Curve::Step });
    }
    VolumeEnvelope::new(out)
}

/// Channel names for imported layers: the file name without its extension, or the
/// skin slot. Names that would repeat get the slot and volume appended.
pub fn layer_names(layers: &[ImportedLayer]) -> Vec<String> {
    let slot = |l: &ImportedLayer| osu_format::sample::sample_file_stem(l.sample_set, l.hitsound, 1).unwrap_or_default();
    let base: Vec<String> = layers
        .iter()
        .map(|l| match &l.sample {
            LayerSample::File(name) => match name.rfind('.') {
                Some(i) if i > 0 => name[..i].to_owned(),
                _ => name.clone(),
            },
            LayerSample::Skin => format!("{} (skin)", slot(l)),
        })
        .collect();
    let mut names = Vec::with_capacity(layers.len());
    for (i, layer) in layers.iter().enumerate() {
        let clashes: Vec<usize> = (0..layers.len()).filter(|&j| base[j] == base[i]).collect();
        let mut name = base[i].clone();
        if clashes.len() > 1 {
            if clashes.iter().any(|&j| slot(&layers[j]) != slot(layer)) && matches!(layer.sample, LayerSample::File(_)) {
                name = format!("{name} ({})", slot(layer));
            }
            if clashes.iter().any(|&j| layers[j].volume != layer.volume) {
                name = format!("{name} {}%", (layer.volume * 100.0).round());
            }
        }
        names.push(name);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic layer map in the Preview Helper's shape. Mapset files: kick.wav,
    /// snare.ogg, crash.wav, soft-hitclap2.wav.
    const LAYERS: &str = "osu file format v14\n\n[General]\nSampleSet: Soft\nMode: 3\n\n[TimingPoints]\n0,500,4,2,0,65,1,0\n0,-100,4,2,0,80,0,0\n2002,-100,4,2,0,50,0,0\n3000,-100,4,2,2,50,0,0\n\n[HitObjects]\n256,192,1000,5,0,1:1:0:100:kick\n295,192,1500,1,8,2:2:0:100:snare\n256,192,2000,1,0,1:1:0:100:kick\n497,192,1000,1,2,0:0:0:0:\n440,192,1000,1,4,0:0:0:70:crash.wav\n440,192,2500,1,4,0:0:0:0:crash\n100,192,3000,1,8,0:0:0:0:\n100,192,3500,1,8,0:0:0:0:missing\n";

    fn mapset(name: &str) -> Option<String> {
        ["kick.wav", "snare.ogg", "crash.wav", "soft-hitclap2.wav"]
            .into_iter()
            .find(|f| f.eq_ignore_ascii_case(name))
            .map(str::to_owned)
    }

    fn file(name: &str) -> LayerSample {
        LayerSample::File(name.into())
    }

    #[test]
    fn notes_group_into_layers_by_sound() {
        let import = import_hitsound_diff(&OsuDocument::parse(LAYERS), mapset).unwrap();
        let summary: Vec<_> = import
            .layers
            .iter()
            .map(|l| (l.sample_set, l.hitsound, l.sample.clone(), l.volume, l.events.clone()))
            .collect();
        assert_eq!(
            summary,
            vec![
                // Index 2 from the greenline at 3000 finds soft-hitclap2.wav; the note at
                // 3500 names a missing file, so its bits play the same sound.
                (SampleSet::Soft, Hitsound::Clap, file("soft-hitclap2.wav"), 1.0, vec![3000, 3500]),
                (SampleSet::Normal, Hitsound::Normal, file("kick.wav"), 1.0, vec![1000, 2000]),
                (SampleSet::Soft, Hitsound::Clap, file("snare.ogg"), 1.0, vec![1500]),
                (SampleSet::Soft, Hitsound::Finish, file("crash.wav"), 0.7, vec![1000]),
                (SampleSet::Soft, Hitsound::Finish, file("crash.wav"), 1.0, vec![2500]),
                // Auto sets resolve through the timing point (soft); index 0 = skin.
                (SampleSet::Soft, Hitsound::Whistle, LayerSample::Skin, 1.0, vec![1000]),
            ]
        );
    }

    #[test]
    fn mania_plays_the_normal_only_for_its_bit_or_no_bits() {
        let doc = OsuDocument::parse(
            "osu file format v14\n\n[General]\nMode: 3\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n64,192,100,1,3,0:0:0:0:\n64,192,200,1,2,0:0:0:0:\n",
        );
        let import = import_hitsound_diff(&doc, |_| None).unwrap();
        let summary: Vec<_> = import.layers.iter().map(|l| (l.hitsound, l.events.clone())).collect();
        assert_eq!(summary, vec![(Hitsound::Normal, vec![100]), (Hitsound::Whistle, vec![100, 200])]);
    }

    #[test]
    fn greenline_volume_becomes_step_points_heard_by_every_note() {
        let import = import_hitsound_diff(&OsuDocument::parse(LAYERS), mapset).unwrap();
        let points: Vec<_> = import.volume.points.iter().map(|p| (p.time, p.volume, p.curve)).collect();
        // The redline and greenline at 0 collapse to the later one (80). The greenline at
        // 2002 already controls the note at 2000 (+5 ms lookup), so it moves to 2000.
        // The one at 3000 only changes the index, so it adds no point.
        assert_eq!(points, vec![(0, 80, Curve::Step), (2000, 50, Curve::Step)]);
        assert_eq!(import.volume.scale(100, 1999), 80);
        assert_eq!(import.volume.scale(100, 2000), 50);
    }

    #[test]
    fn constant_full_volume_adds_no_envelope() {
        let doc = OsuDocument::parse(
            "osu file format v14\n\n[General]\nMode: 3\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n1000,-100,4,1,0,100,0,0\n\n[HitObjects]\n64,192,100,1,0,0:0:0:0:\n",
        );
        assert!(import_hitsound_diff(&doc, |_| None).unwrap().volume.is_empty());
    }

    #[test]
    fn named_samples_follow_stables_extension_lookup() {
        let exists = |name: &str| mapset(name);
        assert_eq!(find_named_sample("kick", &exists).as_deref(), Some("kick.wav"));
        assert_eq!(find_named_sample("snare", &exists).as_deref(), Some("snare.ogg"));
        assert_eq!(find_named_sample("snare.wav", &exists).as_deref(), Some("snare.ogg"));
        assert_eq!(find_named_sample("KICK.WAV", &exists).as_deref(), Some("kick.wav"));
        assert_eq!(find_named_sample("missing", &exists), None);
    }

    #[test]
    fn standard_maps_are_refused() {
        let doc = OsuDocument::parse("osu file format v14\n\n[General]\nMode: 0\n\n[HitObjects]\n64,192,100,1,0,0:0:0:0:\n");
        assert!(matches!(import_hitsound_diff(&doc, |_| None), Err(ImportError::NotMania)));
    }

    #[test]
    fn repeated_names_get_their_volume() {
        let import = import_hitsound_diff(&OsuDocument::parse(LAYERS), mapset).unwrap();
        assert_eq!(
            layer_names(&import.layers),
            vec!["soft-hitclap2", "kick", "snare", "crash 70%", "crash 100%", "soft-hitwhistle (skin)"]
        );
    }

    #[test]
    fn general_sample_set_none_is_normal() {
        let doc = OsuDocument::parse("osu file format v14\n\n[General]\nSampleSet: None\n");
        assert_eq!(general_sample_set(&doc), SampleSet::Normal);
    }
}
