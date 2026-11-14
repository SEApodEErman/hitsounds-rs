//! Recognising osu! sample filenames in a mapset folder.

use crate::library::SAMPLE_EXTENSIONS;

/// A gameplay sample file such as `soft-hitclap3.ogg` or `drum-sliderslide.wav`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleName {
    /// The name without extension, lower-cased: `soft-hitclap3`.
    pub stem: String,
    /// `hitnormal`, `hitwhistle`, `hitfinish`, `hitclap`, `sliderslide`,
    /// `sliderwhistle` or `slidertick`.
    pub kind: &'static str,
    /// The custom index the file belongs to. A name without a number is index 1.
    pub index: u32,
}

impl SampleName {
    pub fn is_hit(&self) -> bool {
        self.kind.starts_with("hit")
    }
}

const KINDS: [&str; 7] = ["hitnormal", "hitwhistle", "hitfinish", "hitclap", "sliderslide", "sliderwhistle", "slidertick"];

/// Parses a file name; `None` if it is not a gameplay sample.
pub fn parse_sample_name(file_name: &str) -> Option<SampleName> {
    let lower = file_name.to_ascii_lowercase();
    let (stem, ext) = lower.rsplit_once('.')?;
    if !SAMPLE_EXTENSIONS.contains(&ext) {
        return None;
    }
    let (set, rest) = stem.split_once('-')?;
    if !matches!(set, "normal" | "soft" | "drum") {
        return None;
    }
    let kind = KINDS.iter().find(|k| rest.starts_with(**k))?;
    let number = &rest[kind.len()..];
    let index = if number.is_empty() {
        1
    } else if number.bytes().all(|b| b.is_ascii_digit()) && !number.starts_with('0') {
        number.parse().ok()?
    } else {
        return None;
    };
    Some(SampleName { stem: stem.to_owned(), kind, index })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_gameplay_samples() {
        let n = parse_sample_name("Soft-HitClap3.OGG").unwrap();
        assert_eq!((n.stem.as_str(), n.kind, n.index, n.is_hit()), ("soft-hitclap3", "hitclap", 3, true));
        let n = parse_sample_name("drum-sliderslide.wav").unwrap();
        assert_eq!((n.kind, n.index, n.is_hit()), ("sliderslide", 1, false));
    }

    #[test]
    fn ignores_everything_else() {
        for name in ["audio.mp3", "kick.wav", "soft-hitclap3.txt", "taiko-normal-hitclap.wav", "soft-hitclapx.wav", "soft-hitclap03.wav", "soft-hit.wav"] {
            assert_eq!(parse_sample_name(name), None, "{name}");
        }
    }
}
