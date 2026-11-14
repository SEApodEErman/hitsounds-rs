//! What osu!stable plays for every clickable edge of a difficulty.
//!
//! Export verification reads the written `.osu` back through this, and preview plays
//! its result, so "what you hear" and "what was written" cannot drift apart (D-26).

use hs_core::projection::{DiffTimeline, EdgeKind};
use hs_core::resolve::{Playing, playing};
use osu_format::timing::{HITSOUND_LOOKAHEAD_MS, active_point};
use osu_format::{HitObject, HitSample, HitsoundFlags, SampleSet, TimingPoint};

/// The sounds of each edge of `timeline`, in edge order.
///
/// As osu!stable plays it (findings 1): the control point is the one active at `time + 5 ms`;
/// circles and spinners may carry their own index and volume; slider edges take sound
/// and sets from `edgeSounds`/`edgeSets` and always take index and volume from the
/// control point.
pub fn edge_sounds(
    timeline: &DiffTimeline,
    objects: &[HitObject],
    points: &[TimingPoint],
    general_set: SampleSet,
) -> Vec<Vec<Playing>> {
    let none = HitSample::default();
    timeline
        .edges
        .iter()
        .map(|edge| {
            let Some(obj) = objects.get(edge.object) else {
                return Vec::new();
            };
            let Some(point) = active_point(points, f64::from(edge.time) + HITSOUND_LOOKAHEAD_MS) else {
                return Vec::new();
            };
            let (flags, normal, addition, sample): (HitsoundFlags, SampleSet, SampleSet, &HitSample) = match edge.kind {
                EdgeKind::Circle | EdgeKind::SpinnerEnd => {
                    (obj.hitsound, obj.hit_sample.normal_set, obj.hit_sample.addition_set, &obj.hit_sample)
                }
                EdgeKind::SliderHead | EdgeKind::SliderRepeat | EdgeKind::SliderTail => {
                    let flags = obj.edge_sounds.get(edge.edge).copied().unwrap_or(obj.hitsound);
                    let (n, a) = obj
                        .edge_sets
                        .get(edge.edge)
                        .copied()
                        .unwrap_or((obj.hit_sample.normal_set, obj.hit_sample.addition_set));
                    (flags, n, a, &none)
                }
            };
            playing(flags, normal, addition, sample, point, general_set)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use osu_format::{Hitsound, OsuDocument};

    #[test]
    fn slider_edges_use_edge_fields_and_the_greenline_at_plus_five() {
        let doc = OsuDocument::parse(
            "osu file format v14\n\n[Difficulty]\nSliderMultiplier:1\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,1,70,1,0\n254,-100,4,3,4,40,0,0\n\n[HitObjects]\n0,0,0,2,0,L|50:0,1,50,2|8,1:0|0:2,0:0:9:90:\n",
        );
        let tl = DiffTimeline::from_document(&doc).unwrap();
        let sounds = edge_sounds(&tl, &doc.hit_objects().unwrap(), &doc.timing_points().unwrap(), SampleSet::Normal);
        // Head: normal set Normal, whistle inherits it; index/volume from the redline,
        // not from the slider's own 9/90.
        assert_eq!(
            sounds[0].iter().map(|p| (p.set, p.sound, p.index, p.volume)).collect::<Vec<_>>(),
            vec![(SampleSet::Normal, Hitsound::Normal, 1, 70), (SampleSet::Normal, Hitsound::Whistle, 1, 70)]
        );
        // Tail at 250: the greenline at 254 is within +5 ms, so it applies. Auto normal
        // set takes the greenline's Drum; the clap uses the edge's Soft addition set.
        assert_eq!(
            sounds[1].iter().map(|p| (p.set, p.sound, p.index, p.volume)).collect::<Vec<_>>(),
            vec![(SampleSet::Drum, Hitsound::Normal, 4, 40), (SampleSet::Soft, Hitsound::Clap, 4, 40)]
        );
    }

    #[test]
    fn circles_may_carry_their_own_index_and_volume() {
        let doc = OsuDocument::parse(
            "osu file format v14\n\n[TimingPoints]\n0,500,4,2,1,70,1,0\n\n[HitObjects]\n0,0,1000,1,0,3:0:7:55:\n",
        );
        let tl = DiffTimeline::from_document(&doc).unwrap();
        let sounds = edge_sounds(&tl, &doc.hit_objects().unwrap(), &doc.timing_points().unwrap(), SampleSet::Normal);
        assert_eq!((sounds[0][0].set, sounds[0][0].index, sounds[0][0].volume), (SampleSet::Drum, 7, 55));
    }
}
