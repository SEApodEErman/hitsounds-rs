//! Master volume envelope (D-44). It scales the greenline volume of every hit, so a
//! volume change or ramp never creates extra sample files: the mix inside each custom
//! sample is untouched, only the greenline that plays it changes.

use crate::Ms;
use serde::{Deserialize, Serialize};

/// osu! clamps greenline volume to 5..=100 (the editor won't go below 5).
pub const MIN_VOLUME: u32 = 5;
pub const MAX_VOLUME: u32 = 100;

/// How the envelope gets from one point to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    /// Interpolate linearly to the next point (a ramp).
    #[default]
    Linear,
    /// Hold this point's volume until the next point.
    Step,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumePoint {
    pub time: Ms,
    /// Percent, 5..=100.
    pub volume: u32,
    /// The segment from this point to the next one.
    #[serde(default)]
    pub curve: Curve,
}

/// Points in time order. Empty means 100 % everywhere (no effect).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VolumeEnvelope {
    pub points: Vec<VolumePoint>,
}

impl VolumeEnvelope {
    /// Sorts by time, keeps the last point given for a time, and clamps volumes.
    pub fn new(mut points: Vec<VolumePoint>) -> Self {
        for p in &mut points {
            p.volume = p.volume.clamp(MIN_VOLUME, MAX_VOLUME);
        }
        // Stable sort keeps input order within a time; keep the last of each run.
        points.sort_by_key(|p| p.time);
        let mut out: Vec<VolumePoint> = Vec::with_capacity(points.len());
        for p in points {
            match out.last_mut() {
                Some(last) if last.time == p.time => *last = p,
                _ => out.push(p),
            }
        }
        Self { points: out }
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Envelope value in percent at `time`. Before the first point the first value
    /// holds, after the last point the last value holds.
    pub fn at(&self, time: f64) -> f64 {
        let points = &self.points;
        let Some(first) = points.first() else {
            return f64::from(MAX_VOLUME);
        };
        let next = points.partition_point(|p| f64::from(p.time) <= time);
        if next == 0 {
            return f64::from(first.volume);
        }
        let a = points[next - 1];
        let Some(b) = points.get(next) else {
            return f64::from(a.volume);
        };
        match a.curve {
            Curve::Step => f64::from(a.volume),
            Curve::Linear => {
                let span = f64::from(b.time) - f64::from(a.time);
                let k = (time - f64::from(a.time)) / span;
                f64::from(a.volume) + (f64::from(b.volume) - f64::from(a.volume)) * k
            }
        }
    }

    /// Scales a greenline volume by the envelope at `time`, staying in 5..=100.
    pub fn scale(&self, volume: u32, time: Ms) -> u32 {
        if self.is_empty() {
            return volume;
        }
        let scaled = f64::from(volume) * self.at(f64::from(time)) / 100.0;
        (scaled.round() as u32).clamp(MIN_VOLUME, MAX_VOLUME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(time: Ms, volume: u32, curve: Curve) -> VolumePoint {
        VolumePoint { time, volume, curve }
    }

    #[test]
    fn empty_envelope_changes_nothing() {
        let e = VolumeEnvelope::default();
        assert_eq!(e.at(1234.0), 100.0);
        assert_eq!(e.scale(73, 1234), 73);
    }

    #[test]
    fn ramps_interpolate_and_steps_hold() {
        let e = VolumeEnvelope::new(vec![
            pt(1000, 100, Curve::Linear),
            pt(2000, 60, Curve::Step),
            pt(3000, 80, Curve::Linear),
        ]);
        assert_eq!(e.at(0.0), 100.0, "first value holds before the first point");
        assert_eq!(e.at(1500.0), 80.0, "halfway down the ramp");
        assert_eq!(e.at(2000.0), 60.0);
        assert_eq!(e.at(2999.0), 60.0, "step holds until the next point");
        assert_eq!(e.at(3000.0), 80.0);
        assert_eq!(e.at(9000.0), 80.0, "last value holds after the last point");
    }

    #[test]
    fn scale_multiplies_and_clamps() {
        let e = VolumeEnvelope::new(vec![pt(0, 50, Curve::Step)]);
        assert_eq!(e.scale(80, 100), 40);
        assert_eq!(e.scale(8, 100), MIN_VOLUME, "never below osu!'s 5 %");
    }

    #[test]
    fn new_sorts_dedupes_and_clamps() {
        let e = VolumeEnvelope::new(vec![
            pt(2000, 300, Curve::Linear),
            pt(1000, 0, Curve::Linear),
            pt(2000, 70, Curve::Step),
        ]);
        assert_eq!(e.points, vec![pt(1000, MIN_VOLUME, Curve::Linear), pt(2000, 70, Curve::Step)]);
    }

    #[test]
    fn serialises_as_a_plain_list() {
        let e = VolumeEnvelope::new(vec![pt(1000, 60, Curve::Step)]);
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, r#"[{"time":1000,"volume":60,"curve":"step"}]"#);
        assert_eq!(serde_json::from_str::<VolumeEnvelope>(&json).unwrap(), e);
    }
}
