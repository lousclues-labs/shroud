//! Instance lock management
//!
//! Provides file-based locking to ensure only one Shroud daemon runs at a time.
//! Uses flock() for advisory locking on a file in XDG_RUNTIME_DIR.

use log::{info, warn};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

/// Get the path to the lock file
pub fn get_lock_file_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .expect("XDG_RUNTIME_DIR not set - cannot safely create lock file");
    PathBuf::from(runtime_dir).join("shroud.lock")
}

/// Acquire an exclusive instance lock
///
/// Returns the lock file handle on success. The lock is held as long as
/// the file handle exists. Returns an error if another instance is running.
pub fn acquire_instance_lock() -> Result<File, String> {
    let lock_path = get_lock_file_path();

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| format!("Failed to open lock file: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600));
    }

    let fd = file.as_raw_fd();
    let result = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };

    if result != 0 {
        let errno = std::io::Error::last_os_error();
        if errno.raw_os_error() == Some(libc::EWOULDBLOCK) {
            let mut contents = String::new();
            if let Ok(mut f) = File::open(&lock_path) {
                let _ = f.read_to_string(&mut contents);
            }
            let pid_info = contents
                .trim()
                .parse::<u32>()
                .map(|pid| format!(" (PID {})", pid))
                .unwrap_or_default();
            return Err(format!("Another instance is already running{}", pid_info));
        }
        return Err(format!("Failed to acquire lock: {}", errno));
    }

    let truncate_result = unsafe { libc::ftruncate(fd, 0) };
    if truncate_result != 0 {
        return Err(format!(
            "Failed to truncate lock file: {}",
            std::io::Error::last_os_error()
        ));
    }

    use std::io::Seek;
    let mut file = file;
    file.seek(std::io::SeekFrom::Start(0))
        .map_err(|e| format!("Failed to seek: {}", e))?;
    write!(file, "{}", std::process::id()).map_err(|e| format!("Failed to write PID: {}", e))?;
    file.sync_all()
        .map_err(|e| format!("Failed to sync: {}", e))?;

    info!("Acquired instance lock (PID {})", std::process::id());
    Ok(file)
}

/// Release the instance lock by removing the lock file
pub fn release_instance_lock() {
    let lock_path = get_lock_file_path();
    if let Err(e) = fs::remove_file(&lock_path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            warn!("Failed to remove lock file: {}", e);
        }
    } else {
        info!("Released instance lock");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_lock_file_path_uses_runtime_dir() {
        // This test only works if XDG_RUNTIME_DIR is set
        if env::var("XDG_RUNTIME_DIR").is_ok() {
            let path = get_lock_file_path();
            assert!(path.to_string_lossy().contains("shroud.lock"));
        }
    }
}
