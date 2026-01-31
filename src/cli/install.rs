//! Atomic binary installation helpers for CLI update/restart flows.

use log::{debug, info};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Error type for installation failures.
#[derive(Debug)]
pub enum InstallError {
    /// Failed to copy to temporary location.
    TempCopy {
        source: PathBuf,
        temp: PathBuf,
        error: io::Error,
    },
    /// Failed to set executable permissions.
    Permissions { path: PathBuf, error: io::Error },
    /// Failed to perform atomic rename.
    Rename {
        temp: PathBuf,
        dest: PathBuf,
        error: io::Error,
    },
    /// Failed to clean up temp file after error.
    Cleanup { path: PathBuf, error: io::Error },
    /// Source file does not exist.
    SourceNotFound { path: PathBuf },
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::TempCopy {
                source,
                temp,
                error,
            } => write!(
                f,
                "Failed to copy {} to {}: {}",
                source.display(),
                temp.display(),
                error
            ),
            InstallError::Permissions { path, error } => {
                write!(
                    f,
                    "Failed to set permissions on {}: {}",
                    path.display(),
                    error
                )
            }
            InstallError::Rename { temp, dest, error } => write!(
                f,
                "Failed to rename {} to {}: {}",
                temp.display(),
                dest.display(),
                error
            ),
            InstallError::Cleanup { path, error } => {
                write!(
                    f,
                    "Failed to clean up temp file {}: {}",
                    path.display(),
                    error
                )
            }
            InstallError::SourceNotFound { path } => {
                write!(f, "Source binary not found: {}", path.display())
            }
        }
    }
}

impl std::error::Error for InstallError {}

/// Install a binary using atomic rename to avoid "file busy" errors.
pub fn install_binary_atomic(source: &Path, dest: &Path) -> Result<(), InstallError> {
    if !source.exists() {
        return Err(InstallError::SourceNotFound {
            path: source.to_path_buf(),
        });
    }

    let temp_dest = dest.with_file_name(".shroud.new");
    debug!(
        "Installing binary: {} -> {} (via {})",
        source.display(),
        dest.display(),
        temp_dest.display()
    );

    fs::copy(source, &temp_dest).map_err(|e| InstallError::TempCopy {
        source: source.to_path_buf(),
        temp: temp_dest.clone(),
        error: e,
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(&temp_dest).map_err(|e| {
            let _ = fs::remove_file(&temp_dest);
            InstallError::Permissions {
                path: temp_dest.clone(),
                error: e,
            }
        })?;

        let mut perms = metadata.permissions();
        perms.set_mode(0o755);

        fs::set_permissions(&temp_dest, perms).map_err(|e| {
            let _ = fs::remove_file(&temp_dest);
            InstallError::Permissions {
                path: temp_dest.clone(),
                error: e,
            }
        })?;
    }

    fs::rename(&temp_dest, dest).map_err(|e| {
        let cleanup = fs::remove_file(&temp_dest).map_err(|cleanup_error| InstallError::Cleanup {
            path: temp_dest.clone(),
            error: cleanup_error,
        });

        if let Err(cleanup_error) = cleanup {
            return cleanup_error;
        }

        InstallError::Rename {
            temp: temp_dest.clone(),
            dest: dest.to_path_buf(),
            error: e,
        }
    })?;

    info!("Successfully installed binary to {}", dest.display());
    Ok(())
}

/// Check if a path is the same file as another (by inode).
#[cfg(unix)]
pub fn is_same_file(path1: &Path, path2: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    match (fs::metadata(path1), fs::metadata(path2)) {
        (Ok(m1), Ok(m2)) => m1.ino() == m2.ino() && m1.dev() == m2.dev(),
        _ => false,
    }
}

#[cfg(not(unix))]
pub fn is_same_file(_path1: &Path, _path2: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_install_binary_atomic_basic() {
        let temp_dir = TempDir::new().unwrap();

        let source = temp_dir.path().join("source_binary");
        let mut f = File::create(&source).unwrap();
        f.write_all(b"#!/bin/bash\necho hello").unwrap();

        let dest = temp_dir.path().join("dest_binary");

        install_binary_atomic(&source, &dest).unwrap();

        assert!(dest.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&dest).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o755);
        }

        let source_content = fs::read(&source).unwrap();
        let dest_content = fs::read(&dest).unwrap();
        assert_eq!(source_content, dest_content);
    }

    #[test]
    fn test_install_binary_atomic_overwrites_existing() {
        let temp_dir = TempDir::new().unwrap();

        let source = temp_dir.path().join("new_binary");
        fs::write(&source, b"NEW CONTENT").unwrap();

        let dest = temp_dir.path().join("old_binary");
        fs::write(&dest, b"OLD CONTENT").unwrap();

        install_binary_atomic(&source, &dest).unwrap();

        let content = fs::read_to_string(&dest).unwrap();
        assert_eq!(content, "NEW CONTENT");
    }

    #[test]
    fn test_install_binary_atomic_source_not_found() {
        let temp_dir = TempDir::new().unwrap();

        let source = temp_dir.path().join("nonexistent");
        let dest = temp_dir.path().join("dest");

        let result = install_binary_atomic(&source, &dest);

        assert!(matches!(result, Err(InstallError::SourceNotFound { .. })));
    }

    #[test]
    fn test_install_binary_atomic_cleans_temp_on_failure() {
        let temp_dir = TempDir::new().unwrap();

        let source = temp_dir.path().join("source");
        fs::write(&source, b"content").unwrap();

        let dest = temp_dir.path().join("dest_dir");
        fs::create_dir(&dest).unwrap();

        let result = install_binary_atomic(&source, &dest);

        assert!(result.is_err());

        let temp_path = dest.with_file_name(".shroud.new");
        assert!(
            !temp_path.exists(),
            "Temp file should be cleaned up on failure"
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_install_over_open_file() {
        use std::os::unix::io::AsRawFd;

        let temp_dir = TempDir::new().unwrap();

        let dest = temp_dir.path().join("binary");
        let file = File::create(&dest).unwrap();
        let _fd = file.as_raw_fd();

        let source = temp_dir.path().join("new_binary");
        fs::write(&source, b"NEW VERSION").unwrap();

        let result = install_binary_atomic(&source, &dest);

        assert!(result.is_ok(), "Atomic rename should work on open file");

        let content = fs::read_to_string(&dest).unwrap();
        assert_eq!(content, "NEW VERSION");
    }

    #[test]
    #[cfg(unix)]
    fn test_is_same_file() {
        let temp_dir = TempDir::new().unwrap();

        let file1 = temp_dir.path().join("file1");
        let file2 = temp_dir.path().join("file2");

        fs::write(&file1, b"content").unwrap();
        fs::write(&file2, b"content").unwrap();

        assert!(is_same_file(&file1, &file1));
        assert!(!is_same_file(&file1, &file2));

        let link = temp_dir.path().join("link");
        std::os::unix::fs::symlink(&file1, &link).unwrap();
        assert!(is_same_file(&file1, &link));
    }
}
