//! Writing resolved hitsounds into a difficulty.
//!
//! Ownership contract (D-19): hitsounds-rs owns, and only owns,
//! * hit object hitsound bits and normal/addition sets (per object or per slider edge),
//! * per-object `index`/`volume` (reset to 0 so greenlines control them),
//! * the `sampleSet`/`sampleIndex`/`volume` fields of timing points.
//!
//! SV, kiai, meter, redlines, positions and everything else are preserved; greenlines
//! are merged with [`TimingPointChange`] exactly like Mapping Tools does.

use hs_core::packing::CustomIndex;
use hs_core::projection::{DiffTimeline, EdgeKind};
use osu_format::sample::{HitsoundFlags, sample_file_stem, slider_file_stem};
use osu_format::timing::active_point;
use osu_format::{HitObject, Hitsound, SampleSet, TimingPoint, TimingPointChange};

/// One edge hit, ready to write.
#[derive(Debug, Clone, PartialEq)]
pub struct WrittenHit {
    /// Index into the timeline's edges.
    pub edge: usize,
    pub normal_set: SampleSet,
    pub addition_set: SampleSet,
    pub whistle: bool,
    pub finish: bool,
    pub clap: bool,
    /// `None` keeps whatever index is active (used by muted ends).
    pub index: Option<u32>,
    /// `None` keeps whatever volume is active (used by reset edges).
    pub volume: Option<u32>,
    /// A reset edge (D-36). Its index is decided once the other hits are written: it
    /// keeps the active index unless that index has a file for a sound it plays (D-81).
    pub reset: bool,
}

impl WrittenHit {
    /// A slider/spinner end silenced by [`hs_core::SliderEndMute`]: no additions, the
    /// configured set, quiet greenline (Hitsound Copier "mute sliderends").
    pub fn muted(edge: usize, mute: &hs_core::SliderEndMute) -> Self {
        Self {
            edge,
            normal_set: mute.sample_set,
            addition_set: SampleSet::Auto,
            whistle: false,
            finish: false,
            clap: false,
            index: mute.index,
            volume: Some(mute.volume),
            reset: false,
        }
    }

    /// An edge with no event and no fallback (D-36): a plain default hit. No additions,
    /// Auto sets, the skin's sound, whatever volume is active. The index stays as it is
    /// when the skin's sound is what plays there anyway, else it goes back to 0 (D-81).
    pub fn reset(edge: usize) -> Self {
        Self {
            edge,
            normal_set: SampleSet::Auto,
            addition_set: SampleSet::Auto,
            whistle: false,
            finish: false,
            clap: false,
            index: None,
            volume: None,
            reset: true,
        }
    }
}

/// How far past an object stable looks for its hitsound greenline (findings 1).
const LOOKAHEAD_MS: f64 = osu_format::timing::HITSOUND_LOOKAHEAD_MS;

