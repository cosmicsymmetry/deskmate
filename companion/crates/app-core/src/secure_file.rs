use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use atomic_write_file::AtomicWriteFile;
#[cfg(unix)]
use atomic_write_file::unix::OpenOptionsExt as AtomicOpenOptionsExt;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as UnixOpenOptionsExt, PermissionsExt};

#[derive(Debug)]
pub(crate) enum BoundedReadError {
    Io(FileIoError),
    TooLarge { maximum: usize },
}

#[derive(Debug)]
pub(crate) struct FileIoError {
    pub operation: FileOperation,
    pub source: io::Error,
}

impl FileIoError {
    pub fn into_strings(self, subject: &str) -> (String, String) {
        (self.operation.describe(subject), self.source.to_string())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FileOperation {
    CreateDirectory,
    Open,
    Read,
    CreateTemporary,
    WriteTemporary,
    FinishTemporary,
    SyncAndReplace,
    SyncDirectory,
}

impl FileOperation {
    pub fn describe(self, subject: &str) -> String {
        let action = match self {
            Self::CreateDirectory => "create",
            Self::Open => "open",
            Self::Read => "read",
            Self::CreateTemporary => "create temporary",
            Self::WriteTemporary => "write temporary",
            Self::FinishTemporary => "finish temporary",
            Self::SyncAndReplace => "sync and replace",
            Self::SyncDirectory => "sync",
        };
        format!("{action} {subject}")
    }
}

pub(crate) fn read_bounded(
    path: &Path,
    maximum: usize,
) -> Result<Option<Vec<u8>>, BoundedReadError> {
    read_bounded_with_mode_repair(path, maximum, secure_open)
}

pub(crate) fn read_bounded_with_mode_repair(
    path: &Path,
    maximum: usize,
    repair_mode: impl FnOnce(&File) -> io::Result<()>,
) -> Result<Option<Vec<u8>>, BoundedReadError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(BoundedReadError::Io(FileIoError {
                operation: FileOperation::Open,
                source: error,
            }));
        }
    };
    // Permission repair is defense in depth. A readable, valid file must still
    // load if chmod is unavailable, for example on a read-only restored volume.
    let _ = repair_mode(&file);
    let mut bytes = Vec::new();
    file.take((maximum + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| {
            BoundedReadError::Io(FileIoError {
                operation: FileOperation::Read,
                source,
            })
        })?;
    if bytes.len() > maximum {
        Err(BoundedReadError::TooLarge { maximum })
    } else {
        Ok(Some(bytes))
    }
}

pub(crate) fn usable_parent(path: &Path) -> Option<&Path> {
    let parent = path.parent()?;
    if parent.as_os_str().is_empty() {
        Some(Path::new("."))
    } else {
        Some(parent)
    }
}

pub(crate) fn create_directory(path: &Path) -> Result<(), FileIoError> {
    fs::create_dir_all(path).map_err(|source| FileIoError {
        operation: FileOperation::CreateDirectory,
        source,
    })
}

pub(crate) fn write_and_replace(target: &Path, bytes: &[u8]) -> Result<(), FileIoError> {
    let mut options = AtomicWriteFile::options();
    secure_atomic_options(&mut options);
    let mut file = options.open(target).map_err(|source| FileIoError {
        operation: FileOperation::CreateTemporary,
        source,
    })?;
    file.write_all(bytes).map_err(|source| FileIoError {
        operation: FileOperation::WriteTemporary,
        source,
    })?;
    file.write_all(b"\n").map_err(|source| FileIoError {
        operation: FileOperation::FinishTemporary,
        source,
    })?;
    file.commit().map_err(|source| FileIoError {
        operation: FileOperation::SyncAndReplace,
        source,
    })
}

#[cfg(unix)]
fn secure_open(file: &File) -> io::Result<()> {
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn secure_open(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_atomic_options(options: &mut atomic_write_file::OpenOptions) {
    options.preserve_mode(false);
    options.mode(0o600);
}

#[cfg(not(unix))]
fn secure_atomic_options(_options: &mut atomic_write_file::OpenOptions) {}

#[cfg(unix)]
pub(crate) fn sync_parent(parent: &Path) -> Result<(), FileIoError> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| FileIoError {
            operation: FileOperation::SyncDirectory,
            source,
        })
}

#[cfg(not(unix))]
pub(crate) fn sync_parent(_parent: &Path) -> Result<(), FileIoError> {
    Ok(())
}
