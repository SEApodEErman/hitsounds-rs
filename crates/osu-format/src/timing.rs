//! Timing points (red/green lines) and the non-destructive greenline merge.

use crate::sample::SampleSet;
use crate::{Result, fmt_num, invalid};
use serde::{Deserialize, Serialize};

/// `time,beatLength,meter,sampleSet,sampleIndex,volume,uninherited,effects`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimingPoint {
    pub time: f64,
    /// ms per beat for redlines; `-100 / sv_multiplier` for greenlines.
    pub beat_length: f64,
    pub meter: u32,
    pub sample_set: SampleSet,
    pub sample_index: u32,
    pub volume: u32,
    pub uninherited: bool,
    pub effects: u8,
}

impl TimingPoint {
    pub const KIAI: u8 = 1;

    pub fn parse(line: &str, line_no: usize) -> Result<Self> {
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        if f.len() < 2 {
            return Err(invalid(line_no, "timing point needs at least time,beatLength"));
        }
        let num = |i: usize, default: f64| -> Result<f64> {
            match f.get(i) {
                None | Some(&"") => Ok(default),
                Some(s) => s
                    .parse::<f64>()
                    .map_err(|_| invalid(line_no, format!("bad timing point field {i}: {s:?}"))),
            }
        };
        let sample_set = SampleSet::from_index(num(3, 0.0)? as u32)
            .ok_or_else(|| invalid(line_no, "bad sample set"))?;
        Ok(Self {
            time: num(0, 0.0)?,
            beat_length: num(1, 0.0)?,
            meter: num(2, 4.0)? as u32,
            sample_set,
            sample_index: num(4, 0.0)? as u32,
            volume: num(5, 100.0)? as u32,
            uninherited: num(6, 1.0)? != 0.0,
            effects: num(7, 0.0)? as u8,
        })
    }

    pub fn serialize(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{}",
            fmt_num(self.time),
            fmt_num(self.beat_length),
            self.meter,
            self.sample_set.index(),
            self.sample_index,
            self.volume,
            u8::from(self.uninherited),
            self.effects
        )
    }

    pub fn kiai(&self) -> bool {
        self.effects & Self::KIAI != 0
    }

    /// Slider velocity multiplier this point applies (redlines reset to 1.0).
    pub fn sv_multiplier(&self) -> f64 {
        if self.uninherited || self.beat_length >= 0.0 {
            1.0
        } else {
            -100.0 / self.beat_length
        }
    }

    /// Whether `self` would change nothing if it followed `previous`.
    pub fn same_effect(&self, previous: &TimingPoint) -> bool {
        !self.uninherited
            && (self.sv_multiplier() - previous.sv_multiplier()).abs() < 1e-9
            && self.sample_set == previous.sample_set
            && self.sample_index == previous.sample_index
            && self.volume == previous.volume
            && self.kiai() == previous.kiai()
    }
}

/// Removes greenlines that change nothing: each one whose [`TimingPoint::same_effect`]
/// matches the point before it (D-80). Redlines are never removed, and neither is a
/// point that sits earlier than the one before it in an unsorted list. Returns how many
/// were removed. The active SV, sample set, index, volume and kiai are the same at
/// every time afterwards, so no object, slider body or tick sounds different.
pub fn remove_redundant(points: &mut Vec<TimingPoint>) -> usize {
    let before = points.len();
    let mut kept: Vec<TimingPoint> = Vec::with_capacity(before);
    for point in points.drain(..) {
        let redundant = kept
            .last()
            .is_some_and(|last| point.time >= last.time && point.same_effect(last));
        if !redundant {
            kept.push(point);
        }
    }
    *points = kept;
    before - points.len()
}

/// The active timing point at `time` (last point with `point.time <= time`).
///
/// osu! resolves an object's hitsound greenline at `object_time + 5 ms`, so callers
/// resolving hitsounds should pass `object_time + HITSOUND_LOOKAHEAD_MS`.
pub fn active_point(points: &[TimingPoint], time: f64) -> Option<&TimingPoint> {
    points.iter().rev().find(|p| p.time <= time).or(points.first())
}

