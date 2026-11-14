//! Line-preserving `.osu` document.

use crate::hitobject::HitObject;
use crate::timing::TimingPoint;
use crate::Result;
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};

/// A `[Section]` and its lines, verbatim (without line terminators).
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// `None` for the preamble (`osu file format vNN` and anything before the first header).
    pub name: Option<String>,
    pub lines: Vec<String>,
    header: Option<(String, &'static str)>,
    endings: Vec<&'static str>,
}

impl Section {
    pub fn replace_lines(&mut self, lines: Vec<String>) {
        self.lines = lines;
        self.endings.clear();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OsuDocument {
    pub sections: Vec<Section>,
    /// The default line ending for new lines. Existing lines retain their own.
    pub line_ending: &'static str,
    /// Whether the file ended with a line terminator.
    pub trailing_newline: bool,
    /// Whether the file started with a UTF-8 BOM.
    pub bom: bool,
}

impl OsuDocument {
    pub fn parse(text: &str) -> Self {
        let bom = text.starts_with('\u{feff}');
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let line_ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let trailing_newline = text.ends_with('\n');
        let mut sections = vec![Section { name: None, lines: Vec::new(), header: None, endings: Vec::new() }];
        for raw in text.split_inclusive('\n') {
            let (line, ending) = if let Some(line) = raw.strip_suffix("\r\n") {
                (line, "\r\n")
            } else if let Some(line) = raw.strip_suffix('\n') {
                (line, "\n")
            } else {
                (raw, "")
            };
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() > 2 {
                sections.push(Section {
                    name: Some(trimmed[1..trimmed.len() - 1].to_owned()),
                    lines: Vec::new(),
                    header: Some((line.to_owned(), ending)),
                    endings: Vec::new(),
                });
            } else {
                let section = sections.last_mut().expect("non-empty");
                section.lines.push(line.to_owned());
                section.endings.push(ending);
            }
        }
        Self {
            sections,
            line_ending,
            trailing_newline,
            bom,
        }
    }

    pub fn serialize(&self) -> String {
        let mut lines = Vec::new();
        for section in &self.sections {
            if let Some(name) = &section.name {
                lines.push(match &section.header {
                    Some((text, ending)) if text.trim().strip_prefix('[').and_then(|s| s.strip_suffix(']')) == Some(name) => {
                        (Cow::Borrowed(text.as_str()), *ending)
                    }
                    _ => (Cow::Owned(format!("[{name}]")), self.line_ending),
                });
            }
            lines.extend(section.lines.iter().enumerate().map(|(i, text)| {
                (Cow::Borrowed(text.as_str()), section.endings.get(i).copied().unwrap_or(self.line_ending))
            }));
        }
        let mut out = String::new();
        if self.bom {
            out.push('\u{feff}');
        }
        let len = lines.len();
        for (i, (text, ending)) in lines.into_iter().enumerate() {
            out.push_str(&text);
            if i + 1 < len || self.trailing_newline {
                out.push_str(if ending.is_empty() { self.line_ending } else { ending });
            }
        }
        out
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name.as_deref() == Some(name))
    }

    pub fn section_mut(&mut self, name: &str) -> Option<&mut Section> {
        self.sections.iter_mut().find(|s| s.name.as_deref() == Some(name))
    }

    pub fn format_version(&self) -> Option<u32> {
        self.sections.first()?.lines.iter().find_map(|line| {
            line.trim().strip_prefix("osu file format v")?.parse().ok()
        })
    }

    /// `Key: Value` lookup in a key-value section such as `[General]` or `[Metadata]`.
    pub fn value(&self, section: &str, key: &str) -> Option<&str> {
        self.section(section)?.lines.iter().find_map(|line| {
            let (k, v) = line.split_once(':')?;
            (k.trim() == key).then(|| v.trim())
        })
    }

    pub fn timing_points(&self) -> Result<Vec<TimingPoint>> {
        self.data_lines("TimingPoints")
            .map(|(i, l)| TimingPoint::parse(l, i))
            .collect()
    }

    pub fn hit_objects(&self) -> Result<Vec<HitObject>> {
        self.data_lines("HitObjects")
            .map(|(i, l)| HitObject::parse(l, i))
            .collect()
    }

    /// Replaces the data lines of `[TimingPoints]`. A point that is unchanged keeps its
    /// original line byte-for-byte, however it was formatted; only changed and new
    /// points are written in canonical form. Blank lines around the data are kept.
    pub fn set_timing_points(&mut self, points: &[TimingPoint]) {
        self.replace_data_lines(
            "TimingPoints",
            points.iter().map(TimingPoint::serialize).collect(),
            |line| TimingPoint::parse(line, 0).ok().map(|p| p.serialize()),
            |line| {
                let mut fields = line.split(',');
                let time = fields.next().unwrap_or_default();
                let redline = fields.nth(5).unwrap_or_default();
                format!("{time},{redline}")
            },
        );
    }

    /// Replaces the data lines of `[HitObjects]`, with the same guarantee as
    /// [`set_timing_points`](Self::set_timing_points).
    pub fn set_hit_objects(&mut self, objects: &[HitObject]) {
        self.replace_data_lines(
            "HitObjects",
            objects.iter().map(HitObject::serialize).collect(),
            |line| HitObject::parse(line, 0).ok().map(|o| o.serialize()),
            |line| line.split(',').take(4).collect::<Vec<_>>().join(","),
        );
    }

    fn data_lines<'a>(&'a self, name: &str) -> impl Iterator<Item = (usize, &'a str)> + 'a {
        self.section(name)
            .into_iter()
            .flat_map(|s| s.lines.iter().enumerate())
            .map(|(i, l)| (i, l.as_str()))
            .filter(|(_, l)| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with("//")
            })
    }

