//! Project edits and undo history.
//!
//! The UI never mutates a project directly: it sends an [`Edit`], the core applies it
//! and the UI reads the result (docs/architecture.md, "Editing"). Keeping edits
//! as data makes them testable here and gives undo/redo one place to live.

use crate::diff::{Override, TickPolicy};
use crate::mute::SliderEndMute;
use crate::project::{ExportSettings, StemOrigin};
use crate::{Channel, ChannelId, Ms, Project, SampleSource, VolumeEnvelope, VolumePoint};
use osu_format::{Hitsound, SampleSet};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("channel {0} does not exist")]
    NoChannel(ChannelId),
    #[error("difficulty {0:?} is not part of this project")]
    NoDiff(String),
    #[error("a channel needs a concrete sample set (normal, soft or drum)")]
    AutoSampleSet,
    #[error("override {0} does not exist")]
    NoOverride(usize),
    #[error("the move exceeds the supported range of event times")]
    TimeOverflow,
    #[error("the move puts events outside the channel list")]
    ChannelOutOfRange,
}

/// The editable properties of a channel (everything except its id and events).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelProps {
    pub name: String,
    pub color: String,
    pub sample_set: SampleSet,
    pub hitsound: Hitsound,
    pub sample: SampleSource,
    pub volume: f32,
}

impl From<&Channel> for ChannelProps {
    fn from(channel: &Channel) -> Self {
        Self {
            name: channel.name.clone(),
            color: channel.color.clone(),
            sample_set: channel.sample_set,
            hitsound: channel.hitsound,
            sample: channel.sample.clone(),
            volume: channel.volume,
        }
    }
}

/// The editable settings of a difficulty profile (everything except its overrides).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffProps {
    pub enabled: bool,
    pub match_leniency_ms: f64,
    pub fallback: Option<ChannelId>,
    pub tick_policy: TickPolicy,
    pub slider_end_mute: Option<SliderEndMute>,
}

impl From<&crate::DiffProfile> for DiffProps {
    fn from(profile: &crate::DiffProfile) -> Self {
        Self {
            enabled: profile.enabled,
            match_leniency_ms: profile.match_leniency_ms,
            fallback: profile.fallback,
            tick_policy: profile.tick_policy,
            slider_end_mute: profile.slider_end_mute.clone(),
        }
    }
}

/// One user action. `diff: None` edits the base layer; `diff: Some(file)` edits that
/// difficulty only, through overrides (D-17).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Edit {
    AddChannel {
        props: ChannelProps,
        /// Position in the priority order; `None` appends.
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        events: Vec<Ms>,
    },
    SetChannel {
        id: ChannelId,
        props: ChannelProps,
    },
    /// Monitoring only (D-14a): never undoable, never affects export.
    SetMonitor {
        id: ChannelId,
        muted: bool,
        solo: bool,
    },
    RemoveChannel {
        id: ChannelId,
    },
    /// Moves a channel to position `to` in the priority order (D-09).
    MoveChannel {
        id: ChannelId,
        to: usize,
    },
    AddEvents {
        #[serde(default)]
        diff: Option<String>,
        events: Vec<(ChannelId, Ms)>,
    },
    RemoveEvents {
        #[serde(default)]
        diff: Option<String>,
        events: Vec<(ChannelId, Ms)>,
    },
    /// Shifts `events` by `delta` ms and `rows` channels down the priority order
    /// (negative = up), e.g. to fix events placed on the wrong channel.
    MoveEvents {
        #[serde(default)]
        diff: Option<String>,
        events: Vec<(ChannelId, Ms)>,
        delta: Ms,
        #[serde(default)]
        rows: i32,
    },
    SetFallback {
        channel: Option<ChannelId>,
    },
    SetDiff {
        file: String,
        props: DiffProps,
    },
    AddOverride {
        file: String,
        value: Override,
    },
    RemoveOverride {
        file: String,
        index: usize,
    },
    SetExport {
        export: ExportSettings,
    },
    /// Sets or clears the stem. `origin` is `None` for an imported stem (D-73).
    SetStem {
        path: Option<PathBuf>,
        #[serde(default)]
        origin: Option<StemOrigin>,
    },
    /// Replaces a volume envelope (D-44). `diff: None` sets the base envelope (`None`
    /// points = empty). On a diff, `Some` sets its override and `None` removes it.
    SetVolume {
        #[serde(default)]
        diff: Option<String>,
        points: Option<Vec<VolumePoint>>,
    },
    /// Several edits as one undo step (paste, import).
    Batch {
        edits: Vec<Edit>,
    },
}

