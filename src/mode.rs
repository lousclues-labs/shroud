// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Runtime mode detection for Shroud.
//!
//! Shroud can run in two modes:
//! - Desktop: With tray icon, notifications, D-Bus session (ALWAYS DEFAULT)
//! - Headless: No GUI, systemd integration, server-optimized (EXPLICIT ONLY)
//!
//! DESIGN PRINCIPLE: Desktop is ALWAYS the default.
//! Headless mode requires EXPLICIT opt-in via:
//! 1. --headless flag
//! 2. SHROUD_MODE=headless environment variable
//!
//! We do NOT auto-detect headless based on missing DISPLAY, SSH sessions,
//! or any other heuristics. This prevents accidental mode switches that
//! could break user workflows.

use std::env;
use tracing::info;

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
/// EXPLICIT OPT-IN ONLY:
/// - --headless flag → Headless
/// - SHROUD_MODE=headless → Headless  
/// - Everything else → Desktop (safe default)
///
/// --desktop flag is accepted but redundant (desktop is always default)
pub fn detect_mode(cli_headless: bool, cli_desktop: bool) -> RuntimeMode {
    let mode = if cli_headless {
        info!("Mode: headless (--headless flag)");
        RuntimeMode::Headless
    } else if cli_desktop {
        info!("Mode: desktop (--desktop flag)");
        RuntimeMode::Desktop
    } else if let Ok(mode) = env::var("SHROUD_MODE") {
        match mode.to_lowercase().as_str() {
            "headless" => {
                info!("Mode: headless (SHROUD_MODE=headless)");
                RuntimeMode::Headless
            }
            "desktop" => {
                info!("Mode: desktop (SHROUD_MODE=desktop)");
                RuntimeMode::Desktop
            }
            _ => {
                // Unknown value - warn but use default
                eprintln!("Warning: Unknown SHROUD_MODE='{}', using desktop", mode);
                info!("Mode: desktop (default)");
                RuntimeMode::Desktop
            }
        }
    } else {
        info!("Mode: desktop (default)");
        RuntimeMode::Desktop
    };

    // Validate requirements for the selected mode
    let validation_result = match mode {
        RuntimeMode::Headless => check_headless_requirements(),
        RuntimeMode::Desktop => check_desktop_requirements(),
    };
    if let Err(warning) = validation_result {
        eprintln!("Warning: {}", warning);
    }

    mode
}

/// Check if headless mode can run (basic requirements).
pub fn check_headless_requirements() -> Result<(), String> {
    // Check if we can access system config directory
    let config_dir = std::path::Path::new("/etc/shroud");
    if !config_dir.exists() {
        return Err("Headless mode requires /etc/shroud directory.\n\
             Create it with: sudo mkdir -p /etc/shroud"
            .to_string());
    }

    Ok(())
}

/// Check if desktop mode can run (basic requirements).
pub fn check_desktop_requirements() -> Result<(), String> {
    // Check for display
    if env::var("DISPLAY").is_err() && env::var("WAYLAND_DISPLAY").is_err() {
        return Err("Desktop mode requires a display (X11 or Wayland).\n\
              Use --headless for server environments."
            .to_string());
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
        // If both flags somehow set, headless wins (it's checked first)
        assert_eq!(detect_mode(true, true), RuntimeMode::Headless);
    }

    #[test]
    fn test_default_is_always_desktop() {
        // No flags = desktop (the safe default)
        assert_eq!(detect_mode(false, false), RuntimeMode::Desktop);
    }

    #[test]
    fn test_display_trait() {
        assert_eq!(format!("{}", RuntimeMode::Desktop), "desktop");
        assert_eq!(format!("{}", RuntimeMode::Headless), "headless");
    }
}
