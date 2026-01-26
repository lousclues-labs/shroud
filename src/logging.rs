//! Logging configuration and runtime control
//!
//! Provides structured logging with multiple activation methods:
//! - Environment variable: RUST_LOG=debug
//! - Command-line flags: -v, --verbose, --log-level, --log-file
//! - Runtime toggle via tray menu
//!
//! Log files are written to ~/.local/share/shroud/ with proper permissions.

use log::LevelFilter;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Maximum log file size before rotation (10 MB)
const MAX_LOG_SIZE: u64 = 10 * 1024 * 1024;

/// Number of rotated log files to keep
const MAX_LOG_FILES: usize = 3;

/// Global flag for debug logging state (for tray menu)
static DEBUG_LOGGING_ENABLED: AtomicBool = AtomicBool::new(false);

/// Shared file writer for runtime file logging
static FILE_WRITER: Mutex<Option<FileWriter>> = Mutex::new(None);

/// Command-line arguments for Shroud
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct Args {
    /// Verbosity level (0=info, 1=debug, 2+=trace)
    pub verbose: u8,
    /// Explicit log level override
    pub log_level: Option<String>,
    /// Log to file instead of stderr
    pub log_file: Option<PathBuf>,
    /// Show help
    pub help: bool,
    /// Show version
    pub version: bool,
}

/// Parse command-line arguments
/// Note: This function is replaced by cli::parse_args() but kept for backward compatibility
#[allow(dead_code)]
pub fn parse_args() -> Args {
    let mut args = Args::default();
    let mut argv = std::env::args().skip(1).peekable();

    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "-v" | "--verbose" => args.verbose = args.verbose.saturating_add(1),
            "-vv" => args.verbose = args.verbose.saturating_add(2),
            "--log-level" => {
                args.log_level = argv.next();
            }
            "--log-file" => {
                args.log_file = argv.next().map(PathBuf::from);
            }
            "-h" | "--help" => args.help = true,
            "-V" | "--version" => args.version = true,
            other => {
                // Check for combined short flags like -vvv
                if other.starts_with('-') && !other.starts_with("--") {
                    for c in other.chars().skip(1) {
                        if c == 'v' {
                            args.verbose = args.verbose.saturating_add(1);
                        }
                    }
                } else {
                    eprintln!("Unknown argument: {}", other);
                    eprintln!("Try 'shroud --help' for usage information.");
                    std::process::exit(1);
                }
            }
        }
    }

    args
}

/// Print help message
/// Note: This function is replaced by cli::help::print_main_help() but kept for backward compatibility
#[allow(dead_code)]
pub fn print_help() {
    println!(
        r#"Shroud - A provider-agnostic VPN connection manager for Linux

USAGE:
    shroud [OPTIONS]

OPTIONS:
    -v, --verbose       Increase logging verbosity (use -vv for trace)
    --log-level LEVEL   Set log level (error, warn, info, debug, trace)
    --log-file PATH     Write logs to file instead of stderr
    -h, --help          Print this help message
    -V, --version       Print version information

ENVIRONMENT:
    RUST_LOG            Set log level (e.g., RUST_LOG=debug)
                        Supports module-specific levels:
                        RUST_LOG=shroud::killswitch=trace,shroud::dbus=debug

EXAMPLES:
    shroud                          # Normal operation (info level)
    shroud -v                       # Debug logging to stderr
    shroud -vv                      # Trace logging to stderr
    shroud --log-file ~/shroud.log  # Log to file
    RUST_LOG=debug shroud           # Debug via environment

For more information, visit: https://github.com/loujr/shroud"#
    );
}

/// Convert verbosity count to log level
pub fn verbose_to_level(verbose: u8) -> LevelFilter {
    match verbose {
        0 => LevelFilter::Info,
        1 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    }
}

/// Parse log level string
pub fn parse_level(s: &str) -> Option<LevelFilter> {
    match s.to_lowercase().as_str() {
        "error" => Some(LevelFilter::Error),
        "warn" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
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

/// Ensure log directory exists with proper permissions
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

/// File writer with rotation support
struct FileWriter {
    path: PathBuf,
    file: File,
    bytes_written: u64,
}

impl FileWriter {
    fn new(path: PathBuf) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }

        let bytes_written = file.metadata()?.len();

        Ok(Self {
            path,
            file,
            bytes_written,
        })
    }

    fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.file.write_all(data)?;
        self.bytes_written += data.len() as u64;

        // Check if rotation is needed
        if self.bytes_written >= MAX_LOG_SIZE {
            self.rotate()?;
        }

        Ok(())
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        // Close current file
        self.file.sync_all()?;

        // Rotate existing files
        for i in (1..MAX_LOG_FILES).rev() {
            let old_path = self.path.with_extension(format!("log.{}", i));
            let new_path = self.path.with_extension(format!("log.{}", i + 1));
            if old_path.exists() {
                if i + 1 >= MAX_LOG_FILES {
                    fs::remove_file(&old_path)?;
                } else {
                    fs::rename(&old_path, &new_path)?;
                }
            }
        }

        // Move current to .1
        let rotated = self.path.with_extension("log.1");
        fs::rename(&self.path, &rotated)?;

        // Create new file
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
        }

        self.bytes_written = 0;

        Ok(())
    }
}

