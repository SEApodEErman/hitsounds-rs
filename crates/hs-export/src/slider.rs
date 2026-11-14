//! Write and simulate slider tick and body samples (D-05).

use hs_core::packing::{ResolvedPackage, SliderSound};
use hs_core::projection::{DiffTimeline, EdgeKind, Target, Tick};
use hs_core::{DiffProfile, Ms, Override};
use osu_format::timing::{HITSOUND_LOOKAHEAD_MS, active_point};
use osu_format::{HitObject, HitObjectKind, Hitsound, SampleSet, TimingPoint, TimingPointChange};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type Bodies = BTreeMap<usize, (SampleSet, bool)>;
const TICK_LOOKAHEAD_MS: f64 = 2.0;
const TICK_REVERT_MS: f64 = 5.0;

/// Direct exports are applied to files that may already contain the last export.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct History {
    pub tick_indices: BTreeSet<u32>,
    pub bodies: BTreeMap<String, Vec<BodyEdit>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BodyFields {
    set: SampleSet,
    addition: SampleSet,
    whistle: bool,
}

impl BodyFields {
    pub fn read(object: &HitObject) -> Self {
        Self {
            set: object.hit_sample.normal_set,
            addition: object.hit_sample.addition_set,
            whistle: object.hitsound.has(Hitsound::Whistle),
        }
    }