    fn replace_data_lines(
        &mut self,
        name: &str,
        new_lines: Vec<String>,
        canonical: impl Fn(&str) -> Option<String>,
        identity: impl Fn(&str) -> String,
    ) {
        let default_ending = self.line_ending;
        let Some(section) = self.section_mut(name) else {
            return;
        };
        let is_data = |l: &str| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with("//")
        };
        let mut originals: HashMap<String, VecDeque<(usize, String)>> = HashMap::new();
        for (i, line) in section.lines.iter().enumerate().filter(|(_, l)| is_data(l)) {
            if let Some(text) = canonical(line) {
                originals.entry(identity(&text)).or_default().push_back((i, text));
            }
        }
        let ending = |i| section.endings.get(i).copied().unwrap_or(default_ending);
        let mut cursor = section.lines.iter().position(|l| is_data(l)).unwrap_or_else(|| {
            section.lines.iter().rposition(|l| !l.trim().is_empty()).map_or(0, |i| i + 1)
        });
        let mut lines = section.lines[..cursor].to_vec();
        let mut endings: Vec<_> = (0..cursor).map(ending).collect();
        let copy_gaps = |range: std::ops::Range<usize>, lines: &mut Vec<String>, endings: &mut Vec<_>| {
            for i in range {
                if !is_data(&section.lines[i]) {
                    lines.push(section.lines[i].clone());
                    endings.push(ending(i));
                }
            }
        };
        for new in new_lines {
            let original = originals.get_mut(&identity(&new)).and_then(VecDeque::pop_front);
            if let Some((i, text)) = original {
                copy_gaps(cursor..i, &mut lines, &mut endings);
                cursor = cursor.max(i + 1);
                lines.push(if text == new { section.lines[i].clone() } else { new });
                endings.push(ending(i));
            } else {
                lines.push(new);
                endings.push(default_ending);
            }
        }
        copy_gaps(cursor..section.lines.len(), &mut lines, &mut endings);
        section.lines = lines;
        section.endings = endings;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "osu file format v14\r\n\r\n[General]\r\nAudioFilename: audio.mp3\r\nSampleSet: Soft\r\n\r\n[TimingPoints]\r\n0,500,4,2,0,70,1,0\r\n1000,-100,4,2,0,50,0,0\r\n\r\n\r\n[HitObjects]\r\n256,192,0,5,0,0:0:0:0:\r\n256,192,1000,1,8,0:0:0:0:\r\n";

