//! osu!stable playback rules: which files play for an object edge (D-26).
//!
//! Used by preview (so what you hear is what osu! plays) and by export verification
//! (re-read the written `.osu` and check every hit resolves to the intended files).

use osu_format::sample::sample_file_stem;
use osu_format::{HitSample, Hitsound, HitsoundFlags, SampleSet, TimingPoint};

/// Where osu! finds a sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// `<mapset>/<stem>.wav|.ogg`
    Beatmap(String),
    /// Skin (then osu! default resources), index stripped.
    Skin(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Playing {
    pub set: SampleSet,
    pub sound: Hitsound,
    pub index: u32,
    pub volume: u32,
}

/// Resolves the sounds of one clickable edge in osu!standard/catch.
///
/// * The normal sound always plays (`LayeredHitSounds`).
/// * `normal_set` 0 -> timing point set; `addition_set` 0 -> the resolved normal set.
/// * `index`/`volume` 0 -> timing point values. Slider edges have no per-edge index or
///   volume, so pass `HitSample::default()` for their index/volume part.
/// * `general_set` is the `[General] SampleSet` used when the timing point set is 0.
pub fn playing(
    flags: HitsoundFlags,
    normal_set: SampleSet,
    addition_set: SampleSet,
    sample: &HitSample,
    point: &TimingPoint,
    general_set: SampleSet,
) -> Vec<Playing> {
    let point_set = if point.sample_set == SampleSet::Auto { general_set } else { point.sample_set };
    let normal = if normal_set == SampleSet::Auto { point_set } else { normal_set };
    let addition = if addition_set == SampleSet::Auto { normal } else { addition_set };
    let index = if sample.index == 0 { point.sample_index } else { sample.index };
    let volume = if sample.volume == 0 { point.volume } else { sample.volume };

    let mut out = vec![Playing { set: normal, sound: Hitsound::Normal, index, volume }];
    out.extend(flags.additions().map(|sound| Playing { set: addition, sound, index, volume }));
    out
}

impl Playing {
    /// Linear playback gain osu!stable applies (findings 1): volume floored at 8 %, then hitnormal x0.8, whistle/clap
    /// x0.85, finish x1.0. Preview must apply this to sound like the game.
    pub fn gain(&self) -> f32 {
        let factor = match self.sound {
            Hitsound::Normal => 0.8,
            Hitsound::Whistle | Hitsound::Clap => 0.85,
            Hitsound::Finish => 1.0,
        };
        self.volume.max(8) as f32 / 100.0 * factor
    }
}

/// Lookup order: beatmap folder (only when index != 0), then skin with the index removed.
pub fn locate(p: &Playing, beatmap_has: impl Fn(&str) -> bool) -> Option<Location> {
    let skin = sample_file_stem(p.set, p.sound, 1)?;
    if p.index != 0 {
        let stem = sample_file_stem(p.set, p.sound, p.index)?;
        if beatmap_has(&stem) {
            return Some(Location::Beatmap(stem));
        }
    }
    Some(Location::Skin(skin))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(set: SampleSet, index: u32) -> TimingPoint {
        TimingPoint {
            time: 0.0,
            beat_length: -100.0,
            meter: 4,
            sample_set: set,
            sample_index: index,
            volume: 70,
            uninherited: false,
            effects: 0,
        }
    }

    #[test]
    fn additions_inherit_normal_set_and_point_index() {
        let p = playing(HitsoundFlags(8), SampleSet::Drum, SampleSet::Auto, &HitSample::default(), &point(SampleSet::Soft, 3), SampleSet::Normal);
        assert_eq!(p.len(), 2);
        assert_eq!((p[1].set, p[1].sound, p[1].index), (SampleSet::Drum, Hitsound::Clap, 3));
    }

    #[test]
    fn gain_matches_stable_scaling() {
        let p = Playing { set: SampleSet::Soft, sound: Hitsound::Normal, index: 1, volume: 5 };
        assert!((p.gain() - 0.08 * 0.8).abs() < 1e-6, "5 % is floored to 8 %");
        let clap = Playing { sound: Hitsound::Clap, volume: 100, ..p };
        assert!((clap.gain() - 0.85).abs() < 1e-6);
    }

    #[test]
    fn missing_custom_file_falls_back_to_skin() {
        let p = Playing { set: SampleSet::Soft, sound: Hitsound::Clap, index: 4, volume: 50 };
        assert_eq!(locate(&p, |s| s == "soft-hitclap4"), Some(Location::Beatmap("soft-hitclap4".into())));
        assert_eq!(locate(&p, |_| false), Some(Location::Skin("soft-hitclap".into())));
        let zero = Playing { index: 0, ..p };
        assert_eq!(locate(&zero, |_| true), Some(Location::Skin("soft-hitclap".into())));
    }
}
