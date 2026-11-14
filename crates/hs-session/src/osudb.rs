//! Reads the beatmap list of osu!stable's `osu!.db` (D-82), so the first scan of a
//! Songs folder doesn't open every `.osu` file: osu! already keeps their metadata in
//! this one file, which its own song select reads.
//!
//! Layout from the osu! wiki ("osu!.db", Legacy database file structure). Anything
//! unexpected is an error, and the caller then reads the `.osu` headers instead, so a
//! format change costs speed, never correctness.

use std::collections::HashMap;

/// The fields of one difficulty the mapset picker uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbBeatmap {
    pub artist: String,
    pub artist_unicode: String,
    pub title: String,
    pub title_unicode: String,
    pub creator: String,
    pub version: String,
    /// The `.osu` file name.
    pub file: String,
    pub set_id: Option<u32>,
    pub mode: u8,
    pub source: String,
    pub tags: String,
    /// The mapset folder under Songs.
    pub folder: String,
}

/// Before this version each beatmap starts with its size in bytes.
const SIZE_PREFIX_BEFORE: i32 = 20_191_106;
/// From this version difficulty values are floats and star ratings are stored.
const FLOAT_VALUES_FROM: i32 = 20_140_609;

/// The difficulties in `bytes`, grouped by lowercase folder name.
pub fn read(bytes: &[u8]) -> Result<HashMap<String, Vec<DbBeatmap>>, String> {
    let mut r = Reader { bytes, at: 0 };
    let version = r.i32()?;
    r.i32()?; // folder count
    r.bool()?; // account unlocked
    r.skip(8)?; // unlock date
    r.string()?; // player name
    let count = usize::try_from(r.i32()?).map_err(|_| "negative beatmap count")?;
    // Every entry is well over 40 bytes; a larger count is a misread.
    if count > bytes.len() / 40 {
        return Err(format!("implausible beatmap count {count}"));
    }
    let mut out: HashMap<String, Vec<DbBeatmap>> = HashMap::new();
    for _ in 0..count {
        let map = beatmap(&mut r, version)?;
        out.entry(map.folder.to_lowercase()).or_default().push(map);
    }
    Ok(out)
}

