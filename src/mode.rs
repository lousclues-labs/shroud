//! Runtime mode detection for Shroud.
//!
//! Shroud can run in two modes:
//! - Desktop: With tray icon, notifications, D-Bus session
//! - Headless: No GUI, systemd integration, server-optimized
//!
//! Mode is determined by:
//! 1. Explicit --headless flag (highest priority)
//! 2. Explicit --desktop flag
//! 3. Auto-detection based on environment

use log::{debug, info};
use std::env;

/// Runtime mode for Shroud
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Desktop mode with tray icon and notifications
    Desktop,
    /// Headless mode for servers, no GUI
    Headless,
}

impl std::fmt::Display for RuntimeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeMode::Desktop => write!(f, "desktop"),
            RuntimeMode::Headless => write!(f, "headless"),
        }
    }
}

/// Detect the appropriate runtime mode.
///
/// Priority:
/// 1. Explicit CLI flag (--headless or --desktop)
/// 2. Environment variable (SHROUD_MODE=headless|desktop)
/// 3. Auto-detection based on session type
pub fn detect_mode(cli_headless: bool, cli_desktop: bool) -> RuntimeMode {
    // Explicit flags take priority
    if cli_headless {
        info!("Mode: headless (explicit flag)");
        return RuntimeMode::Headless;
    }

    if cli_desktop {
        info!("Mode: desktop (explicit flag)");
        return RuntimeMode::Desktop;
    }

    // Check environment variable
    if let Ok(mode) = env::var("SHROUD_MODE") {
        match mode.to_lowercase().as_str() {
            "headless" => {
                info!("Mode: headless (SHROUD_MODE env)");
                return RuntimeMode::Headless;
            }
            "desktop" => {
                info!("Mode: desktop (SHROUD_MODE env)");
                return RuntimeMode::Desktop;
            }
            _ => {
                debug!("Unknown SHROUD_MODE value: {}, auto-detecting", mode);
            }
        }
    }

    // Auto-detect based on environment
    auto_detect_mode()
}

/// Auto-detect mode based on session environment.
fn auto_detect_mode() -> RuntimeMode {
    // Check for display server
    let has_display = env::var("DISPLAY").is_ok() || env::var("WAYLAND_DISPLAY").is_ok();

    // Check for desktop session
    let has_desktop_session =
        env::var("XDG_CURRENT_DESKTOP").is_ok() || env::var("DESKTOP_SESSION").is_ok();

    // Check if running as system service (no user session)
    let is_system_service = env::var("USER").map(|u| u == "root").unwrap_or(false) && !has_display;

    // Check for SSH session
    let is_ssh = env::var("SSH_CONNECTION").is_ok() || env::var("SSH_CLIENT").is_ok();

    // Check systemd invocation
    let is_systemd_service = env::var("INVOCATION_ID").is_ok();

    debug!(
        "Auto-detect: display={}, desktop_session={}, system_service={}, ssh={}, systemd={}",
        has_display, has_desktop_session, is_system_service, is_ssh, is_systemd_service
    );

    // Decision logic
    if is_system_service || is_systemd_service {
        info!("Mode: headless (detected system service)");
        RuntimeMode::Headless
    } else if is_ssh && !has_display {
        info!("Mode: headless (detected SSH without display)");
        RuntimeMode::Headless
    } else if has_display && has_desktop_session {
        info!("Mode: desktop (detected display + desktop session)");
        RuntimeMode::Desktop
    } else if has_display {
        info!("Mode: desktop (detected display)");
        RuntimeMode::Desktop
    } else {
        info!("Mode: headless (no display detected)");
        RuntimeMode::Headless
    }
}

/// Check if headless mode can run (basic requirements).
pub fn check_headless_requirements() -> Result<(), String> {
    // Check if we can access system config directory
    let config_dir = std::path::Path::new("/etc/shroud");
    if !config_dir.exists() {
        return Err(format!(
            "Headless mode requires /etc/shroud directory.\n\
             Create it with: sudo mkdir -p /etc/shroud"
        ));
    }

    Ok(())
}

/// Check if desktop mode can run (basic requirements).
pub fn check_desktop_requirements() -> Result<(), String> {
    // Check for display
    if env::var("DISPLAY").is_err() && env::var("WAYLAND_DISPLAY").is_err() {
        return Err(
            "Desktop mode requires a display (X11 or Wayland).\n\
             Use --headless for server environments."
                .to_string(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explicit_headless_flag() {
        assert_eq!(detect_mode(true, false), RuntimeMode::Headless);
    }

    #[test]
    fn test_explicit_desktop_flag() {
        assert_eq!(detect_mode(false, true), RuntimeMode::Desktop);
    }

    #[test]
    fn test_headless_takes_priority() {
        // If both flags somehow set, headless wins
        assert_eq!(detect_mode(true, true), RuntimeMode::Headless);
    }

    #[test]
    fn test_display_trait() {
        assert_eq!(format!("{}", RuntimeMode::Desktop), "desktop");
        assert_eq!(format!("{}", RuntimeMode::Headless), "headless");
    }
}