    fn write(self, object: &mut HitObject) {
        object.hit_sample.normal_set = self.set;
        object.hit_sample.addition_set = self.addition;
        object.hitsound.set(Hitsound::Whistle, self.whistle);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BodyEdit {
    identity: (Ms, i32, i32),
    before: BodyFields,
    after: BodyFields,
}

impl History {
    /// Restore only our previous slider changes, keeping edits made in osu! since then.
    pub fn restore(&self, file: &str, objects: &mut [HitObject], points: &mut [TimingPoint]) {
        if let Some(edits) = self.bodies.get(file) {
            let mut by_object: BTreeMap<_, Vec<_>> = BTreeMap::new();
            for edit in edits {
                by_object.entry(edit.identity).or_default().push(edit);
            }
            for object in objects
                .iter_mut()
                .filter(|o| o.kind == HitObjectKind::Slider)
            {
                if let Some(edits) = by_object.get_mut(&(object.time, object.x, object.y))
                    && let Some(i) = edits
                        .iter()
                        .position(|e| e.after == BodyFields::read(object))
                {
                    edits.remove(i).before.write(object);
                }
            }
        }
        if self.tick_indices.is_empty() {
            return;
        }
        let mut i = 0;
        let mut previous = None;
        while i < points.len() {
            if !self.tick_indices.contains(&points[i].sample_index) {
                previous = Some(i);
                i += 1;
                continue;
            }
            let next = points[i + 1..]
                .iter()
                .position(|p| !self.tick_indices.contains(&p.sample_index))
                .map_or(points.len(), |offset| i + offset + 1);
            for j in i..next {
                let revert = (next < points.len()
                    && points[next].time <= points[j].time + TICK_REVERT_MS)
                    .then_some(next)
                    .or(previous);
                if let Some(k) = revert {
                    points[j].sample_set = points[k].sample_set;
                    points[j].sample_index = points[k].sample_index;
                    points[j].volume = points[k].volume;
                } else {
                    points[j].sample_index = 0;
                }
            }
            i = next;
        }
    }

    pub fn remember_bodies(&mut self, file: &str, before: &[BodyFields], objects: &[HitObject]) {
        let edits: Vec<_> = objects
            .iter()
            .zip(before)
            .filter_map(|(o, &before)| {
                let after = BodyFields::read(o);
                (o.kind == HitObjectKind::Slider && before != after).then_some(BodyEdit {
                    identity: (o.time, o.x, o.y),
                    before,
                    after,
                })
            })
            .collect();
        if edits.is_empty() {
            self.bodies.remove(file);
        } else {
            self.bodies.insert(file.to_owned(), edits);
        }
    }
}

pub(crate) fn bodies(profile: &DiffProfile, timeline: &DiffTimeline) -> Bodies {
    let heads: BTreeMap<_, Vec<_>> = timeline
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::SliderHead)
        .fold(BTreeMap::<Ms, Vec<usize>>::new(), |mut starts, edge| {
            starts.entry(edge.time).or_default().push(edge.object);
            starts
        });
    let mut result = Bodies::new();
    for edit in &profile.overrides {
        if let Override::SliderBody {
            slider_time,
            sample_set,
            whistle,
        } = *edit
        {
            for &object in heads.get(&slider_time).into_iter().flatten() {
                result.insert(object, (sample_set, whistle));
            }
        }
    }
    result
}

pub(crate) fn apply(
    timeline: &DiffTimeline,
    objects: &mut [HitObject],
    points: &mut Vec<TimingPoint>,
    bodies: &Bodies,
    resolved: &[ResolvedPackage],
    indices: &[u32],
) {
    for (&object, &(set, whistle)) in bodies {
        let o = &mut objects[object];
        o.hit_sample.normal_set = set;
        o.hit_sample.addition_set = SampleSet::Auto;
        o.hitsound.set(Hitsound::Whistle, whistle);
    }
    if !resolved.iter().any(|r| matches!(r.target, Target::Tick(_))) {
        return;
    }
    let baseline = points.clone();
    for (tick, hit, index) in assigned_ticks(timeline, resolved, indices) {
        if !bodies.contains_key(&tick.object) {
            objects[tick.object].hit_sample.normal_set = SampleSet::Auto;
        }
        let start = f64::from(tick.time);
        let revert = start + TICK_REVERT_MS;
        // Use the pre-tick state for the revert, including any mapper timing point
        // in this window. Only sample fields change; SV, BPM, kiai and meter survive.
        let Some(restore) = active_point(&baseline, revert) else {
            continue;
        };
        let write = |point: &TimingPoint, time, set, index, volume| {
            let mut change = TimingPointChange::hitsound(TimingPoint {
                time,
                sample_set: set,
                sample_index: index,
                volume,
                ..point.clone()
            });
            change.fuzziness = 0.0;
            change
        };
        let enter = write(restore, start, hit.normal_set, index, hit.volume);
        let exit = write(
            restore,
            revert,
            restore.sample_set,
            restore.sample_index,
            restore.volume,
        );
        TimingPointChange::apply_all(points, &[enter], false);
        for point in points
            .iter_mut()
            .filter(|p| p.time > start && p.time < revert)
        {
            point.sample_set = hit.normal_set;
            point.sample_index = index;
            point.volume = hit.volume;
        }
        TimingPointChange::apply_all(points, &[exit], false);
    }
}

pub(crate) fn assigned_ticks<'a>(
    timeline: &'a DiffTimeline,
    resolved: &'a [ResolvedPackage],
    indices: &'a [u32],
) -> impl Iterator<Item = (Tick, &'a ResolvedPackage, u32)> {
    resolved
        .iter()
        .zip(indices)
        .filter_map(|(hit, &index)| match hit.target {
            Target::Tick(i) => Some((timeline.ticks[i], hit, index)),
            Target::Edge(_) => None,
        })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SliderPlaying {
    pub set: SampleSet,
    pub sound: SliderSound,
    pub index: u32,
    pub volume: u32,
}

impl SliderPlaying {
    pub fn gain(self) -> f32 {
        self.volume.min(100) as f32 / 100.0
    }

    pub fn stem(self) -> Option<String> {
        self.sound.stem(self.set, self.index)
    }
}

fn set_at(own: SampleSet, point: Option<&TimingPoint>, general: SampleSet) -> SampleSet {
    if own != SampleSet::Auto {
        own
    } else {
        point
            .map(|p| p.sample_set)
            .filter(|s| *s != SampleSet::Auto)
            .unwrap_or(general)
    }
}

pub(crate) fn tick_at(
    tick: Tick,
    objects: &[HitObject],
    points: &[TimingPoint],
    general: SampleSet,
) -> SliderPlaying {
    let point = active_point(points, f64::from(tick.time) + TICK_LOOKAHEAD_MS);
    SliderPlaying {
        set: set_at(objects[tick.object].hit_sample.normal_set, point, general),
        sound: SliderSound::Tick,
        index: point.map_or(0, |p| p.sample_index),
        volume: point.map_or(100, |p| p.volume),
    }
}

/// While a slider plays, its slide and whistle follow the live control point.
/// Adjacent spans with the same samples and gain are coalesced for playback.
pub(crate) fn slides(
    timeline: &DiffTimeline,
    objects: &[HitObject],
    points: &[TimingPoint],
    general: SampleSet,
) -> Vec<(Ms, Ms, SliderPlaying)> {
    let ends: BTreeMap<_, _> = timeline
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::SliderTail)
        .map(|e| (e.object, e.time))
        .collect();
    let mut spans: Vec<(Ms, Ms, SliderPlaying)> = Vec::new();
    for edge in timeline
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::SliderHead)
    {
        let Some(&end) = ends.get(&edge.object).filter(|&&end| end > edge.time) else {
            continue;
        };
        let object = &objects[edge.object];
        let mut start = edge.time;
        let start_point = active_point(points, f64::from(start) + HITSOUND_LOOKAHEAD_MS);
        let after = points.partition_point(|p| p.time <= f64::from(start) + HITSOUND_LOOKAHEAD_MS);
        let mut point = start_point;
        let mut segments: Vec<(Ms, Ms, SliderPlaying)> = Vec::new();
        for next in points[after..].iter().map(Some).chain([None]) {
            let stop = next.map_or(end, |p| {
                ((p.time - HITSOUND_LOOKAHEAD_MS).ceil() as Ms).min(end)
            });
            if stop > start {
                let set = set_at(object.hit_sample.normal_set, point, general);
                let index = point.map_or(0, |p| p.sample_index);
                let volume = point.map_or(100, |p| p.volume);
                segments.push((
                    start,
                    stop,
                    SliderPlaying {
                        set,
                        sound: SliderSound::Slide,
                        index,
                        volume,
                    },
                ));
                if object.hitsound.has(Hitsound::Whistle) {
                    let set = if object.hit_sample.addition_set == SampleSet::Auto {
                        set
                    } else {
                        object.hit_sample.addition_set
                    };
                    segments.push((
                        start,
                        stop,
                        SliderPlaying {
                            set,
                            sound: SliderSound::Whistle,
                            index,
                            volume,
                        },
                    ));
                }
            }
            start = stop;
            point = next;
            if stop == end {
                break;
            }
        }
        segments.sort_by_key(|(_, _, p)| u8::from(p.sound == SliderSound::Whistle));
        let first = spans.len();
        for (start, stop, sound) in segments {
            let has_previous = spans.len() > first;
            if let Some(last) = spans
                .last_mut()
                .filter(|last| has_previous && last.1 == start && last.2 == sound)
            {
                last.1 = stop;
            } else {
                spans.push((start, stop, sound));
            }
        }
    }
    spans
}