/// Offset osu! uses when looking up the greenline that controls a hitsound.
pub const HITSOUND_LOOKAHEAD_MS: f64 = 5.0;

/// A field-selective edit of the timing at one instant.
///
/// Semantics (ported from Mapping Tools `TimingPointChange`, see THIRD_PARTY_NOTICES):
/// * Only the fields flagged here are written; SV, meter and everything else are kept.
/// * Existing points within `fuzziness` ms are edited in place.
/// * Otherwise a new greenline is inserted as a copy of the previous effective point
///   (a copy of a redline becomes a 1.0x greenline), so SV is preserved.
/// * The new greenline is dropped if it would have the same effect as the previous one.
#[derive(Debug, Clone)]
pub struct TimingPointChange {
    pub point: TimingPoint,
    pub sample_set: bool,
    pub index: bool,
    pub volume: bool,
    pub kiai: bool,
    pub fuzziness: f64,
}

impl TimingPointChange {
    pub fn hitsound(point: TimingPoint) -> Self {
        Self {
            point,
            sample_set: true,
            index: true,
            volume: true,
            kiai: false,
            fuzziness: 2.0,
        }
    }

    /// Applies `changes` in time order. With `all_after`, flagged fields are also
    /// propagated to every later point (Hitsound Copier's "overwrite everything").
    pub fn apply_all(points: &mut Vec<TimingPoint>, changes: &[TimingPointChange], all_after: bool) {
        let mut sorted: Vec<&TimingPointChange> = changes.iter().collect();
        sorted.sort_by(|a, b| a.point.time.total_cmp(&b.point.time));
        for change in sorted {
            change.apply(points, all_after);
        }
    }

    fn apply(&self, points: &mut Vec<TimingPoint>, all_after: bool) {
        let t = self.point.time;
        let mut previous: Option<usize> = None;
        let mut at_offset: Vec<usize> = Vec::new();
        for (i, p) in points.iter().enumerate() {
            if p.time < t && previous.is_none_or(|j| p.time >= points[j].time) {
                previous = Some(i);
            }
            if (p.time - t).abs() <= self.fuzziness {
                at_offset.push(i);
            }
        }
        if let Some(&last) = at_offset.last() {
            previous = Some(last);
        }

        let mut adding: Option<TimingPoint> = None;
        if at_offset.is_empty() {
            let mut new = previous.map_or_else(|| self.point.clone(), |j| points[j].clone());
            new.time = t;
            if new.uninherited {
                new.uninherited = false;
                new.beat_length = -100.0;
            }
            adding = Some(new);
        }

        let write = |p: &mut TimingPoint| {
            if self.sample_set {
                p.sample_set = self.point.sample_set;
            }
            if self.index {
                p.sample_index = self.point.sample_index;
            }
            if self.volume {
                p.volume = self.point.volume;
            }
            if self.kiai {
                p.effects = (p.effects & !TimingPoint::KIAI) | (self.point.effects & TimingPoint::KIAI);
            }
        };

        for &i in &at_offset {
            write(&mut points[i]);
        }
        if let Some(mut new) = adding {
            write(&mut new);
            let redundant = previous.is_some_and(|j| new.same_effect(&points[j]));
            if !redundant {
                insert_sorted(points, new);
            }
        }
        if all_after {
            for p in points.iter_mut().filter(|p| p.time > t) {
                write(p);
            }
        }
    }
}

