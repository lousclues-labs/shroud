// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 loujr (lousclues)

//! XDG Autostart management for Shroud.
//!
//! Uses ~/.config/autostart/shroud.desktop to start on login.
//! This is preferred over systemd user services because:
//! - Runs after full desktop session is initialized
//! - PATH and environment are properly set
//! - Works consistently across desktop environments

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Autostart manager using XDG desktop files
pub struct Autostart;

impl Autostart {
    /// Get the path to the autostart desktop file
    fn desktop_file_path() -> Result<PathBuf, String> {
        dirs::config_dir()
            .map(|c| c.join("autostart/shroud.desktop"))
            .ok_or_else(|| "Could not determine XDG config directory".to_string())
    }

    /// Find the installed shroud binary with absolute path.
    ///
    /// SECURITY: Prefers system-wide paths over user-writable paths
    /// to prevent autostart entry from pointing at an attacker-controlled
    /// binary in ~/.cargo/bin (SHROUD-VULN-047).
    fn find_binary() -> Result<PathBuf, String> {
        // Check system-wide paths first (not user-writable)
        let system_candidates = [
            PathBuf::from("/usr/local/bin/shroud"),
            PathBuf::from("/usr/bin/shroud"),
        ];

        for candidate in &system_candidates {
            if candidate.exists() && is_executable(candidate) {
                return Ok(candidate.clone());
            }
        }

        // Then try current_exe (the actually running binary)
        if let Ok(exe) = std::env::current_exe() {
            if exe.exists() && !exe.to_string_lossy().contains(" (deleted)") {
                return Ok(exe);
            }
        }

        // Last resort: user-writable paths
        let user_candidates = [
            dirs::home_dir().map(|h| h.join(".local/bin/shroud")),
            dirs::home_dir().map(|h| h.join(".cargo/bin/shroud")),
        ];

        for candidate in user_candidates.into_iter().flatten() {
            if candidate.exists() && is_executable(&candidate) {
                return Ok(candidate);
            }
        }

        Err("Could not find shroud binary".to_string())
    }

    /// Generate desktop file content with absolute path
    fn generate_desktop_entry() -> Result<String, String> {
        let binary_path = Self::find_binary()?;

        Ok(format!(
            r#"[Desktop Entry]
Type=Application
Version=1.0
Name=Shroud VPN Manager
GenericName=VPN Manager
Comment=VPN connection manager with kill switch protection
Exec={}
Icon=network-vpn
Terminal=false
Categories=Network;System;Security;
Keywords=vpn;wireguard;privacy;killswitch;
StartupNotify=false
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=2
X-KDE-autostart-after=panel
"#,
            binary_path.display()
        ))
    }

    /// Check if autostart is enabled
    pub fn is_enabled() -> bool {
        Self::desktop_file_path()
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Get detailed status
    pub fn status() -> AutostartStatus {
        let desktop_file = Self::desktop_file_path().ok();
        let enabled = desktop_file.as_ref().map(|p| p.exists()).unwrap_or(false);
        let binary_path = Self::find_binary().ok();
        let binary_exists = binary_path.as_ref().map(|p| p.exists()).unwrap_or(false);

        let systemd_service_path =
            dirs::config_dir().map(|c| c.join("systemd/user/shroud.service"));
        let has_old_systemd = systemd_service_path
            .as_ref()
            .map(|p| p.exists())
            .unwrap_or(false);

        AutostartStatus {
            enabled,
            desktop_file,
            binary_path,
            binary_exists,
            has_old_systemd,
            systemd_service_path,
        }
    }

    /// Enable autostart
    pub fn enable() -> Result<(), String> {
        let path = Self::desktop_file_path()?;
        let content = Self::generate_desktop_entry()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create autostart directory: {}", e))?;
        }

        let _ = Self::cleanup_old_systemd();

        fs::write(&path, &content).map_err(|e| format!("Failed to write desktop file: {}", e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o755);
            let _ = fs::set_permissions(&path, perms);
        }

        Ok(())
    }

    /// Disable autostart
    pub fn disable() -> Result<(), String> {
        let path = Self::desktop_file_path()?;

        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("Failed to remove desktop file: {}", e))?;
        }

        Ok(())
    }

    /// Toggle autostart
    pub fn toggle() -> Result<bool, String> {
        if Self::is_enabled() {
            Self::disable()?;
            Ok(false)
        } else {
            Self::enable()?;
            Ok(true)
        }
    }

    /// Clean up old systemd user service
    pub fn cleanup_old_systemd() -> Result<Option<String>, String> {
        let service_path = dirs::config_dir()
            .map(|c| c.join("systemd/user/shroud.service"))
            .ok_or("Could not determine config directory")?;

        if !service_path.exists() {
            return Ok(None);
        }

        let _ = Command::new("systemctl")
            .args(["--user", "stop", "shroud"])
            .output();

        let _ = Command::new("systemctl")
            .args(["--user", "disable", "shroud"])
            .output();

        fs::remove_file(&service_path)
            .map_err(|e| format!("Failed to remove old service file: {}", e))?;

        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();

        Ok(Some(service_path.display().to_string()))
    }
}

