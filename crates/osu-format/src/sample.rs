//! Sample sets, hitsound flags and the `hitSample` field.

use serde::{Deserialize, Serialize};

/// osu! sample set. `Auto` (0) means "inherit": from the timing point for a normal
/// set, and from the normal set for an addition set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SampleSet {
    #[default]
    Auto = 0,
    Normal = 1,
    Soft = 2,
    Drum = 3,
}

impl SampleSet {
    pub fn from_index(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Auto,
            1 => Self::Normal,
            2 => Self::Soft,
            3 => Self::Drum,
            _ => return None,
        })
    }

    pub fn index(self) -> u32 {
        self as u32
    }

    /// Filename prefix (`normal`, `soft`, `drum`). `Auto` has no file of its own.
    pub fn file_prefix(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Normal => Some("normal"),
            Self::Soft => Some("soft"),
            Self::Drum => Some("drum"),
        }
    }
}

/// One of the four gameplay hitsounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hitsound {
    Normal,
    Whistle,
    Finish,
    Clap,
}

impl Hitsound {
    pub const ALL: [Hitsound; 4] = [Self::Normal, Self::Whistle, Self::Finish, Self::Clap];

    pub fn is_addition(self) -> bool {
        self != Self::Normal
    }

    pub fn file_suffix(self) -> &'static str {
        match self {
            Self::Normal => "hitnormal",
            Self::Whistle => "hitwhistle",
            Self::Finish => "hitfinish",
            Self::Clap => "hitclap",
        }
    }

    fn bit(self) -> u8 {
        match self {
            Self::Normal => 1,
            Self::Whistle => 2,
            Self::Finish => 4,
            Self::Clap => 8,
        }
    }
}

/// The `hitSound` bit field of a hit object or slider edge.
///
/// In osu!standard/catch the normal sound plays regardless of bit 0
/// (`LayeredHitSounds`), so the normal bit is informational there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct HitsoundFlags(pub u8);

impl HitsoundFlags {
    pub fn has(self, hitsound: Hitsound) -> bool {
        self.0 & hitsound.bit() != 0
    }

    pub fn set(&mut self, hitsound: Hitsound, on: bool) {
        if on {
            self.0 |= hitsound.bit();
        } else {
            self.0 &= !hitsound.bit();
        }
    }

    pub fn additions(self) -> impl Iterator<Item = Hitsound> {
        Hitsound::ALL
            .into_iter()
            .filter(move |h| h.is_addition() && self.has(*h))
    }
}

/// `normalSet:additionSet:index:volume:filename`. Zero values defer to the timing point.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HitSample {
    pub normal_set: SampleSet,
    pub addition_set: SampleSet,
    pub index: u32,
    pub volume: u32,
    /// hitsounds-rs never writes this field (decision D-08); kept only for round-tripping.
    pub filename: String,
}

impl HitSample {
    pub fn parse(field: &str) -> Option<Self> {
        let mut parts = field.splitn(5, ':');
        let mut next_u32 = |default: u32| -> Option<u32> {
            match parts.next() {
                None | Some("") => Some(default),
                Some(s) => s.trim().parse().ok(),
            }
        };
        let normal_set = SampleSet::from_index(next_u32(0)?)?;
        let addition_set = SampleSet::from_index(next_u32(0)?)?;
        let index = next_u32(0)?;
        let volume = next_u32(0)?;
        let filename = parts.next().unwrap_or_default().to_owned();
        Some(Self {
            normal_set,
            addition_set,
            index,
            volume,
            filename,
        })
    }

    pub fn serialize(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}",
            self.normal_set.index(),
            self.addition_set.index(),
            self.index,
            self.volume,
            self.filename
        )
    }
}

/// Beatmap-folder filename stem for a sample, e.g. `soft-hitclap3`.
///
/// osu! omits the number for index 0 and 1. Index 0 additionally skips the beatmap
/// folder entirely (skin → default resources), so a file we write is only ever
/// picked up for index >= 1.
pub fn sample_file_stem(set: SampleSet, sound: Hitsound, index: u32) -> Option<String> {
    let prefix = set.file_prefix()?;
    let suffix = sound.file_suffix();
    Some(match index {
        0 | 1 => format!("{prefix}-{suffix}"),
        n => format!("{prefix}-{suffix}{n}"),
    })
}

/// Same as [`sample_file_stem`] for slider body/tick samples (`sliderslide`,
/// `sliderwhistle`, `slidertick`).
pub fn slider_file_stem(set: SampleSet, kind: &str, index: u32) -> Option<String> {
    let prefix = set.file_prefix()?;
    Some(match index {
        0 | 1 => format!("{prefix}-{kind}"),
        n => format!("{prefix}-{kind}{n}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_sample_round_trip() {
        let s = HitSample::parse("2:3:12:60:").unwrap();
        assert_eq!(s.normal_set, SampleSet::Soft);
        assert_eq!(s.addition_set, SampleSet::Drum);
        assert_eq!(s.index, 12);
        assert_eq!(s.volume, 60);
        assert_eq!(s.serialize(), "2:3:12:60:");
    }

    #[test]
    fn hit_sample_short_form_defaults() {
        let s = HitSample::parse("0:0").unwrap();
        assert_eq!(s, HitSample::default());
    }

    #[test]
    fn file_stems_follow_osu_naming() {
        assert_eq!(sample_file_stem(SampleSet::Soft, Hitsound::Normal, 1).unwrap(), "soft-hitnormal");
        assert_eq!(sample_file_stem(SampleSet::Drum, Hitsound::Clap, 4).unwrap(), "drum-hitclap4");
        assert!(sample_file_stem(SampleSet::Auto, Hitsound::Clap, 4).is_none());
        assert_eq!(slider_file_stem(SampleSet::Normal, "slidertick", 100).unwrap(), "normal-slidertick100");
    }

    #[test]
    fn flags() {
        let f = HitsoundFlags(2 | 8);
        assert!(f.has(Hitsound::Whistle) && f.has(Hitsound::Clap) && !f.has(Hitsound::Finish));
        assert_eq!(f.additions().collect::<Vec<_>>(), vec![Hitsound::Whistle, Hitsound::Clap]);
    }
}
