//! Channels: DAW-style tracks, one osu! sample slot each.

use crate::Ms;
use osu_format::{Hitsound, SampleSet};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Stable identifier; never reused within a project so overrides stay valid.
pub type ChannelId = u32;

/// Where a channel's audio comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SampleSource {
    /// The osu! default sound for this channel's sample set + hitsound, taken from
    /// the user's skin folder (decision D-30). Exports need no file for this.
    SkinDefault,
    /// A custom sample, path relative to the project directory (or absolute).
    File { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    /// `#rrggbb`
    pub color: String,
    /// Normal / Soft / Drum. `Auto` is not valid for a channel.
    pub sample_set: SampleSet,
    pub hitsound: Hitsound,
    pub sample: SampleSource,
    /// Linear gain 0.0..=1.0, mapped to osu! volume on export.
    pub volume: f32,
    /// Monitoring only - never affects export (decision D-14a).
    #[serde(default)]
    pub muted: bool,
    /// Monitoring only - never affects export (decision D-14a).
    #[serde(default)]
    pub solo: bool,
    /// Base-layer event times, sorted and unique.
    #[serde(default)]
    pub events: Vec<Ms>,
}

impl Channel {
    pub fn new(id: ChannelId, name: impl Into<String>, sample_set: SampleSet, hitsound: Hitsound) -> Self {
        Self {
            id,
            name: name.into(),
            color: "#8899aa".into(),
            sample_set,
            hitsound,
            sample: SampleSource::SkinDefault,
            volume: 1.0,
            muted: false,
            solo: false,
            events: Vec::new(),
        }
    }

    /// Inserts an event, keeping `events` sorted and unique. Returns false if present.
    pub fn add_event(&mut self, time: Ms) -> bool {
        match self.events.binary_search(&time) {
            Ok(_) => false,
            Err(pos) => {
                self.events.insert(pos, time);
                true
            }
        }
    }

    pub fn remove_event(&mut self, time: Ms) -> bool {
        match self.events.binary_search(&time) {
            Ok(pos) => {
                self.events.remove(pos);
                true
            }
            Err(_) => false,
        }
    }

    /// `soft-hitclap`-style label for UI and diagnostics.
    pub fn slot_label(&self) -> String {
        osu_format::sample::sample_file_stem(self.sample_set, self.hitsound, 1).unwrap_or_else(|| "auto".into())
    }
}