/// Applies `hits` to the objects and timing points of one difficulty.
///
/// `schema` is the custom indices the export writes and `general_set` the difficulty's
/// `[General] SampleSet`; reset edges need both to know what an index would play.
pub fn apply_hits(
    timeline: &DiffTimeline,
    objects: &mut [HitObject],
    points: &mut Vec<TimingPoint>,
    hits: &[WrittenHit],
    schema: &[CustomIndex],
    general_set: SampleSet,
) {
    let mut changes = Vec::with_capacity(hits.len());
    for hit in hits {
        let edge = timeline.edges[hit.edge];
        let mut flags = HitsoundFlags::default();
        flags.set(Hitsound::Whistle, hit.whistle);
        flags.set(Hitsound::Finish, hit.finish);
        flags.set(Hitsound::Clap, hit.clap);

        let obj = &mut objects[edge.object];
        match edge.kind {
            EdgeKind::Circle | EdgeKind::SpinnerEnd => {
                obj.hitsound = flags;
                obj.hit_sample.normal_set = hit.normal_set;
                obj.hit_sample.addition_set = hit.addition_set;
                obj.hit_sample.index = 0;
                obj.hit_sample.volume = 0;
            }
            EdgeKind::SliderHead | EdgeKind::SliderRepeat | EdgeKind::SliderTail => {
                let edges = obj.slides.max(1) as usize + 1;
                obj.edge_sounds.resize(edges, obj.hitsound);
                obj.edge_sets.resize(edges, (obj.hit_sample.normal_set, obj.hit_sample.addition_set));
                obj.edge_sounds[edge.edge] = flags;
                obj.edge_sets[edge.edge] = (hit.normal_set, hit.addition_set);
            }
        }

        // Decided below, once every other hit's greenline is in place (D-81).
        if hit.reset {
            continue;
        }
        // Slider edges have no per-edge index/volume, so the greenline carries them for
        // every edge type (Mapping Tools convention). Its sample set is left alone (D-79).
        let point = TimingPoint {
            time: f64::from(edge.time),
            beat_length: -100.0,
            meter: 4,
            sample_set: hit.normal_set,
            sample_index: hit.index.unwrap_or(0),
            volume: hit.volume.unwrap_or(100),
            uninherited: false,
            effects: 0,
        };
        let mut change = TimingPointChange::hitsound(point);
        change.index = hit.index.is_some();
        change.volume = hit.volume.is_some();
        // D-79: the greenline keeps the difficulty's own sample set. Every hit carries
        // concrete sets on its object or slider edge, so it doesn't need the greenline's.
        // Whatever stays on Auto (objects with no hit, slider slides and unrendered
        // ticks) then plays the mapset's set instead of the set of the hit before it.
        change.sample_set = false;
        changes.push(change);
    }
    merge(points, &changes);

    // D-81: a reset edge plays the skin's hitnormal. An index without its own file for
    // that sound plays exactly that, so the index of the hit before it can stay, and
    // the map doesn't switch back to index 0 and out again around every empty object.
    // In time order: a reset that does go back to 0 makes 0 the active index for the
    // resets after it, up to the next hit.
    let mut resets: Vec<&WrittenHit> = hits.iter().filter(|h| h.reset).collect();
    resets.sort_by_key(|h| timeline.edges[h.edge].time);
    for hit in resets {
        let edge = timeline.edges[hit.edge];
        let time = f64::from(edge.time);
        let Some(point) = active_point(points, time + LOOKAHEAD_MS) else {
            continue;
        };
        let resolve = |set: SampleSet| match (set, point.sample_set) {
            (SampleSet::Auto, SampleSet::Auto) => general_set,
            (SampleSet::Auto, set) | (set, _) => set,
        };
        let edge_set = resolve(hit.normal_set);
        let mut stems: Vec<String> = sample_file_stem(edge_set, Hitsound::Normal, 1).into_iter().collect();
        // A slider body after a head or repeat plays at the same greenline.
        if matches!(edge.kind, EdgeKind::SliderHead | EdgeKind::SliderRepeat) {
            let body_set = resolve(objects[edge.object].hit_sample.normal_set);
            for set in [edge_set, body_set] {
                for kind in ["sliderslide", "sliderwhistle", "slidertick"] {
                    stems.extend(slider_file_stem(set, kind, 1));
                }
            }
        }
        if plays_skin(schema, point.sample_index, &stems) {
            continue;
        }
        let mut change = TimingPointChange::hitsound(TimingPoint {
            time,
            beat_length: -100.0,
            meter: 4,
            sample_set: SampleSet::Auto,
            sample_index: 0,
            volume: 100,
            uninherited: false,
            effects: 0,
        });
        change.sample_set = false;
        change.volume = false;
        merge(points, &[change]);
    }
}