/// Custom logger that supports runtime file logging toggle
struct ShroudLogger {
    default_level: LevelFilter,
}

impl log::Log for ShroudLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        let level = if DEBUG_LOGGING_ENABLED.load(Ordering::Relaxed) {
            LevelFilter::Debug
        } else {
            self.default_level
        };
        metadata.level() <= level
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let now = chrono_lite_timestamp();
        let level = record.level();
        let target = record.target();
        let message = record.args();

        let line = format!("[{}] [{:5}] [{}] {}\n", now, level, target, message);

        // Write to file if enabled
        if DEBUG_LOGGING_ENABLED.load(Ordering::Relaxed) {
            if let Ok(mut guard) = FILE_WRITER.lock() {
                if let Some(ref mut writer) = *guard {
                    let _ = writer.write(line.as_bytes());
                }
            }
        }

        // Always write to stderr for error/warn, or if not file logging
        if level <= log::Level::Warn || !DEBUG_LOGGING_ENABLED.load(Ordering::Relaxed) {
            eprint!("{}", line);
        }
    }

    fn flush(&self) {
        if let Ok(guard) = FILE_WRITER.lock() {
            if let Some(ref writer) = *guard {
                let _ = writer.file.sync_all();
            }
        }
    }
}

/// Simple timestamp without external dependency
fn chrono_lite_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs();
    let millis = now.subsec_millis();

    // Convert to approximate UTC time components
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Approximate year/month/day (good enough for logging)
    let years = 1970 + days / 365;
    let day_of_year = days % 365;
    let month = (day_of_year / 30).min(11) + 1;
    let day = (day_of_year % 30) + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        years, month, day, hours, minutes, seconds, millis
    )
}

/// Initialize logging with the given configuration
pub fn init_logging(args: &Args) {
    // Determine log level
    let level = if let Some(ref level_str) = args.log_level {
        parse_level(level_str).unwrap_or(LevelFilter::Info)
    } else if args.verbose > 0 {
        verbose_to_level(args.verbose)
    } else {
        // Check RUST_LOG environment variable
        std::env::var("RUST_LOG")
            .ok()
            .and_then(|s| parse_level(&s))
            .unwrap_or(LevelFilter::Info)
    };

    // If log file specified, set up file logging immediately
    if let Some(ref path) = args.log_file {
        if let Err(e) = enable_file_logging_internal(path) {
            eprintln!("Warning: Failed to open log file: {}", e);
        } else {
            DEBUG_LOGGING_ENABLED.store(true, Ordering::Relaxed);
        }
    }

    // Set up the logger
    let logger = ShroudLogger {
        default_level: level,
    };

    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(LevelFilter::Trace); // Allow runtime level changes
    }
}

/// Enable debug file logging (called from tray toggle)
pub fn enable_debug_logging() -> Result<PathBuf, String> {
    let path = default_log_path();
    enable_file_logging_internal(&path).map_err(|e| e.to_string())?;
    DEBUG_LOGGING_ENABLED.store(true, Ordering::Relaxed);
    Ok(path)
}

/// Disable debug file logging (called from tray toggle)
pub fn disable_debug_logging() {
    DEBUG_LOGGING_ENABLED.store(false, Ordering::Relaxed);
    if let Ok(mut guard) = FILE_WRITER.lock() {
        *guard = None;
    }
}

/// Check if debug logging is enabled
pub fn is_debug_logging_enabled() -> bool {
    DEBUG_LOGGING_ENABLED.load(Ordering::Relaxed)
}

/// Internal function to enable file logging
fn enable_file_logging_internal(path: &Path) -> std::io::Result<()> {
    ensure_log_directory()?;
    let writer = FileWriter::new(path.to_path_buf())?;
    if let Ok(mut guard) = FILE_WRITER.lock() {
        *guard = Some(writer);
    }
    Ok(())
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
        assert_eq!(verbose_to_level(0), LevelFilter::Info);
        assert_eq!(verbose_to_level(1), LevelFilter::Debug);
        assert_eq!(verbose_to_level(2), LevelFilter::Trace);
        assert_eq!(verbose_to_level(99), LevelFilter::Trace);
    }

    #[test]
    fn test_parse_level() {
        assert_eq!(parse_level("error"), Some(LevelFilter::Error));
        assert_eq!(parse_level("WARN"), Some(LevelFilter::Warn));
        assert_eq!(parse_level("Info"), Some(LevelFilter::Info));
        assert_eq!(parse_level("debug"), Some(LevelFilter::Debug));
        assert_eq!(parse_level("trace"), Some(LevelFilter::Trace));
        assert_eq!(parse_level("invalid"), None);
    }

    #[test]
    fn test_parse_args_verbose() {
        // Test is limited since we can't easily mock std::env::args
        let args = Args::default();
        assert_eq!(args.verbose, 0);
        assert!(args.log_file.is_none());
    }

    #[test]
    fn test_log_directory() {
        let dir = log_directory();
        assert!(dir.to_string_lossy().contains("shroud"));
    }
}