    #[test]
    fn untouched_round_trip_is_byte_identical() {
        let doc = OsuDocument::parse(SAMPLE);
        assert_eq!(doc.serialize(), SAMPLE);
        assert_eq!(doc.line_ending, "\r\n");
    }

    #[test]
    fn mixed_line_endings_and_comments_survive_an_edit() {
        const MAP: &str = "\u{feff}osu file format v14\r\n \t[TimingPoints]  \n0,500,4,2,0,70,1,0\r\n// between\n1000,-100,4,2,0,50,0,0\r\n \n\t[HitObjects]\r\n256,192,0,1,0";
        let mut doc = OsuDocument::parse(MAP);
        assert_eq!(doc.serialize(), MAP);
        let mut points = doc.timing_points().unwrap();
        points[1].volume = 45;
        doc.set_timing_points(&points);
        assert_eq!(doc.serialize(), MAP.replace(",50,0,0", ",45,0,0"));
    }

    #[test]
    fn typed_views() {
        let doc = OsuDocument::parse(SAMPLE);
        assert_eq!(doc.value("General", "AudioFilename"), Some("audio.mp3"));
        assert_eq!(doc.timing_points().unwrap().len(), 2);
        assert_eq!(doc.hit_objects().unwrap().len(), 2);
    }

    #[test]
    fn rewriting_sections_keeps_layout() {
        let mut doc = OsuDocument::parse(SAMPLE);
        let points = doc.timing_points().unwrap();
        let objects = doc.hit_objects().unwrap();
        doc.set_timing_points(&points);
        doc.set_hit_objects(&objects);
        assert_eq!(doc.serialize(), SAMPLE);
    }

    #[test]
    fn untouched_lines_keep_their_formatting_when_others_change() {
        // Old-style and oddly formatted lines: a 6-field redline, "-100.0", a circle
        // with no hitSample, and a short hitSample.
        const ODD: &str = "osu file format v9\r\n\r\n[TimingPoints]\r\n0,500,4,2,0,70\r\n1000,-100.0,4,2,0,50,0,0\r\n2000,-50,4,2,0,50,0,0\r\n\r\n[HitObjects]\r\n256,192,0,5,0\r\n256,192,1000,1,8,0:0\r\n256,192,2000,1,0,0:0:0:0:\r\n";
        let mut doc = OsuDocument::parse(ODD);
        let mut points = doc.timing_points().unwrap();
        let mut objects = doc.hit_objects().unwrap();
        points[2].volume = 80;
        objects[2].hitsound = crate::HitsoundFlags(2);
        doc.set_timing_points(&points);
        doc.set_hit_objects(&objects);
        let expected = ODD
            .replace("2000,-50,4,2,0,50,0,0", "2000,-50,4,2,0,80,0,0")
            .replace("256,192,2000,1,0,0:0:0:0:", "256,192,2000,1,2,0:0:0:0:");
        assert_eq!(doc.serialize(), expected);
    }

    #[test]
    fn inserted_points_do_not_disturb_their_neighbours() {
        let mut doc = OsuDocument::parse(SAMPLE);
        let mut points = doc.timing_points().unwrap();
        points.insert(1, TimingPoint::parse("500,-100,4,3,2,40,0,0", 0).unwrap());
        doc.set_timing_points(&points);
        let expected = SAMPLE.replace("0,500,4,2,0,70,1,0\r\n", "0,500,4,2,0,70,1,0\r\n500,-100,4,3,2,40,0,0\r\n");
        assert_eq!(doc.serialize(), expected);
    }

    #[test]
    fn filling_an_empty_section_keeps_the_gap_after_it() {
        let mut doc = OsuDocument::parse("osu file format v14\n\n[TimingPoints]\n\n[HitObjects]\n");
        doc.set_timing_points(&[TimingPoint::parse("0,500,4,2,0,70,1,0", 0).unwrap()]);
        assert_eq!(doc.serialize(), "osu file format v14\n\n[TimingPoints]\n0,500,4,2,0,70,1,0\n\n[HitObjects]\n");
    }
}
