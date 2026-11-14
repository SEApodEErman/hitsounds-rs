//! Hit objects, parsed only as far as hitsounding needs.
//!
//! `x,y,time,type,hitSound,objectParams,hitSample`
//! Sliders: `x,y,time,type,hitSound,curve,slides,length,edgeSounds,edgeSets,hitSample`

use crate::sample::{HitSample, HitsoundFlags, SampleSet};
use crate::{Result, invalid};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitObjectKind {
    Circle,
    Slider,
    Spinner,
    /// osu!mania hold note. Not a target mode (D-04) but parsed for completeness.
    Hold,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HitObject {
    pub x: i32,
    pub y: i32,
    pub time: i32,
    pub kind: HitObjectKind,
    /// The raw type field (keeps new-combo and colour-skip bits).
    pub type_bits: u32,
    pub hitsound: HitsoundFlags,
    /// Slider only: number of slides (1 = no repeat).
    pub slides: u32,
    /// Slider only: pixel length.
    pub length: f64,
    /// Slider only: one entry per edge (head, repeats..., tail).
    pub edge_sounds: Vec<HitsoundFlags>,
    /// Slider only: `(normalSet, additionSet)` per edge.
    pub edge_sets: Vec<(SampleSet, SampleSet)>,
    pub hit_sample: HitSample,
    /// All comma-separated fields as read; untouched fields are written back verbatim.
    raw: Vec<String>,
}

impl HitObject {
    pub fn parse(line: &str, line_no: usize) -> Result<Self> {
        let raw: Vec<String> = line.split(',').map(str::to_owned).collect();
        if raw.len() < 5 {
            return Err(invalid(line_no, "hit object needs at least 5 fields"));
        }
        let int = |i: usize| -> Result<i64> {
            raw[i]
                .trim()
                .parse::<f64>()
                .map(|v| v as i64)
                .map_err(|_| invalid(line_no, format!("bad hit object field {i}")))
        };
        let type_bits = int(3)? as u32;
        let kind = if type_bits & 1 != 0 {
            HitObjectKind::Circle
        } else if type_bits & 2 != 0 {
            HitObjectKind::Slider
        } else if type_bits & 8 != 0 {
            HitObjectKind::Spinner
        } else if type_bits & 128 != 0 {
            HitObjectKind::Hold
        } else {
            return Err(invalid(line_no, "unknown hit object type"));
        };

        let mut obj = Self {
            x: int(0)? as i32,
            y: int(1)? as i32,
            time: int(2)? as i32,
            kind,
            type_bits,
            hitsound: HitsoundFlags(int(4)? as u8),
            slides: 1,
            length: 0.0,
            edge_sounds: Vec::new(),
            edge_sets: Vec::new(),
            hit_sample: HitSample::default(),
            raw,
        };

        match kind {
            HitObjectKind::Slider => {
                obj.slides = obj.field(6).and_then(|s| s.parse().ok()).unwrap_or(1);
                obj.length = obj.field(7).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                if obj.slides > 100_000 || !obj.length.is_finite() || obj.length < 0.0 {
                    return Err(invalid(line_no, "slider has an invalid length or repeat count"));
                }
                let edges = obj.slides as usize + 1;
                obj.edge_sounds = match obj.field(8) {
                    Some(s) => s.split('|').map(|v| HitsoundFlags(v.parse().unwrap_or(0))).collect(),
                    None => vec![obj.hitsound; edges],
                };
                obj.edge_sets = match obj.field(9) {
                    Some(s) => s.split('|').map(parse_edge_set).collect(),
                    None => vec![(SampleSet::Auto, SampleSet::Auto); edges],
                };
                if let Some(s) = obj.field(10) {
                    obj.hit_sample = HitSample::parse(s).unwrap_or_default();
                }
            }
            HitObjectKind::Spinner => {
                if let Some(s) = obj.field(6) {
                    obj.hit_sample = HitSample::parse(s).unwrap_or_default();
                }
            }
            HitObjectKind::Hold => {
                // `endTime:hitSample`
                if let Some((_, sample)) = obj.field(5).and_then(|s| s.split_once(':')) {
                    obj.hit_sample = HitSample::parse(sample).unwrap_or_default();
                }
            }
            HitObjectKind::Circle => {
                if let Some(s) = obj.field(5) {
                    obj.hit_sample = HitSample::parse(s).unwrap_or_default();
                }
            }
        }
        Ok(obj)
    }

    /// End time of a spinner or hold note (`None` for circles and sliders, whose
    /// end depends on timing).
    pub fn explicit_end_time(&self) -> Option<i32> {
        let raw = match self.kind {
            HitObjectKind::Spinner => self.field(5)?,
            HitObjectKind::Hold => self.field(5)?.split(':').next()?,
            _ => return None,
        };
        raw.parse::<f64>().ok().map(|v| v as i32)
    }

    fn field(&self, i: usize) -> Option<&str> {
        self.raw.get(i).map(|s| s.trim()).filter(|s| !s.is_empty())
    }

    /// Re-serialises the object, rewriting only hitsound-related fields.
    pub fn serialize(&self) -> String {
        let mut out = self.raw.clone();
        out[4] = self.hitsound.0.to_string();
        let set_field = |out: &mut Vec<String>, i: usize, value: String| {
            if out.len() <= i {
                out.resize(i + 1, String::new());
            }
            out[i] = value;
        };
        match self.kind {
            HitObjectKind::Circle => set_field(&mut out, 5, self.hit_sample.serialize()),
            HitObjectKind::Spinner => set_field(&mut out, 6, self.hit_sample.serialize()),
            HitObjectKind::Slider => {
                let sounds = self.edge_sounds.iter().map(|f| f.0.to_string()).collect::<Vec<_>>().join("|");
                let sets = self
                    .edge_sets
                    .iter()
                    .map(|(n, a)| format!("{}:{}", n.index(), a.index()))
                    .collect::<Vec<_>>()
                    .join("|");
                set_field(&mut out, 8, sounds);
                set_field(&mut out, 9, sets);
                set_field(&mut out, 10, self.hit_sample.serialize());
            }
            HitObjectKind::Hold => {
                let end = out.get(5).and_then(|s| s.split(':').next()).unwrap_or("0").to_owned();
                set_field(&mut out, 5, format!("{end}:{}", self.hit_sample.serialize()));
            }
        }
        out.join(",")
    }
}

fn parse_edge_set(s: &str) -> (SampleSet, SampleSet) {
    let mut it = s.split(':').map(|v| v.trim().parse::<u32>().ok().and_then(SampleSet::from_index));
    let normal = it.next().flatten().unwrap_or_default();
    let addition = it.next().flatten().unwrap_or_default();
    (normal, addition)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hitsound;

    #[test]
    fn circle_round_trip() {
        let line = "256,192,1000,5,2,0:0:0:0:";
        let o = HitObject::parse(line, 0).unwrap();
        assert_eq!(o.kind, HitObjectKind::Circle);
        assert!(o.hitsound.has(Hitsound::Whistle));
        assert_eq!(o.serialize(), line);
    }

    #[test]
    fn slider_edges() {
        let line = "100,100,2000,2,0,B|200:200,2,140,2|0|8,2:0|0:0|3:2,0:0:0:0:";
        let o = HitObject::parse(line, 0).unwrap();
        assert_eq!(o.kind, HitObjectKind::Slider);
        assert_eq!(o.slides, 2);
        assert_eq!(o.edge_sounds.len(), 3);
        assert_eq!(o.edge_sets[2], (SampleSet::Drum, SampleSet::Soft));
        assert_eq!(o.serialize(), line);
    }
}
    #[test]
    fn malformed_slider_is_rejected_before_allocating_edges() {
        let line = "256,192,1000,2,0,L|256:288,4294967295,100";
        assert!(HitObject::parse(line, 0).is_err());
        assert_eq!(crate::OsuDocument::parse(line).serialize(), line);
    }
