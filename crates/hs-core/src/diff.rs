//! Per-difficulty profiles: which `.osu` they target and how they deviate from the base.

use crate::mute::SliderEndMute;
use crate::{ChannelId, Ms};
use osu_format::SampleSet;
use serde::{Deserialize, Serialize};

/// Inclusive time range in ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Ms,
    pub end: Ms,
}

impl TimeRange {
    pub fn contains(&self, t: Ms) -> bool {
        (self.start..=self.end).contains(&t)
    }
}

fn in_range(range: &Option<TimeRange>, t: Ms) -> bool {
    range.as_ref().is_none_or(|r| r.contains(t))
}

/// One deviation from the base layer. Overrides are applied in list order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Override {
    /// Diff-only hitsound (e.g. a kick on an extra triple, or a hand-picked sound
    /// on a selected object in the diff's timeline).
    AddEvent { channel: ChannelId, time: Ms },
    /// Drop a base event in this diff only.
    RemoveEvent { channel: ChannelId, time: Ms },
    /// Silence a channel for this diff, optionally only in a range.
    MuteChannel {
        channel: ChannelId,
        #[serde(default)]
        range: Option<TimeRange>,
    },
    /// Play `to` wherever the base plays `from` (e.g. kicks become a soft-hitwhistle in Hard).
    ReplaceChannel {
        from: ChannelId,
        to: ChannelId,
        #[serde(default)]
        range: Option<TimeRange>,
    },
    /// Slider body sound for the slider starting at `slider_time`
    /// (`sliderslide` always, plus `sliderwhistle` when `whistle`).
    SliderBody { slider_time: Ms, sample_set: SampleSet, whistle: bool },
}

/// What to do with events that land on a slider tick instead of a clickable edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TickPolicy {
    /// Report as orphaned (default; matches Hitsound Copier's default).
    #[default]
    Ignore,
    /// Render the event mix into a custom `slidertick` sample (Hitsound Copier's
    /// "copy to slider ticks").
    Render,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffProfile {
    /// The `.osu` filename inside the mapset folder.
    pub file: String,
    /// Display label; defaults to the diff's `Version`.
    pub label: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Max distance between a base event and a diff object to count as a match.
    #[serde(default = "default_match_leniency")]
    pub match_leniency_ms: f64,
    /// Overrides the project fallback for objects no event lands on.
    #[serde(default)]
    pub fallback: Option<ChannelId>,
    #[serde(default)]
    pub tick_policy: TickPolicy,
    /// Mute empty slider/spinner ends by snap and length (`None` = off).
    #[serde(default)]
    pub slider_end_mute: Option<SliderEndMute>,
    #[serde(default)]
    pub overrides: Vec<Override>,
    /// Replaces the project's volume envelope for this diff (`None` = use the base, D-44).
    #[serde(default)]
    pub volume: Option<crate::VolumeEnvelope>,
}

fn yes() -> bool {
    true
}

fn default_match_leniency() -> f64 {
    5.0
}

impl DiffProfile {
    pub fn new(file: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            label: label.into(),
            enabled: true,
            match_leniency_ms: default_match_leniency(),
            fallback: None,
            tick_policy: TickPolicy::Ignore,
            slider_end_mute: None,
            overrides: Vec::new(),
            volume: None,
        }
    }

    /// Adds a diff-only event. If this diff had removed that base event, the removal
    /// is cancelled instead, so add and remove never pile up for one event.
    pub fn add_event(&mut self, channel: ChannelId, time: Ms) {
        let removal = Override::RemoveEvent { channel, time };
        let addition = Override::AddEvent { channel, time };
        if let Some(i) = self.overrides.iter().position(|o| *o == removal) {
            self.overrides.remove(i);
        } else if !self.overrides.contains(&addition) {
            self.overrides.push(addition);
        }
    }

    /// Removes an event in this diff only: drops the diff-only event if there is one,
    /// else hides the base event.
    pub fn remove_event(&mut self, channel: ChannelId, time: Ms) {
        let removal = Override::RemoveEvent { channel, time };
        let addition = Override::AddEvent { channel, time };
        if let Some(i) = self.overrides.iter().position(|o| *o == addition) {
            self.overrides.remove(i);
        } else if !self.overrides.contains(&removal) {
            self.overrides.push(removal);
        }
    }

    /// Applies overrides to the base `(channel, time)` events.
    pub fn apply_overrides(&self, base: impl IntoIterator<Item = (ChannelId, Ms)>) -> Vec<(ChannelId, Ms)> {
        let mut events: Vec<(ChannelId, Ms)> = base.into_iter().collect();
        for o in &self.overrides {
            match o {
                Override::AddEvent { channel, time } => events.push((*channel, *time)),
                Override::RemoveEvent { channel, time } => events.retain(|e| e != &(*channel, *time)),
                Override::MuteChannel { channel, range } => {
                    events.retain(|(c, t)| !(c == channel && in_range(range, *t)))
                }
                Override::ReplaceChannel { from, to, range } => {
                    for (c, t) in events.iter_mut() {
                        if c == from && in_range(range, *t) {
                            *c = *to;
                        }
                    }
                }
                Override::SliderBody { .. } => {}
            }
        }
        events.sort_by_key(|&(c, t)| (t, c));
        events.dedup();
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_apply_in_order() {
        let mut d = DiffProfile::new("a.osu", "Hard");
        d.overrides = vec![
            Override::ReplaceChannel { from: 1, to: 2, range: Some(TimeRange { start: 0, end: 1000 }) },
            Override::RemoveEvent { channel: 3, time: 500 },
            Override::AddEvent { channel: 4, time: 750 },
            Override::MuteChannel { channel: 5, range: None },
        ];
        let out = d.apply_overrides([(1, 500), (1, 2000), (3, 500), (5, 100)]);
        assert_eq!(out, vec![(2, 500), (4, 750), (1, 2000)]);
    }
}