/// Whether every sound in `stems` plays the skin's own sample at `index`: the index is
/// 0, or the export writes it and has no file for any of them. An index the export
/// doesn't write (the mapper's own files) may have any file, so it never counts.
fn plays_skin(schema: &[CustomIndex], index: u32, stems: &[String]) -> bool {
    index == 0
        || schema.iter().find(|c| c.index == Some(index)).is_some_and(|c| {
            stems.iter().all(|stem| {
                c.slots
                    .get(stem)
                    .is_none_or(|mixes| mixes.is_empty() || !CustomIndex::needs_file(stem, mixes))
            })
        })
}

/// Merges greenline changes as Mapping Tools does (D-07), then copies them onto points
/// the merge leaves alone but stable still reads.
fn merge(points: &mut Vec<TimingPoint>, changes: &[TimingPointChange]) {
    TimingPointChange::apply_all(points, changes, false);

    // Stable reads the greenline at `time + 5 ms`, but the merge only edits points
    // within 2 ms. A point the mapper left 3-5 ms after an object would therefore
    // still decide that object's sound, so it gets the same fields.
    for change in changes {
        let t = change.point.time;
        for p in points.iter_mut().filter(|p| p.time > t + change.fuzziness && p.time <= t + LOOKAHEAD_MS) {
            if change.sample_set {
                p.sample_set = change.point.sample_set;
            }
            if change.index {
                p.sample_index = change.point.sample_index;
            }
            if change.volume {
                p.volume = change.point.volume;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use osu_format::OsuDocument;

    const MAP: &str = "osu file format v14\n\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,70,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";

    #[test]
    fn writes_object_fields_and_greenline() {
        let doc = OsuDocument::parse(MAP);
        let tl = DiffTimeline::from_document(&doc).unwrap();
        let mut objects = doc.hit_objects().unwrap();
        let mut points = doc.timing_points().unwrap();
        apply_hits(&tl, &mut objects, &mut points, &[WrittenHit {
            edge: 0,
            normal_set: SampleSet::Soft,
            addition_set: SampleSet::Drum,
            whistle: false,
            finish: false,
            clap: true,
            index: Some(3),
            volume: Some(60),
            reset: false,
        }], &[], SampleSet::Normal);
        assert_eq!(objects[0].serialize(), "256,192,1000,1,8,2:3:0:0:");
        assert_eq!(points.len(), 2);
        assert_eq!(points[1].serialize(), "1000,-100,4,2,3,60,0,0");
    }

    #[test]
    fn muted_end_keeps_index_and_sets_quiet_volume() {
        const SLIDER: &str = "osu file format v14

[Difficulty]
SliderMultiplier:1
SliderTickRate:1

[TimingPoints]
0,500,4,2,7,70,1,0

[HitObjects]
0,0,0,2,0,L|50:0,1,50,0|0,0:0|0:0,0:0:0:0:
";
        let doc = OsuDocument::parse(SLIDER);
        let tl = DiffTimeline::from_document(&doc).unwrap();
        let mut objects = doc.hit_objects().unwrap();
        let mut points = doc.timing_points().unwrap();
        let tail = tl.edges.iter().position(|e| e.kind == EdgeKind::SliderTail).unwrap();
        apply_hits(&tl, &mut objects, &mut points, &[WrittenHit::muted(tail, &hs_core::SliderEndMute::default())], &[], SampleSet::Normal);
        assert_eq!(points.len(), 2);
        assert_eq!(points[1].serialize(), "250,-100,4,2,7,5,0,0");
    }

    #[test]
    fn reset_edge_clears_additions_and_uses_the_skin_index() {
        let doc = OsuDocument::parse(&MAP.replace("1,0,0:0:0:0:", "1,10,3:1:4:30:").replace("0,500,4,2,0,70,1,0", "0,500,4,2,6,70,1,0"));
        let tl = DiffTimeline::from_document(&doc).unwrap();
        let mut objects = doc.hit_objects().unwrap();
        let mut points = doc.timing_points().unwrap();
        apply_hits(&tl, &mut objects, &mut points, &[WrittenHit::reset(0)], &[], SampleSet::Normal);
        assert_eq!(objects[0].serialize(), "256,192,1000,1,0,0:0:0:0:");
        // Index back to 0 (skin), set and volume untouched.
        assert_eq!(points[1].serialize(), "1000,-100,4,2,0,70,0,0");
    }

    /// Index 7 holds a custom `drum-hitclap` (the snare) and, optionally, a custom
    /// `soft-hitnormal`.
    fn schema_7(soft_hitnormal: bool) -> Vec<CustomIndex> {
        use hs_core::packing::{Source, SourceMix};
        let file = |path: &str| {
            std::collections::BTreeSet::from([SourceMix { source: Source::File { path: path.into() }, gain_pct: 100 }])
        };
        let mut slots = std::collections::BTreeMap::from([("drum-hitclap".to_owned(), file("snare.wav"))]);
        if soft_hitnormal {
            slots.insert("soft-hitnormal".to_owned(), file("kick.wav"));
        }
        vec![CustomIndex { index: Some(7), slots }]
    }

    fn snare(edge: usize) -> WrittenHit {
        WrittenHit {
            edge,
            normal_set: SampleSet::Soft,
            addition_set: SampleSet::Drum,
            whistle: false,
            finish: false,
            clap: true,
            index: Some(7),
            volume: Some(80),
            reset: false,
        }
    }

    /// A snare at 1000 on index 7, then an empty circle at 1500.
    fn snare_then_empty(schema: &[CustomIndex], redline: &str) -> (Vec<HitObject>, Vec<TimingPoint>) {
        let doc = OsuDocument::parse(&MAP.replace("0,500,4,2,0,70,1,0", redline).replace(
            "256,192,1000,1,0,0:0:0:0:\n",
            "256,192,1000,1,0,0:0:0:0:\n256,192,1500,1,8,0:0:0:0:\n",
        ));
        let tl = DiffTimeline::from_document(&doc).unwrap();
        let mut objects = doc.hit_objects().unwrap();
        let mut points = doc.timing_points().unwrap();
        apply_hits(&tl, &mut objects, &mut points, &[snare(0), WrittenHit::reset(1)], schema, SampleSet::Normal);
        (objects, points)
    }

    fn lines(points: &[TimingPoint]) -> Vec<String> {
        points.iter().map(TimingPoint::serialize).collect()
    }

    #[test]
    fn reset_keeps_a_custom_index_that_has_no_file_for_its_hitnormal() {
        // D-81: index 7 has no soft-hitnormal7, so the empty circle plays the skin's
        // soft-hitnormal at index 7 as well. No greenline back to index 0.
        let (objects, points) = snare_then_empty(&schema_7(false), "0,500,4,2,0,70,1,0");
        assert_eq!(objects[1].serialize(), "256,192,1500,1,0,0:0:0:0:");
        assert_eq!(lines(&points), ["0,500,4,2,0,70,1,0", "1000,-100,4,2,7,80,0,0"]);
    }

    #[test]
    fn reset_goes_back_to_the_skin_when_the_index_customises_its_hitnormal() {
        let (_, points) = snare_then_empty(&schema_7(true), "0,500,4,2,0,70,1,0");
        assert_eq!(
            lines(&points),
            ["0,500,4,2,0,70,1,0", "1000,-100,4,2,7,80,0,0", "1500,-100,4,2,0,80,0,0"]
        );
    }

    #[test]
    fn reset_decides_by_the_set_it_will_play() {
        // The redline is Normal, so the empty circle plays normal-hitnormal: a custom
        // soft-hitnormal7 is no reason to leave index 7.
        let (_, points) = snare_then_empty(&schema_7(true), "0,500,4,1,0,70,1,0");
        assert_eq!(lines(&points), ["0,500,4,1,0,70,1,0", "1000,-100,4,1,7,80,0,0"]);
    }

    #[test]
    fn reset_never_keeps_an_index_the_export_does_not_write() {
        // The mapper's own index 6 may have any file, so the reset goes back to 0.
        let doc = OsuDocument::parse(&MAP.replace("0,500,4,2,0,70,1,0", "0,500,4,2,6,70,1,0"));
        let tl = DiffTimeline::from_document(&doc).unwrap();
        let mut objects = doc.hit_objects().unwrap();
        let mut points = doc.timing_points().unwrap();
        apply_hits(&tl, &mut objects, &mut points, &[WrittenHit::reset(0)], &schema_7(false), SampleSet::Normal);
        assert_eq!(points[1].serialize(), "1000,-100,4,2,0,70,0,0");
    }

    #[test]
    fn a_reset_slider_head_leaves_an_index_with_a_custom_slider_body() {
        let doc = OsuDocument::parse(
            "osu file format v14\n\n[Difficulty]\nSliderMultiplier:1\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,70,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n0,0,1500,2,0,L|50:0,1,50,0|0,0:0|0:0,0:0:0:0:\n",
        );
        let tl = DiffTimeline::from_document(&doc).unwrap();
        let head = tl.edges.iter().position(|e| e.kind == EdgeKind::SliderHead).unwrap();
        let mut schema = schema_7(true);
        let slide = schema[0].slots.remove("soft-hitnormal").unwrap();
        schema[0].slots.insert("soft-sliderslide".to_owned(), slide);
        let mut objects = doc.hit_objects().unwrap();
        let mut points = doc.timing_points().unwrap();
        apply_hits(&tl, &mut objects, &mut points, &[snare(0), WrittenHit::reset(head)], &schema, SampleSet::Normal);
        assert_eq!(points.last().unwrap().serialize(), "1500,-100,4,2,0,80,0,0");
    }

    #[test]
    fn greenline_just_after_the_object_cannot_override_the_hit() {
        // A greenline 4 ms after the circle is outside the 2 ms merge window but inside
        // stable's 5 ms lookahead.
        let doc = OsuDocument::parse(&MAP.replace("0,500,4,2,0,70,1,0", "0,500,4,2,0,70,1,0\n1004,-100,4,1,9,20,0,0"));
        let tl = DiffTimeline::from_document(&doc).unwrap();
        let mut objects = doc.hit_objects().unwrap();
        let mut points = doc.timing_points().unwrap();
        apply_hits(&tl, &mut objects, &mut points, &[WrittenHit {
            edge: 0,
            normal_set: SampleSet::Drum,
            addition_set: SampleSet::Drum,
            whistle: false,
            finish: false,
            clap: false,
            index: Some(2),
            volume: Some(80),
            reset: false,
        }], &[], SampleSet::Normal);
        let late = points.iter().find(|p| p.time == 1004.0).unwrap();
        // Index and volume follow the hit; the set stays the mapper's (D-79), which the
        // circle's own drum sets override anyway.
        assert_eq!((late.sample_set, late.sample_index, late.volume), (SampleSet::Normal, 2, 80));
        assert_eq!(objects[0].serialize(), "256,192,1000,1,0,3:3:0:0:");
    }

    #[test]
    fn hit_greenlines_keep_the_difficultys_sample_set() {
        // D-79: a drum hit must not turn the soft greenline into drum, or every Auto
        // field after it (empty objects, slider slides, ticks) would play drum too.
        let doc = OsuDocument::parse(MAP);
        let tl = DiffTimeline::from_document(&doc).unwrap();
        let mut objects = doc.hit_objects().unwrap();
        let mut points = doc.timing_points().unwrap();
        apply_hits(&tl, &mut objects, &mut points, &[WrittenHit {
            edge: 0,
            normal_set: SampleSet::Drum,
            addition_set: SampleSet::Normal,
            whistle: true,
            finish: false,
            clap: false,
            index: Some(4),
            volume: Some(90),
            reset: false,
        }], &[], SampleSet::Normal);
        assert_eq!(objects[0].serialize(), "256,192,1000,1,2,3:1:0:0:");
        assert_eq!(points[1].serialize(), "1000,-100,4,2,4,90,0,0");
    }
}
