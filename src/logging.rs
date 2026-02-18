// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Logging configuration and runtime control
//!
//! Provides structured logging with multiple activation methods:
//! - Environment variable: RUST_LOG=debug
//! - Command-line flags: -v, --verbose, --log-level, --log-file
//! - Runtime toggle via tray menu
//!
//! Log files are written to ~/.local/share/shroud/ with proper permissions via `tracing` + `tracing-subscriber`.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::{filter_fn, EnvFilter};
use tracing_subscriber::{fmt, prelude::*};

/// Maximum log file size before rotation (10 MB)
const MAX_LOG_SIZE: u64 = 10 * 1024 * 1024;
/// Number of rotated log files to keep
const MAX_LOG_FILES: usize = 3;

/// Global flag for debug logging state (for tray menu)
static DEBUG_LOGGING_ENABLED: AtomicBool = AtomicBool::new(false);

/// Command-line arguments for logging configuration
#[derive(Debug, Clone, Default)]
pub struct Args {
    /// Verbosity level (0=info, 1=debug, 2+=trace)
    pub verbose: u8,
    /// Explicit log level override
    pub log_level: Option<String>,
    /// Log to file instead of stderr
    pub log_file: Option<PathBuf>,
}

/// Convert verbosity count to log level
pub fn verbose_to_level(verbose: u8) -> LevelFilter {
    match verbose {
        0 => LevelFilter::INFO,
        1 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    }
}

/// Parse log level string
pub fn parse_level(s: &str) -> Option<LevelFilter> {
    match s.to_lowercase().as_str() {
        "error" => Some(LevelFilter::ERROR),
        "warn" => Some(LevelFilter::WARN),
        "info" => Some(LevelFilter::INFO),
        "debug" => Some(LevelFilter::DEBUG),
        "trace" => Some(LevelFilter::TRACE),
        _ => None,
    }
}

/// Get the log directory path
pub fn log_directory() -> PathBuf {
    let data_home = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".local/share")
        });
    data_home.join("shroud")
}

/// Get the default log file path
pub fn default_log_path() -> PathBuf {
    log_directory().join("debug.log")
}

/// Ensure log directory exists with proper permissions.
pub fn ensure_log_directory() -> std::io::Result<PathBuf> {
    let dir = log_directory();
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(dir)
}

/// Rotating writer (size-based, keeps MAX_LOG_FILES)
struct RotatingWriter {
    path: PathBuf,
    file: File,
    bytes_written: u64,
}

impl RotatingWriter {
    fn new(path: PathBuf) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        // Create with restricted permissions from the start (no TOCTOU window)
        #[cfg(unix)]
        let file = {
            use std::os::unix::fs::OpenOptionsExt;
            OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&path)?
        };
        #[cfg(not(unix))]
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let bytes_written = file.metadata()?.len();
        Ok(Self {
            path,
            file,
            bytes_written,
        })
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        // Rename debug.log.(N-1) -> debug.log.N
        // Note: debug.log.{MAX_LOG_FILES} is overwritten by rename (not leaked).
        // Lowering MAX_LOG_FILES at compile time will orphan higher-numbered files.
        for i in (1..MAX_LOG_FILES).rev() {
            let from = self.path.with_extension(format!("log.{}", i));
            let to = self.path.with_extension(format!("log.{}", i + 1));
            let _ = fs::rename(from, to);
        }
        // Rename debug.log -> debug.log.1
        let rotated = self.path.with_extension("log.1");
        let _ = fs::rename(&self.path, rotated);
        // Reopen fresh file
        // Create with restricted permissions from the start (no TOCTOU window)
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            self.file = OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&self.path)?;
        }
        #[cfg(not(unix))]
        {
            self.file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
        }
        self.bytes_written = 0;
        Ok(())
    }

    fn write_inner(&mut self, buf: &[u8]) -> std::io::Result<()> {
        if self.bytes_written + buf.len() as u64 >= MAX_LOG_SIZE {
            let _ = self.rotate();
        }
        self.file.write_all(buf)?;
        self.bytes_written += buf.len() as u64;
        Ok(())
    }
}

/// MakeWriter that shares a RotatingWriter across threads
#[derive(Clone)]
struct RotatingMakeWriter(Arc<Mutex<RotatingWriter>>);
impl<'a> fmt::MakeWriter<'a> for RotatingMakeWriter {
    type Writer = RotatingWriterGuard;
    fn make_writer(&'a self) -> Self::Writer {
        RotatingWriterGuard(self.0.clone())
    }
}