impl Edit {
    /// Monitoring edits are not recorded in the undo history.
    pub fn undoable(&self) -> bool {
        match self {
            Edit::SetMonitor { .. } => false,
            Edit::Batch { edits } => edits.iter().any(Edit::undoable),
            _ => true,
        }
    }
}

fn apply_props(channel: &mut Channel, props: &ChannelProps) -> Result<(), EditError> {
    if props.sample_set == SampleSet::Auto {
        return Err(EditError::AutoSampleSet);
    }
    channel.name = props.name.clone();
    channel.color = props.color.clone();
    channel.sample_set = props.sample_set;
    channel.hitsound = props.hitsound;
    channel.sample = props.sample.clone();
    channel.volume = props.volume.clamp(0.0, 1.0);
    Ok(())
}

impl Project {
    /// Applies one edit. On error the project is left unchanged.
    pub fn apply(&mut self, edit: &Edit) -> Result<(), EditError> {
        let mut next = self.clone();
        next.apply_in_place(edit)?;
        *self = next;
        Ok(())
    }

    fn apply_in_place(&mut self, edit: &Edit) -> Result<(), EditError> {
        match edit {
            Edit::AddChannel { props, index, events } => {
                let mut channel = Channel::new(0, "", SampleSet::Soft, Hitsound::Normal);
                apply_props(&mut channel, props)?;
                for &t in events {
                    channel.add_event(t);
                }
                let id = self.add_channel(channel);
                if let Some(to) = index {
                    self.move_channel(id, *to);
                }
            }
            Edit::SetChannel { id, props } => {
                let channel = self.channel_mut(*id).ok_or(EditError::NoChannel(*id))?;
                apply_props(channel, props)?;
            }
            Edit::SetMonitor { id, muted, solo } => {
                let channel = self.channel_mut(*id).ok_or(EditError::NoChannel(*id))?;
                channel.muted = *muted;
                channel.solo = *solo;
            }
            Edit::RemoveChannel { id } => {
                if self.channel(*id).is_none() {
                    return Err(EditError::NoChannel(*id));
                }
                self.channels.retain(|c| c.id != *id);
                // Overrides that name the channel stay: ids are never reused, so they
                // are inert, and undo brings them back to life with the channel.
                if self.fallback == Some(*id) {
                    self.fallback = None;
                }
                for d in &mut self.diffs {
                    if d.fallback == Some(*id) {
                        d.fallback = None;
                    }
                }
            }
            Edit::MoveChannel { id, to } => {
                if self.channel(*id).is_none() {
                    return Err(EditError::NoChannel(*id));
                }
                self.move_channel(*id, *to);
            }
            Edit::AddEvents { diff: None, events } => {
                for &(id, t) in events {
                    self.channel_mut(id).ok_or(EditError::NoChannel(id))?.add_event(t);
                }
            }
            Edit::RemoveEvents { diff: None, events } => {
                for &(id, t) in events {
                    self.channel_mut(id).ok_or(EditError::NoChannel(id))?.remove_event(t);
                }
            }
            Edit::MoveEvents { diff: None, events, delta, rows } => {
                let moved = self.moved_events(events, *delta, *rows)?;
                for &(id, t) in events {
                    self.channel_mut(id).ok_or(EditError::NoChannel(id))?.remove_event(t);
                }
                for (id, t) in moved {
                    self.channel_mut(id).ok_or(EditError::NoChannel(id))?.add_event(t);
                }
            }
            Edit::AddEvents { diff: Some(file), events } => {
                for &(id, _) in events {
                    self.channel(id).ok_or(EditError::NoChannel(id))?;
                }
                let d = self.diff_mut(file).ok_or_else(|| EditError::NoDiff(file.clone()))?;
                for &(id, t) in events {
                    d.add_event(id, t);
                }
            }
            Edit::RemoveEvents { diff: Some(file), events } => {
                let d = self.diff_mut(file).ok_or_else(|| EditError::NoDiff(file.clone()))?;
                for &(id, t) in events {
                    d.remove_event(id, t);
                }
            }
            Edit::MoveEvents { diff: Some(file), events, delta, rows } => {
                let moved = self.moved_events(events, *delta, *rows)?;
                let d = self.diff_mut(file).ok_or_else(|| EditError::NoDiff(file.clone()))?;
                for &(id, t) in events {
                    d.remove_event(id, t);
                }
                for (id, t) in moved {
                    d.add_event(id, t);
                }
            }
            Edit::SetFallback { channel } => {
                if let Some(id) = channel {
                    self.channel(*id).ok_or(EditError::NoChannel(*id))?;
                }
                self.fallback = *channel;
            }
            Edit::SetDiff { file, props } => {
                if let Some(id) = props.fallback {
                    self.channel(id).ok_or(EditError::NoChannel(id))?;
                }
                let d = self.diff_mut(file).ok_or_else(|| EditError::NoDiff(file.clone()))?;
                d.enabled = props.enabled;
                d.match_leniency_ms = props.match_leniency_ms.clamp(0.0, 50.0);
                d.fallback = props.fallback;
                d.tick_policy = props.tick_policy;
                d.slider_end_mute = props.slider_end_mute.clone();
            }
            Edit::AddOverride { file, value } => {
                let d = self.diff_mut(file).ok_or_else(|| EditError::NoDiff(file.clone()))?;
                d.overrides.push(value.clone());
            }
            Edit::RemoveOverride { file, index } => {
                let d = self.diff_mut(file).ok_or_else(|| EditError::NoDiff(file.clone()))?;
                if *index >= d.overrides.len() {
                    return Err(EditError::NoOverride(*index));
                }
                d.overrides.remove(*index);
            }
            Edit::SetExport { export } => self.export = export.clone(),
            Edit::SetStem { path, origin } => {
                self.audio.stem = path.clone();
                self.audio.stem_origin = path.as_ref().and(origin.clone());
            }
            Edit::SetVolume { diff: None, points } => {
                self.volume = VolumeEnvelope::new(points.clone().unwrap_or_default());
            }
            Edit::SetVolume { diff: Some(file), points } => {
                let d = self.diff_mut(file).ok_or_else(|| EditError::NoDiff(file.clone()))?;
                d.volume = points.clone().map(VolumeEnvelope::new);
            }
            Edit::Batch { edits } => {
                for e in edits {
                    self.apply_in_place(e)?;
                }
            }
        }
        Ok(())
    }