/// Inserts keeping time order; at equal times redlines come before greenlines.
fn insert_sorted(points: &mut Vec<TimingPoint>, point: TimingPoint) {
    let pos = points
        .iter()
        .position(|p| p.time > point.time || (p.time == point.time && !p.uninherited && point.uninherited))
        .unwrap_or(points.len());
    points.insert(pos, point);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tp(line: &str) -> TimingPoint {
        TimingPoint::parse(line, 0).unwrap()
    }

    #[test]
    fn parse_serialize_round_trip() {
        for line in ["1000,461.538461538462,4,2,0,70,1,0", "1500,-50,4,3,5,40,0,1"] {
            assert_eq!(tp(line).serialize(), line);
        }
    }

    #[test]
    fn change_preserves_sv_and_inserts_greenline() {
        let mut points = vec![tp("0,500,4,2,0,70,1,0"), tp("1000,-50,4,2,0,70,0,0")];
        let mut target = tp("2000,-100,4,3,7,40,0,0");
        target.time = 2000.0;
        TimingPointChange::apply_all(&mut points, &[TimingPointChange::hitsound(target)], false);
        assert_eq!(points.len(), 3);
        let new = &points[2];
        assert_eq!(new.time, 2000.0);
        assert_eq!(new.beat_length, -50.0, "SV of previous greenline kept");
        assert_eq!((new.sample_set, new.sample_index, new.volume), (SampleSet::Drum, 7, 40));
    }

    #[test]
    fn change_edits_existing_point_within_fuzziness() {
        let mut points = vec![tp("0,500,4,2,0,70,1,0"), tp("1001,-50,4,2,0,70,0,0")];
        let target = tp("1000,-100,4,2,3,70,0,0");
        TimingPointChange::apply_all(&mut points, &[TimingPointChange::hitsound(target)], false);
        assert_eq!(points.len(), 2);
        assert_eq!(points[1].sample_index, 3);
        assert_eq!(points[1].beat_length, -50.0);
    }

    #[test]
    fn remove_redundant_keeps_redlines_and_every_real_change() {
        let mut points: Vec<_> = [
            "0,500,4,2,1,70,1,0",
            "0,-100,4,2,1,70,0,0",    // same as the redline: removed
            "100,-100,4,2,1,70,0,0",  // removed
            "200,-50,4,2,1,70,0,0",   // SV change: kept
            "300,-50,4,2,1,70,0,0",   // removed
            "400,-50,4,2,1,70,0,1",   // kiai on: kept
            "500,-50,4,2,1,70,0,0",   // kiai off: kept
            "600,-50,4,2,2,70,0,0",   // index: kept
            "700,-50,4,3,2,70,0,0",   // set: kept
            "800,-50,4,3,2,60,0,0",   // volume: kept
            "900,400,4,3,2,60,1,0",   // redline with the same fields: kept
            "900,-100,4,3,2,60,0,0",  // removed
            "850,-100,4,1,0,10,0,0",  // out of order: kept
        ]
        .into_iter()
        .map(tp)
        .collect();
        assert_eq!(remove_redundant(&mut points), 4);
        let times: Vec<_> = points.iter().map(|p| p.time).collect();
        assert_eq!(times, vec![0.0, 200.0, 400.0, 500.0, 600.0, 700.0, 800.0, 900.0, 850.0]);
    }

    #[test]
    fn remove_redundant_does_not_change_any_active_point() {
        let original: Vec<_> = [
            "0,500,4,2,0,80,1,0",
            "1000,-100,4,2,3,80,0,0",
            "1005,-100,4,2,3,80,0,0",
            "1500,-100,4,2,0,80,0,0",
            "2000,-100,4,2,0,80,0,0",
        ]
        .into_iter()
        .map(tp)
        .collect();
        let mut cleaned = original.clone();
        assert_eq!(remove_redundant(&mut cleaned), 2);
        for t in (0..2500).step_by(1) {
            let (a, b) = (
                active_point(&original, f64::from(t)).unwrap(),
                active_point(&cleaned, f64::from(t)).unwrap(),
            );
            assert!(a.same_effect(b) || a == b, "differs at {t} ms");
            assert_eq!((a.sample_set, a.sample_index, a.volume), (b.sample_set, b.sample_index, b.volume));
        }
    }

    #[test]
    fn redundant_greenline_is_skipped() {
        let mut points = vec![tp("0,500,4,2,0,70,1,0")];
        let same = tp("1000,-100,4,2,0,70,0,0");
        TimingPointChange::apply_all(&mut points, &[TimingPointChange::hitsound(same)], false);
        assert_eq!(points.len(), 1);
    }
}