struct RotatingWriterGuard(Arc<Mutex<RotatingWriter>>);
impl Write for RotatingWriterGuard {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut writer) = self.0.lock() {
            writer.write_inner(buf)?;
            Ok(buf.len())
        } else {
            // If poisoned, drop the log
            Ok(buf.len())
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if let Ok(mut writer) = self.0.lock() {
            writer.file.flush()
        } else {
            Ok(())
        }
    }
}

static ROTATING_WRITER: OnceLock<Arc<Mutex<RotatingWriter>>> = OnceLock::new();

fn get_rotating_writer(path: PathBuf) -> Arc<Mutex<RotatingWriter>> {
    ROTATING_WRITER
        .get_or_init(|| {
            Arc::new(Mutex::new(
                RotatingWriter::new(path).expect("failed to create log file"),
            ))
        })
        .clone()
}

/// Initialize logging with the given configuration
pub fn init_logging(args: &Args) {
    // Determine log level
    let level = if let Some(ref level_str) = args.log_level {
        parse_level(level_str).unwrap_or(LevelFilter::INFO)
    } else if args.verbose > 0 {
        verbose_to_level(args.verbose)
    } else {
        std::env::var("RUST_LOG")
            .ok()
            .and_then(|s| s.parse::<LevelFilter>().ok())
            .unwrap_or(LevelFilter::INFO)
    };

    // Base env filter
    let env_filter = EnvFilter::builder()
        .with_default_directive(level.into())
        .from_env_lossy();

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filter_fn(|meta| {
            if DEBUG_LOGGING_ENABLED.load(Ordering::Relaxed) {
                *meta.level() <= tracing::Level::WARN
            } else {
                true
            }
        }));

    // File logging disabled by default; toggled at runtime or via --log-file
    let log_path = args.log_file.clone().unwrap_or_else(default_log_path);
    let _ = ensure_log_directory();
    let rotating_mw = RotatingMakeWriter(get_rotating_writer(log_path));
    let file_layer = fmt::layer()
        .with_writer(rotating_mw)
        .with_filter(filter_fn(|_meta| {
            DEBUG_LOGGING_ENABLED.load(Ordering::Relaxed)
        }));

    if args.log_file.is_some() {
        DEBUG_LOGGING_ENABLED.store(true, Ordering::Relaxed);
    }

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();
}

/// Enable debug file logging (called from tray toggle)
pub fn enable_debug_logging() -> Result<PathBuf, String> {
    let path = default_log_path();
    DEBUG_LOGGING_ENABLED.store(true, Ordering::Relaxed);
    Ok(path)
}

/// Disable debug file logging (called from tray toggle)
pub fn disable_debug_logging() {
    DEBUG_LOGGING_ENABLED.store(false, Ordering::Relaxed);
}

/// Check if debug logging is enabled
pub fn is_debug_logging_enabled() -> bool {
    DEBUG_LOGGING_ENABLED.load(Ordering::Relaxed)
}

/// Open log file in default viewer
pub fn open_log_file() -> Result<(), String> {
    let path = default_log_path();
    if !path.exists() {
        return Err("No log file exists yet. Enable debug logging first.".to_string());
    }

    // Try common Linux file openers
    let openers = ["xdg-open", "kde-open", "gnome-open"];
    for opener in &openers {
        if std::process::Command::new(opener)
            .arg(&path)
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    Err(format!("Could not open log file: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verbose_to_level() {
        assert_eq!(verbose_to_level(0), LevelFilter::INFO);
        assert_eq!(verbose_to_level(1), LevelFilter::DEBUG);
        assert_eq!(verbose_to_level(2), LevelFilter::TRACE);
        assert_eq!(verbose_to_level(99), LevelFilter::TRACE);
    }

    #[test]
    fn test_parse_level() {
        assert_eq!(parse_level("error"), Some(LevelFilter::ERROR));
        assert_eq!(parse_level("warn"), Some(LevelFilter::WARN));
        assert_eq!(parse_level("info"), Some(LevelFilter::INFO));
        assert_eq!(parse_level("debug"), Some(LevelFilter::DEBUG));
        assert_eq!(parse_level("trace"), Some(LevelFilter::TRACE));
        assert_eq!(parse_level("invalid"), None);
    }

    #[test]
    fn test_log_directory_not_empty() {
        let dir = log_directory();
        assert!(!dir.to_string_lossy().is_empty());
    }

    #[test]
    fn test_get_log_file_path() {
        let path = default_log_path();
        assert!(path.to_string_lossy().ends_with(".log"));
    }

    #[test]
    fn test_debug_logging_toggle() {
        let initial = is_debug_logging_enabled();
        enable_debug_logging().unwrap();
        assert!(is_debug_logging_enabled());
        disable_debug_logging();
        assert_eq!(is_debug_logging_enabled(), initial);
    }
}