    /// Where `events` land after moving `delta` ms and `rows` channels.
    fn moved_events(&self, events: &[(ChannelId, Ms)], delta: Ms, rows: i32) -> Result<Vec<(ChannelId, Ms)>, EditError> {
        events
            .iter()
            .map(|&(id, t)| {
                let row = self.priority(id).ok_or(EditError::NoChannel(id))?;
                let target = i64::try_from(row).map_err(|_| EditError::ChannelOutOfRange)? + i64::from(rows);
                let channel = usize::try_from(target)
                    .ok()
                    .and_then(|r| self.channels.get(r))
                    .ok_or(EditError::ChannelOutOfRange)?;
                Ok((channel.id, t.checked_add(delta).ok_or(EditError::TimeOverflow)?))
            })
            .collect()
    }

    fn move_channel(&mut self, id: ChannelId, to: usize) {
        if let Some(from) = self.priority(id) {
            let channel = self.channels.remove(from);
            let to = to.min(self.channels.len());
            self.channels.insert(to, channel);
        }
    }
}

/// Snapshot undo/redo. A project is a few kilobytes of integers, so whole-project
/// snapshots are cheaper than getting inverse edits right.
#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
}

const HISTORY_LIMIT: usize = 200;

impl History {
    /// Records the state from before an undoable edit.
    pub fn record(&mut self, before: Project) {
        self.undo.push(before);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, current: &mut Project) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(current, previous));
        true
    }

    pub fn redo(&mut self, current: &mut Project) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(current, next));
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Every state the history can return to, undo and redo alike.
    pub fn states(&self) -> impl Iterator<Item = &Project> {
        self.undo.iter().chain(&self.redo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DiffProfile;

    fn props(name: &str, set: SampleSet, sound: Hitsound) -> ChannelProps {
        ChannelProps {
            name: name.into(),
            color: "#ffffff".into(),
            sample_set: set,
            hitsound: sound,
            sample: SampleSource::SkinDefault,
            volume: 1.0,
        }
    }

    fn project() -> Project {
        let mut p = Project::new("t", ".");
        p.apply(&Edit::AddChannel { props: props("Kick", SampleSet::Drum, Hitsound::Normal), index: None, events: vec![] }).unwrap();
        p.apply(&Edit::AddChannel { props: props("Snare", SampleSet::Drum, Hitsound::Clap), index: None, events: vec![] }).unwrap();
        p.diffs.push(DiffProfile::new("hard.osu", "Hard"));
        p
    }

    #[test]
    fn base_events_add_move_remove() {
        let mut p = project();
        p.apply(&Edit::AddEvents { diff: None, events: vec![(1, 1000), (1, 1500), (2, 1250)] }).unwrap();
        p.apply(&Edit::MoveEvents { diff: None, events: vec![(1, 1000), (1, 1500)], delta: 500, rows: 0 }).unwrap();
        assert_eq!(p.channels[0].events, vec![1500, 2000]);
        p.apply(&Edit::RemoveEvents { diff: None, events: vec![(2, 1250)] }).unwrap();
        assert!(p.channels[1].events.is_empty());
    }

    #[test]
    fn events_move_to_another_channel() {
        let mut p = project();
        p.apply(&Edit::AddEvents { diff: None, events: vec![(1, 1000), (1, 1500), (2, 1250)] }).unwrap();
        // Kick events placed by mistake go one row down to Snare, and a beat later.
        p.apply(&Edit::MoveEvents { diff: None, events: vec![(1, 1000), (1, 1500)], delta: 0, rows: 1 }).unwrap();
        assert!(p.channels[0].events.is_empty());
        assert_eq!(p.channels[1].events, vec![1000, 1250, 1500]);
        // Off the end of the channel list: rejected and nothing changes.
        let before = p.clone();
        let err = p.apply(&Edit::MoveEvents { diff: None, events: vec![(2, 1000)], delta: 0, rows: 1 }).unwrap_err();
        assert_eq!(err, EditError::ChannelOutOfRange);
        assert_eq!(p, before);
    }

    #[test]
    fn diff_events_move_across_channels_as_overrides() {
        let mut p = project();
        p.apply(&Edit::AddEvents { diff: None, events: vec![(2, 1000)] }).unwrap();
        let diff = Some("hard.osu".to_string());
        p.apply(&Edit::MoveEvents { diff: diff.clone(), events: vec![(2, 1000)], delta: 10, rows: -1 }).unwrap();
        assert_eq!(p.channels[1].events, vec![1000], "the base layer is untouched");
        let d = p.diff("hard.osu").unwrap();
        assert_eq!(d.apply_overrides(p.base_events()), vec![(1, 1010)]);
    }

    #[test]
    fn volume_envelope_base_and_diff_override() {
        use crate::{Curve, VolumePoint};
        let mut p = project();
        let points = vec![VolumePoint { time: 2000, volume: 60, curve: Curve::Step }, VolumePoint { time: 0, volume: 100, curve: Curve::Linear }];
        p.apply(&Edit::SetVolume { diff: None, points: Some(points) }).unwrap();
        assert_eq!(p.volume.points.iter().map(|v| v.time).collect::<Vec<_>>(), vec![0, 2000], "sorted");
        let hard = Some("hard.osu".to_string());
        assert_eq!(p.volume_for(p.diff("hard.osu")).at(1000.0), 80.0, "diffs share the base");
        p.apply(&Edit::SetVolume { diff: hard.clone(), points: Some(vec![VolumePoint { time: 0, volume: 50, curve: Curve::Step }]) }).unwrap();
        assert_eq!(p.volume_for(p.diff("hard.osu")).at(1000.0), 50.0, "the override replaces it");
        p.apply(&Edit::SetVolume { diff: hard, points: None }).unwrap();
        assert!(p.diff("hard.osu").unwrap().volume.is_none());
        p.apply(&Edit::SetVolume { diff: None, points: None }).unwrap();
        assert!(p.volume.is_empty());
    }

    #[test]
    fn failed_edit_leaves_project_untouched() {
        let mut p = project();
        let before = p.clone();
        let err = p.apply(&Edit::AddEvents { diff: None, events: vec![(1, 1000), (99, 1000)] }).unwrap_err();
        assert_eq!(err, EditError::NoChannel(99));
        assert_eq!(p, before);
    }

    #[test]
    fn diff_events_become_overrides_and_cancel_out() {
        let mut p = project();
        p.apply(&Edit::AddEvents { diff: None, events: vec![(1, 1000)] }).unwrap();
        let hard = || Some("hard.osu".to_owned());

        // Removing a base event in the diff is a RemoveEvent override...
        p.apply(&Edit::RemoveEvents { diff: hard(), events: vec![(1, 1000)] }).unwrap();
        assert_eq!(p.diffs[0].overrides, vec![Override::RemoveEvent { channel: 1, time: 1000 }]);
        // ...and adding it back cancels the override instead of stacking another.
        p.apply(&Edit::AddEvents { diff: hard(), events: vec![(1, 1000)] }).unwrap();
        assert!(p.diffs[0].overrides.is_empty());

        // A diff-only event is an AddEvent override, removed the same way.
        p.apply(&Edit::AddEvents { diff: hard(), events: vec![(2, 1250)] }).unwrap();
        assert_eq!(p.diffs[0].overrides, vec![Override::AddEvent { channel: 2, time: 1250 }]);
        p.apply(&Edit::RemoveEvents { diff: hard(), events: vec![(2, 1250)] }).unwrap();
        assert!(p.diffs[0].overrides.is_empty());
        // The base layer never changed.
        assert_eq!(p.channels[0].events, vec![1000]);
    }

    #[test]
    fn removing_a_channel_clears_fallbacks() {
        let mut p = project();
        p.apply(&Edit::SetFallback { channel: Some(2) }).unwrap();
        p.diffs[0].fallback = Some(2);
        p.apply(&Edit::RemoveChannel { id: 2 }).unwrap();
        assert_eq!((p.fallback, p.diffs[0].fallback), (None, None));
        // Ids are never reused.
        p.apply(&Edit::AddChannel { props: props("Hat", SampleSet::Soft, Hitsound::Whistle), index: Some(0), events: vec![] }).unwrap();
        assert_eq!(p.channels[0].id, 3);
    }

    #[test]
    fn channel_order_is_priority() {
        let mut p = project();
        p.apply(&Edit::MoveChannel { id: 2, to: 0 }).unwrap();
        assert_eq!(p.channels.iter().map(|c| c.id).collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(p.priority(2), Some(0));
    }

    #[test]
    fn auto_sample_set_is_rejected() {
        let mut p = project();
        let err = p.apply(&Edit::SetChannel { id: 1, props: props("Kick", SampleSet::Auto, Hitsound::Normal) }).unwrap_err();
        assert_eq!(err, EditError::AutoSampleSet);
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut p = project();
        let mut h = History::default();
        let original = p.clone();
        h.record(p.clone());
        p.apply(&Edit::AddEvents { diff: None, events: vec![(1, 1000)] }).unwrap();
        let edited = p.clone();
        assert!(h.undo(&mut p));
        assert_eq!(p, original);
        assert!(h.redo(&mut p));
        assert_eq!(p, edited);
        assert!(!h.redo(&mut p));
    }

    #[test]
    fn monitoring_is_not_undoable() {
        assert!(!Edit::SetMonitor { id: 1, muted: true, solo: false }.undoable());
        assert!(Edit::SetFallback { channel: None }.undoable());
    }

    #[test]
    fn edits_round_trip_through_json() {
        let e = Edit::AddEvents { diff: Some("hard.osu".into()), events: vec![(1, 1000)] };
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, r#"{"type":"add_events","diff":"hard.osu","events":[[1,1000]]}"#);
        assert_eq!(serde_json::from_str::<Edit>(&json).unwrap(), e);
    }
}