fn beatmap(r: &mut Reader<'_>, version: i32) -> Result<DbBeatmap, String> {
    if version < SIZE_PREFIX_BEFORE {
        r.i32()?;
    }
    let artist = r.string()?;
    let artist_unicode = r.string()?;
    let title = r.string()?;
    let title_unicode = r.string()?;
    let creator = r.string()?;
    let diff = r.string()?;
    r.string()?; // audio file
    r.string()?; // MD5
    let file = r.string()?;
    r.skip(1 + 2 * 3 + 8)?; // ranked status, object counts, modified time
    let floats = version >= FLOAT_VALUES_FROM;
    r.skip(if floats { 4 * 4 } else { 4 })?; // AR, CS, HP, OD
    r.skip(8)?; // slider velocity
    if floats {
        // Star ratings for standard, taiko, catch and mania: (mods, rating) pairs.
        for _ in 0..4 {
            let pairs = r.i32()?;
            for _ in 0..pairs {
                r.expect(0x08)?;
                r.skip(4)?;
                match r.u8()? {
                    0x0d => r.skip(8)?,
                    0x0c => r.skip(4)?,
                    other => return Err(format!("unknown star rating type {other:#x}")),
                }
            }
        }
    }
    r.skip(4 * 3)?; // drain, total and preview time
    let points = usize::try_from(r.i32()?).map_err(|_| "negative timing point count")?;
    r.skip(points.checked_mul(17).ok_or("timing point overflow")?)?;
    r.i32()?; // difficulty ID
    let set_id = u32::try_from(r.i32()?).ok().filter(|&id| id > 0);
    r.i32()?; // thread ID
    r.skip(4 + 2 + 4)?; // grades, local offset, stack leniency
    let mode = r.u8()?;
    let source = r.string()?;
    let tags = r.string()?;
    r.skip(2)?; // online offset
    r.string()?; // title font
    r.skip(1 + 8 + 1)?; // unplayed, last played, osz2
    let folder = r.string()?;
    r.skip(8 + 5)?; // last checked online, five override switches
    if !floats {
        r.skip(2)?;
    }
    r.skip(4 + 1)?; // last modification, mania scroll speed
    if mode > 3 {
        return Err(format!("unknown mode {mode} for {file}"));
    }
    Ok(DbBeatmap {
        artist,
        artist_unicode,
        title,
        title_unicode,
        creator,
        version: diff,
        file,
        set_id,
        mode,
        source,
        tags,
        folder,
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|&end| end <= self.bytes.len());
        let end = end.ok_or_else(|| format!("osu!.db ends early at byte {}", self.at))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn skip(&mut self, n: usize) -> Result<(), String> {
        self.take(n).map(|_| ())
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self) -> Result<bool, String> {
        Ok(self.u8()? != 0)
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        let at = self.at;
        match self.u8()? {
            b if b == byte => Ok(()),
            b => Err(format!("expected {byte:#x} at byte {at}, found {b:#x}")),
        }
    }

    fn i32(&mut self) -> Result<i32, String> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// `0x00` for none, or `0x0b`, a ULEB128 length and UTF-8 bytes.
    fn string(&mut self) -> Result<String, String> {
        match self.u8()? {
            0x00 => Ok(String::new()),
            0x0b => {
                let mut len: usize = 0;
                for shift in (0..35).step_by(7) {
                    let b = self.u8()?;
                    len |= usize::from(b & 0x7f) << shift;
                    if b & 0x80 == 0 {
                        let bytes = self.take(len)?;
                        return Ok(String::from_utf8_lossy(bytes).into_owned());
                    }
                }
                Err("string length too long".into())
            }
            other => Err(format!("unknown string marker {other:#x} at byte {}", self.at - 1)),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Writes an `osu!.db` with `maps` (folder, file, mode, title) in the current
    /// layout, with one float star rating so both value types are covered.
    pub(crate) fn synthetic(version: i32, maps: &[(&str, &str, u8, &str)]) -> Vec<u8> {
        let mut b = Vec::new();
        let string = |b: &mut Vec<u8>, s: &str| {
            if s.is_empty() {
                b.push(0);
            } else {
                b.push(0x0b);
                let mut n = s.len();
                loop {
                    let byte = (n & 0x7f) as u8;
                    n >>= 7;
                    b.push(if n > 0 { byte | 0x80 } else { byte });
                    if n == 0 {
                        break;
                    }
                }
                b.extend_from_slice(s.as_bytes());
            }
        };
        b.extend(version.to_le_bytes());
        b.extend(1i32.to_le_bytes());
        b.push(1);
        b.extend([0; 8]);
        string(&mut b, "player");
        b.extend((maps.len() as i32).to_le_bytes());
        for (i, (folder, file, mode, title)) in maps.iter().enumerate() {
            let mut m = Vec::new();
            for s in ["Artist", "", title, "", "Mapper", &format!("Diff {i}"), "audio.mp3", "md5", file] {
                string(&mut m, s);
            }
            m.extend([4, 1, 0, 2, 0, 0, 0]);
            m.extend([0; 8]);
            let floats = version >= FLOAT_VALUES_FROM;
            m.extend(if floats { vec![0; 16] } else { vec![0; 4] });
            m.extend(1.4f64.to_le_bytes());
            if floats {
                m.extend(1i32.to_le_bytes());
                m.push(0x08);
                m.extend(0i32.to_le_bytes());
                m.push(0x0d);
                m.extend(5.5f64.to_le_bytes());
                for _ in 0..2 {
                    m.extend(0i32.to_le_bytes());
                }
                m.extend(1i32.to_le_bytes());
                m.push(0x08);
                m.extend(0i32.to_le_bytes());
                m.push(0x0c);
                m.extend(2.5f32.to_le_bytes());
            }
            m.extend([0; 12]);
            m.extend(1i32.to_le_bytes());
            m.extend([0; 17]);
            m.extend(7i32.to_le_bytes());
            m.extend(500i32.to_le_bytes());
            m.extend(0i32.to_le_bytes());
            m.extend([9; 4]);
            m.extend([0; 2]);
            m.extend(0.7f32.to_le_bytes());
            m.push(*mode);
            string(&mut m, "Source");
            string(&mut m, "tag words");
            m.extend([0; 2]);
            string(&mut m, "");
            m.extend([1; 1]);
            m.extend([0; 9]);
            string(&mut m, folder);
            m.extend([0; 13]);
            if !floats {
                m.extend([0; 2]);
            }
            m.extend([0; 5]);
            if version < SIZE_PREFIX_BEFORE {
                b.extend((m.len() as i32).to_le_bytes());
            }
            b.extend(m);
        }
        b.extend(0i32.to_le_bytes());
        b
    }

    #[test]
    fn reads_current_and_older_layouts() {
        for version in [20_250_107, 20_191_105, 20_140_608] {
            let bytes = synthetic(version, &[("1 A - B", "a.osu", 0, "B"), ("1 A - B", "b.osu", 3, "B"), ("Pack\\2 C", "c.osu", 2, "D")]);
            let maps = read(&bytes).unwrap_or_else(|e| panic!("{version}: {e}"));
            let set = &maps["1 a - b"];
            assert_eq!(set.len(), 2);
            assert_eq!((set[0].file.as_str(), set[0].mode, set[0].set_id), ("a.osu", 0, Some(500)));
            assert_eq!((set[1].version.as_str(), set[1].tags.as_str()), ("Diff 1", "tag words"));
            assert_eq!(maps["pack\\2 c"][0].title, "D");
        }
    }

    #[test]
    fn a_truncated_or_foreign_file_is_an_error() {
        let bytes = synthetic(20_250_107, &[("1 A - B", "a.osu", 0, "B")]);
        assert!(read(&bytes[..bytes.len() - 30]).is_err());
        assert!(read(b"not a database at all, just some text").is_err());
    }
}
