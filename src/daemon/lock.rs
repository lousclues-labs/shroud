// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Instance lock management
//!
//! Provides file-based locking to ensure only one Shroud daemon runs at a time.
//! Uses flock() for advisory locking on a file in XDG_RUNTIME_DIR.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use tracing::{info, warn};

/// Get the path to the lock file
pub fn get_lock_file_path() -> PathBuf {
    // HARDENING: Use fallback instead of panic if XDG_RUNTIME_DIR not set
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        // Fall back to /tmp with uid-based path for isolation
        let uid = unsafe { libc::getuid() };
        format!("/tmp/shroud-{}", uid)
    });

    let path = PathBuf::from(&runtime_dir);

    // Ensure directory exists
    if !path.exists() {
        let _ = std::fs::create_dir_all(&path);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
        }
    }

    path.join("shroud.lock")
}

/// Check if a process with the given PID is still running
fn is_process_running(pid: u32) -> bool {
    if pid == 0 {
        return false; // PID 0 = kernel scheduler, never a shroud instance
    }
    // Use kill(pid, 0) to check if process exists
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        return true;
    }
    // ESRCH means process doesn't exist
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// Acquire an exclusive instance lock
///
/// Returns the lock file handle on success. The lock is held as long as
/// the file handle exists. Returns an error if another instance is running.
pub fn acquire_instance_lock() -> Result<File, String> {
    acquire_instance_lock_inner(1) // allow one retry for stale locks
}

fn acquire_instance_lock_inner(retries_left: u32) -> Result<File, String> {
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
            // Lock is held - but check if the holding process is actually running
            let mut contents = String::new();
            if let Ok(mut f) = File::open(&lock_path) {
                let _ = f.read_to_string(&mut contents);
            }

            if let Ok(pid) = contents.trim().parse::<u32>() {
                if !is_process_running(pid) {
                    if retries_left == 0 {
                        return Err("Stale lock file persists after retry".to_string());
                    }
                    // Process is dead - this is a stale lock!
                    warn!("Stale lock file from dead process (PID {}), removing", pid);
                    drop(file); // Release our non-exclusive handle

                    // Remove the stale lock file and retry
                    if let Err(e) = fs::remove_file(&lock_path) {
                        return Err(format!("Failed to remove stale lock: {}", e));
                    }

                    // Retry with decremented counter
                    return acquire_instance_lock_inner(retries_left - 1);
                }
                return Err(format!("Another instance is already running (PID {})", pid));
            }

            return Err("Another instance is already running".to_string());
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
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn with_temp_runtime_dir<F, R>(f: F) -> R
    where
        F: FnOnce(&std::path::Path) -> R,
    {
        let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let temp = TempDir::new().unwrap();
        let prev = env::var("XDG_RUNTIME_DIR").ok();
        env::set_var("XDG_RUNTIME_DIR", temp.path());
        let result = f(temp.path());
        if let Some(val) = prev {
            env::set_var("XDG_RUNTIME_DIR", val);
        } else {
            env::remove_var("XDG_RUNTIME_DIR");
        }
        result
    }

    #[test]
    fn test_lock_file_path_uses_runtime_dir() {
        // This test only works if XDG_RUNTIME_DIR is set
        if env::var("XDG_RUNTIME_DIR").is_ok() {
            let path = get_lock_file_path();
            assert!(path.to_string_lossy().contains("shroud.lock"));
        }
    }

    #[test]
    fn test_acquire_lock_writes_pid() {
        with_temp_runtime_dir(|_| {
            let result = acquire_instance_lock();
            if let Ok(file) = result {
                let path = get_lock_file_path();
                let content = std::fs::read_to_string(&path).unwrap();
                let pid: u32 = content.trim().parse().unwrap();
                assert_eq!(pid, std::process::id());

                drop(file);
                release_instance_lock();
            }
        });
    }

    #[test]
    fn test_release_lock_removes_file() {
        with_temp_runtime_dir(|_| {
            if let Ok(file) = acquire_instance_lock() {
                let path = get_lock_file_path();
                assert!(path.exists());

                drop(file);
                release_instance_lock();

                assert!(!path.exists());
            }
        });
    }

    #[test]
    fn test_release_lock_idempotent() {
        with_temp_runtime_dir(|_| {
            release_instance_lock();
            release_instance_lock();
        });
    }

    #[test]
    fn test_lock_conflict_detection() {
        with_temp_runtime_dir(|_| {
            #[cfg(unix)]
            unsafe {
                let pid = libc::fork();
                if pid == 0 {
                    let _lock = acquire_instance_lock().expect("child should acquire lock");
                    std::thread::sleep(Duration::from_secs(2));
                    std::process::exit(0);
                } else {
                    let start = Instant::now();
                    let pid_str = pid.to_string();
                    while start.elapsed() < Duration::from_secs(1) {
                        if let Ok(contents) = std::fs::read_to_string(get_lock_file_path()) {
                            if contents.trim() == pid_str {
                                break;
                            }
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }

                    let second = acquire_instance_lock();
                    assert!(second.is_err());

                    let err = second.unwrap_err();
                    assert!(err.contains("Another instance is already running"));

                    let _ = libc::waitpid(pid, std::ptr::null_mut(), 0);
                    release_instance_lock();
                }
            }

            #[cfg(not(unix))]
            {
                // No reliable way to test flock conflicts on non-Unix platforms.
                assert!(true);
            }
        });
    }

    #[test]
    fn test_lock_file_permissions() {
        with_temp_runtime_dir(|_| {
            if let Ok(file) = acquire_instance_lock() {
                let path = get_lock_file_path();
                let metadata = std::fs::metadata(&path).unwrap();

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = metadata.permissions().mode();
                    assert_eq!(mode & 0o777, 0o600);
                }

                drop(file);
                release_instance_lock();
            }
        });
    }
}
