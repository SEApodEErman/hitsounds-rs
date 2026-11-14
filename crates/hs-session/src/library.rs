//! Sample library browser (D-45): lists the audio files under the user's library
//! folders. Read-only; a sample is copied into the project only when it is used.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Stop listing a folder after this many files, so pointing the library at a whole
/// drive can't stall the app.
const MAX_FILES: usize = 20_000;
const AUDIO: [&str; 4] = ["wav", "ogg", "mp3", "flac"];

#[derive(Debug, Serialize)]
pub struct LibraryFolder {
    pub root: PathBuf,
    /// Paths relative to `root`, `/`-separated, sorted case-insensitively.
    pub files: Vec<String>,
    pub truncated: bool,
    pub error: Option<String>,
}

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO.iter().any(|a| a.eq_ignore_ascii_case(e)))
}

pub fn scan(root: &Path) -> LibraryFolder {
    let mut files = Vec::new();
    let mut truncated = false;
    let error = walk(root, root, &mut files, &mut truncated).err().map(|e| format!("{}: {e}", root.display()));
    files.sort_by_cached_key(|f| natural_key(f));
    LibraryFolder { root: root.to_path_buf(), files, truncated, error }
}

/// One run of a name: digits compare by value, text case-insensitively.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Chunk {
    // Numbers sort before text, as in Explorer ("1.wav" before "a.wav").
    /// Value (leading zeros stripped, compared by length then digits), then the
    /// zero-padded length so "01" and "1" stay distinct but adjacent.
    Number(usize, String, usize),
    Text(String),
}

/// Natural sort key, per path segment, so "kick 2" comes before "kick 10" (D-45).
fn natural_key(path: &str) -> Vec<Vec<Chunk>> {
    path.split('/')
        .map(|segment| {
            let mut chunks = Vec::new();
            let mut rest = segment;
            while let Some(c) = rest.chars().next() {
                let digit = c.is_ascii_digit();
                let end = rest.find(|ch: char| ch.is_ascii_digit() != digit).unwrap_or(rest.len());
                let (run, tail) = rest.split_at(end);
                chunks.push(if digit {
                    let value = run.trim_start_matches('0');
                    Chunk::Number(value.len(), value.to_owned(), run.len())
                } else {
                    Chunk::Text(run.to_lowercase())
                });
                rest = tail;
            }
            chunks
        })
        .collect()
}

fn walk(root: &Path, dir: &Path, files: &mut Vec<String>, truncated: &mut bool) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if files.len() >= MAX_FILES {
            *truncated = true;
            return Ok(());
        }
        // `file_type` doesn't follow symlinks, so linked folders can't loop the walk.
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            // An unreadable subfolder is skipped rather than failing the whole library.
            let _ = walk(root, &path, files, truncated);
        } else if kind.is_file()
            && is_audio(&path)
            && let Ok(relative) = path.strip_prefix(root)
        {
            let parts: Vec<_> = relative.components().map(|c| c.as_os_str().to_string_lossy()).collect();
            files.push(parts.join("/"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_audio_recursively_and_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Kicks/Deep")).unwrap();
        for f in ["b.wav", "A.ogg", "notes.txt", "Kicks/k.WAV", "Kicks/Deep/sub.flac"] {
            std::fs::write(root.join(f), b"").unwrap();
        }
        let listed = scan(root);
        assert!(listed.error.is_none());
        assert!(!listed.truncated);
        assert_eq!(listed.files, vec!["A.ogg", "b.wav", "Kicks/Deep/sub.flac", "Kicks/k.WAV"]);
    }

    #[test]
    fn natural_order_puts_2_before_10() {
        let mut names = vec![
            "electro kick 10.wav", "electro kick 2.wav", "Electro kick 1.wav", "electro kick 20.wav",
            "electro hard kick + hihat 2.wav", "breakcore kick 1.wav", "kick 02.wav", "kick 2.wav",
            "Toms/Phat/t 10.wav", "Toms/Phat/t 9.wav", "Toms 2/a.wav",
        ];
        names.sort_by_cached_key(|f| natural_key(f));
        assert_eq!(names, vec![
            "breakcore kick 1.wav", "electro hard kick + hihat 2.wav", "Electro kick 1.wav",
            "electro kick 2.wav", "electro kick 10.wav", "electro kick 20.wav",
            "kick 2.wav", "kick 02.wav", "Toms/Phat/t 9.wav", "Toms/Phat/t 10.wav", "Toms 2/a.wav",
        ]);
    }

    #[test]
    fn missing_folder_reports_an_error() {
        let listed = scan(Path::new("this/folder/does/not/exist"));
        assert!(listed.error.is_some());
        assert!(listed.files.is_empty());
    }
}
