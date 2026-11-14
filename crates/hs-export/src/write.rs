//! Stage replacements and restore earlier files if a later write fails.

use crate::ExportError;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tempfile::NamedTempFile;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileStamp {
    size: u64,
    modified: SystemTime,
}

pub(crate) fn snapshot(
    dir: &Path,
    relevant: impl Fn(&str) -> bool,
) -> io::Result<BTreeMap<String, FileStamp>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !relevant(&name) {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(io::Error::other(format!("{name} is a symbolic link")));
        }
        if file_type.is_file() {
            let metadata = entry.metadata()?;
            out.insert(
                name,
                FileStamp {
                    size: metadata.len(),
                    modified: metadata.modified()?,
                },
            );
        }
    }
    Ok(out)
}

fn stage(path: &Path, bytes: &[u8]) -> io::Result<NamedTempFile> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("output has no parent folder"))?;
    let mut file = NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    Ok(file)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    stage(path, bytes)?.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => fs::read(path).map(Some),
        Ok(_) => Err(io::Error::other(format!(
            "{} is not a regular file",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub(crate) struct Change<'a> {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<&'a [u8]>,
    staged: Option<NamedTempFile>,
}

impl<'a> Change<'a> {
    pub(crate) fn prepare(path: PathBuf, after: Option<&'a [u8]>) -> io::Result<Self> {
        let before = read_optional(&path)?;
        let staged = after.map(|bytes| stage(&path, bytes)).transpose()?;
        Ok(Self {
            path,
            before,
            after,
            staged,
        })
    }

    pub(crate) fn backup(&self, dir: &Path) -> io::Result<()> {
        if let Some(bytes) = &self.before {
            let file = self
                .path
                .file_name()
                .ok_or_else(|| io::Error::other("output has no filename"))?;
            let mut backup = fs::File::create_new(dir.join(file))?;
            backup.write_all(bytes)?;
            backup.sync_all()?;
        }
        Ok(())
    }

    pub(crate) fn replaces_file(&self) -> bool {
        self.before.is_some()
    }

    fn apply(&mut self) -> Result<(), ExportError> {
        if read_optional(&self.path)? != self.before {
            return Err(ExportError::Changed(self.path.display().to_string()));
        }
        if let Some(staged) = self.staged.take() {
            let replaced = if self.before.is_some() {
                staged.persist(&self.path)
            } else {
                staged.persist_noclobber(&self.path)
            };
            replaced.map_err(|e| e.error)?;
        } else if self.before.is_some() {
            fs::remove_file(&self.path)?;
        }
        Ok(())
    }

    fn restore(&self) -> io::Result<()> {
        if read_optional(&self.path)?.as_deref() != self.after {
            return Err(io::Error::other("the file changed during export"));
        }
        if let Some(before) = &self.before {
            write_atomic(&self.path, before)
        } else {
            fs::remove_file(&self.path)
        }
    }
}

pub(crate) fn commit(
    changes: &mut [Change<'_>],
    backup_dir: Option<&Path>,
) -> Result<(), ExportError> {
    let mut applied = 0;
    let result = (|| {
        for change in &mut *changes {
            change.apply()?;
            applied += 1;
        }
        for change in &*changes {
            if read_optional(&change.path)?.as_deref() != change.after {
                return Err(ExportError::Changed(change.path.display().to_string()));
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let failures: Vec<_> = changes[..applied]
            .iter()
            .rev()
            .filter_map(|change| {
                change
                    .restore()
                    .err()
                    .map(|e| format!("{}: {e}", change.path.display()))
            })
            .collect();
        if !failures.is_empty() {
            return Err(ExportError::Rollback {
                error: error.to_string(),
                files: failures.join(", "),
                backup: backup_dir.map_or_else(|| "none".into(), |p| p.display().to_string()),
            });
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_later_replacement_rolls_back_the_earlier_writes() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        fs::write(&first, b"before").unwrap();
        fs::write(&second, b"before").unwrap();
        let mut changes = [
            Change::prepare(first.clone(), Some(b"updated")).unwrap(),
            Change::prepare(second.clone(), Some(b"updated")).unwrap(),
        ];
        fs::write(&second, b"someone else edited this").unwrap();
        assert!(matches!(
            commit(&mut changes, None),
            Err(ExportError::Changed(_))
        ));
        assert_eq!(fs::read(first).unwrap(), b"before");
        assert_eq!(fs::read(second).unwrap(), b"someone else edited this");
    }
}