/// Check if a file is executable
#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.exists()
}

/// Detailed autostart status
#[derive(Debug)]
pub struct AutostartStatus {
    pub enabled: bool,
    pub desktop_file: Option<PathBuf>,
    pub binary_path: Option<PathBuf>,
    pub binary_exists: bool,
    pub has_old_systemd: bool,
    pub systemd_service_path: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_desktop_file_path_returns_valid_path() {
        let path = Autostart::desktop_file_path();
        assert!(path.is_ok());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains("autostart"));
        assert!(path.to_string_lossy().ends_with("shroud.desktop"));
    }

    #[test]
    fn test_find_binary_returns_existing_path() {
        let result = Autostart::find_binary();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.is_absolute());
    }

    #[test]
    fn test_generate_desktop_entry_contains_required_fields() {
        let result = Autostart::generate_desktop_entry();
        assert!(result.is_ok());
        let content = result.unwrap();

        assert!(content.contains("[Desktop Entry]"));
        assert!(content.contains("Type=Application"));
        assert!(content.contains("Name=Shroud"));
        assert!(content.contains("Exec="));
        assert!(content.contains("Terminal=false"));

        for line in content.lines() {
            if let Some(exec_path) = line.strip_prefix("Exec=") {
                assert!(
                    exec_path.starts_with('/'),
                    "Exec path should be absolute: {}",
                    exec_path
                );
            }
        }
    }

    #[test]
    fn test_is_enabled_does_not_panic() {
        let _ = Autostart::is_enabled();
    }

    #[test]
    #[ignore = "requires XDG desktop environment - run with: cargo test -- --ignored"]
    fn test_enable_creates_desktop_file() {
        let result = Autostart::enable();
        assert!(result.is_ok(), "Enable failed: {:?}", result);

        assert!(Autostart::is_enabled());

        let path = Autostart::desktop_file_path().unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("[Desktop Entry]"));

        let _ = Autostart::disable();
    }

    #[test]
    #[ignore = "requires XDG desktop environment - run with: cargo test -- --ignored"]
    fn test_disable_removes_desktop_file() {
        Autostart::enable().unwrap();
        assert!(Autostart::is_enabled());

        let result = Autostart::disable();
        assert!(result.is_ok());
        assert!(!Autostart::is_enabled());
    }

    #[test]
    fn test_disable_succeeds_when_not_enabled() {
        let _ = Autostart::disable();

        let result = Autostart::disable();
        assert!(result.is_ok());
    }

    #[test]
    #[ignore = "requires XDG desktop environment - run with: cargo test -- --ignored"]
    fn test_toggle_enables_when_disabled() {
        let _ = Autostart::disable();
        assert!(!Autostart::is_enabled());

        let result = Autostart::toggle();
        assert!(result.is_ok());
        assert!(result.unwrap());
        assert!(Autostart::is_enabled());

        let _ = Autostart::disable();
    }

    #[test]
    #[ignore = "requires XDG desktop environment - run with: cargo test -- --ignored"]
    fn test_toggle_disables_when_enabled() {
        Autostart::enable().unwrap();
        assert!(Autostart::is_enabled());

        let result = Autostart::toggle();
        assert!(result.is_ok());
        assert!(!result.unwrap());
        assert!(!Autostart::is_enabled());
    }

    #[test]
    fn test_status_returns_valid_struct() {
        let status = Autostart::status();

        assert!(status.desktop_file.is_some());

        if let Some(ref path) = status.binary_path {
            assert_eq!(status.binary_exists, path.exists());
        }

        // Note: We don't check status.enabled == Autostart::is_enabled() here
        // because parallel tests may create/remove the desktop file, causing races.
        // The enabled status is tested separately in ignored tests.
    }

    #[test]
    fn test_cleanup_old_systemd_succeeds_when_no_service() {
        let result = Autostart::cleanup_old_systemd();
        assert!(result.is_ok());
    }

    #[test]
    fn test_enable_creates_parent_directory() {
        let path = Autostart::desktop_file_path().unwrap();
        let parent = path.parent().unwrap();

        let result = Autostart::enable();
        assert!(result.is_ok());
        assert!(parent.exists());

        let _ = Autostart::disable();
    }
}
