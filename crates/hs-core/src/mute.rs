//! "Mute slider ends", after Mapping Tools Hitsound Copier (`FilterMute`).
//!
//! Only edges that received no event are candidates, so a deliberate hitsound on a
//! slider end always wins. The user picks which ends are muted by beat snap and length.

use crate::projection::{DiffTimeline, Edge, EdgeKind};
use osu_format::SampleSet;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SliderEndMute {
    /// Beat divisors to consider, in priority order (1 = 1/1, 4 = 1/4, ...).
    /// An end is classified by the first divisor in this list it lies on.
    #[serde(default = "default_divisors")]
    pub divisors: Vec<u32>,
    /// Ends classified as one of these divisors are muted.
    #[serde(default = "default_muted")]
    pub muted: Vec<u32>,
    /// Minimum object duration, in beats, for its end to be muted.
    #[serde(default = "default_min_length")]
    pub min_length_beats: f64,
    /// Also mute spinner ends (Mapping Tools does).
    #[serde(default = "yes")]
    pub spinner_ends: bool,
    /// Sample set written on muted ends (Auto = leave to the greenline).
    #[serde(default)]
    pub sample_set: SampleSet,
    /// Custom index for the greenline; `None` keeps the current index.
    #[serde(default)]
    pub index: Option<u32>,
    /// Greenline volume. osu! floors hit sample volume at 8 %, so 5 % is "very quiet",
    /// not silent (findings 1).
    #[serde(default = "five")]
    pub volume: u32,
}

fn default_divisors() -> Vec<u32> {
    vec![1, 4, 3, 8, 6, 16, 12]
}
fn default_muted() -> Vec<u32> {
    vec![4, 3, 8, 6, 16, 12]
}
fn default_min_length() -> f64 {
    0.5
}
fn yes() -> bool {
    true
}
fn five() -> u32 {
    5
}

impl Default for SliderEndMute {
    fn default() -> Self {
        Self {
            divisors: default_divisors(),
            muted: default_muted(),
            min_length_beats: default_min_length(),
            spinner_ends: true,
            sample_set: SampleSet::Auto,
            index: None,
            volume: 5,
        }
    }
}

/// Snap tolerance in ms after resnapping (object times are integer-rounded).
const SNAP_EPSILON_MS: f64 = 1.5;

impl SliderEndMute {
    /// Whether `edge` is an end this filter mutes.
    pub fn should_mute(&self, edge: &Edge, timeline: &DiffTimeline) -> bool {
        let candidate = match edge.kind {
            // Repeating sliders are excluded, as in Hitsound Copier.
            EdgeKind::SliderTail => edge.slides == 1,
            EdgeKind::SpinnerEnd => self.spinner_ends,
            _ => false,
        };
        if !candidate {
            return false;
        }
        let t = f64::from(edge.time);
        let Some(red) = timeline.redline_at(t - 1.0) else {
            return false;
        };
        if red.beat_length <= 0.0 || edge.object_duration_ms + 1e-6 < self.min_length_beats * red.beat_length {
            return false;
        }
        match self.classify(t - red.time, red.beat_length) {
            Some(d) => self.muted.contains(&d),
            None => false,
        }
    }

    /// The first divisor (priority order) the time lies on, after resnapping to the
    /// nearest grid point of any considered divisor.
    pub fn classify(&self, offset_ms: f64, beat_ms: f64) -> Option<u32> {
        let all: Vec<u32> = self.divisors.iter().chain(&self.muted).copied().filter(|&d| d > 0).collect();
        let grid = |d: u32| {
            let step = beat_ms / f64::from(d);
            (offset_ms / step).round() * step
        };
        let snapped = all
            .iter()
            .map(|&d| grid(d))
            .min_by(|a, b| (a - offset_ms).abs().total_cmp(&(b - offset_ms).abs()))?;
        all.into_iter().find(|&d| (grid(d) - snapped).abs() < SNAP_EPSILON_MS && (snapped - offset_ms).abs() < SNAP_EPSILON_MS * 4.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::Redline;

    fn tl() -> DiffTimeline {
        DiffTimeline { redlines: vec![Redline { time: 0.0, beat_length: 500.0, meter: 4 }], ..Default::default() }
    }

    fn tail(time: i32, duration: f64, slides: u32) -> Edge {
        Edge { time, kind: EdgeKind::SliderTail, object: 0, edge: slides as usize, slides, object_duration_ms: duration, point_set: osu_format::SampleSet::Auto }
    }

    #[test]
    fn classification_uses_priority_order() {
        let m = SliderEndMute::default();
        assert_eq!(m.classify(1000.0, 500.0), Some(1)); // on the beat
        assert_eq!(m.classify(1250.0, 500.0), Some(4)); // 1/2 lies on the 1/4 grid first
        assert_eq!(m.classify(1167.0, 500.0), Some(3)); // 1/3, rounded ms
    }

    #[test]
    fn defaults_mute_off_beat_ends_but_not_on_beat() {
        let m = SliderEndMute::default();
        assert!(m.should_mute(&tail(1250, 250.0, 1), &tl()));
        assert!(!m.should_mute(&tail(1500, 500.0, 1), &tl()));
    }

    #[test]
    fn short_and_repeating_sliders_are_kept() {
        let m = SliderEndMute::default();
        assert!(!m.should_mute(&tail(1125, 125.0, 1), &tl()), "shorter than 1/2 beat");
        assert!(!m.should_mute(&tail(1750, 750.0, 3), &tl()), "repeating slider");
    }
}
