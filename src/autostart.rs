// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! XDG Autostart management for Shroud.
//!
//! Uses ~/.config/autostart/shroud.desktop to start on login.
//! This is preferred over systemd user services because:
//! - Runs after full desktop session is initialized
//! - PATH and environment are properly set
//! - Works consistently across desktop environments
//!
//! ## Restart policy
//!
//! `systemd-xdg-autostart-generator` turns the desktop file into a transient
//! `app-shroud@autostart.service` unit, and that generated unit always carries
//! `Restart=no`. For a process whose job is enforcing a kill switch, dying
//! unsupervised is the worst failure mode: the firewall rules persist but
//! nothing is left to manage state, reconnect, or clean up.
//!
//! Rather than abandoning XDG autostart (and the desktop-environment
//! compatibility above), we keep it as the activation mechanism and layer a
//! systemd drop-in on top of the generated unit to supply the missing restart
//! policy. Drop-ins under `~/.config/systemd/user/` take precedence over
//! generator output, so this works without owning the unit file itself.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Drop-in contents supplying the restart policy the XDG generator omits.
///
/// `on-failure` deliberately does not restart after a clean exit, so quitting
/// from the tray still quits. The start-limit pair prevents a crash-looping
/// binary from being respawned forever.
const RESTART_DROPIN: &str = r#"# Managed by shroud — do not edit.
#
# systemd-xdg-autostart-generator emits Restart=no for every desktop file.
# Shroud enforces a kill switch, so an unsupervised crash must not leave the
# firewall in place with no daemon to manage it.
[Unit]
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
Restart=on-failure
RestartSec=5s
"#;

/// Autostart manager using XDG desktop files
pub struct Autostart;

impl Autostart {
    /// Get the path to the autostart desktop file
    fn desktop_file_path() -> Result<PathBuf, String> {
        dirs::config_dir()
            .map(|c| c.join("autostart/shroud.desktop"))
            .ok_or_else(|| "Could not determine XDG config directory".to_string())
    }

    /// Name of the unit `systemd-xdg-autostart-generator` synthesises for our
    /// desktop file.
    ///
    /// The generator derives it from the desktop file's basename, so this must
    /// stay in lockstep with [`Self::desktop_file_path`].
    fn generated_unit_name() -> &'static str {
        "app-shroud@autostart.service"
    }

    /// Directory holding our drop-in for the generated autostart unit.
    fn restart_dropin_dir() -> Result<PathBuf, String> {
        dirs::config_dir()
            .map(|c| {
                c.join("systemd/user")
                    .join(format!("{}.d", Self::generated_unit_name()))
            })
            .ok_or_else(|| "Could not determine XDG config directory".to_string())
    }

    /// Path to the drop-in file itself.
    fn restart_dropin_path() -> Result<PathBuf, String> {
        Ok(Self::restart_dropin_dir()?.join("50-shroud-restart.conf"))
    }

    /// Install the restart-policy drop-in over the generated autostart unit.
    ///
    /// Best effort: autostart still works without it, so a failure here is
    /// surfaced to the caller but must not fail `enable()`.
    fn install_restart_policy() -> Result<(), String> {
        let dir = Self::restart_dropin_dir()?;
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create systemd drop-in directory: {}", e))?;

        let path = Self::restart_dropin_path()?;
        fs::write(&path, RESTART_DROPIN)
            .map_err(|e| format!("Failed to write restart drop-in: {}", e))?;

        // The drop-in only takes effect once systemd re-reads unit state.
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();

        Ok(())
    }

    /// Remove the restart-policy drop-in (and its directory when empty).
    fn remove_restart_policy() -> Result<(), String> {
        let path = Self::restart_dropin_path()?;
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| format!("Failed to remove restart drop-in: {}", e))?;
        }

        // Only removes the directory if we left it empty; ignore failure.
        if let Ok(dir) = Self::restart_dropin_dir() {
            let _ = fs::remove_dir(dir);
        }

        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();

        Ok(())
    }

    /// Whether the restart-policy drop-in is currently installed.
    pub fn has_restart_policy() -> bool {
        Self::restart_dropin_path()
            .map(|p| p.exists())
            .unwrap_or(false)
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
Name=VPN Shroud Manager
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
            has_restart_policy: Self::has_restart_policy(),
            restart_dropin_path: Self::restart_dropin_path().ok(),
        }
    }

    /// Enable autostart
    pub fn enable() -> Result<(), String> {
        let path = Self::desktop_file_path()?;
        let _ = Self::cleanup_old_systemd();
        Self::enable_at(&path)?;

        // Best effort: autostart is still functional without the drop-in, it
        // just loses crash supervision. Warn rather than fail the whole call.
        if let Err(e) = Self::install_restart_policy() {
            tracing::warn!("Could not install autostart restart policy: {}", e);
        }

        Ok(())
    }

    /// Write the autostart entry to `path`.
    ///
    /// Split out so tests can drive it with a temporary path: they would
    /// otherwise create and delete the real `~/.config/autostart` entry of
    /// whoever runs the suite, silently disabling autostart on that machine.
    fn enable_at(path: &std::path::Path) -> Result<(), String> {
        let content = Self::generate_desktop_entry()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create autostart directory: {}", e))?;
        }

        fs::write(path, &content).map_err(|e| format!("Failed to write desktop file: {}", e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o755);
            let _ = fs::set_permissions(path, perms);
        }

        Ok(())
    }

    /// Disable autostart
    pub fn disable() -> Result<(), String> {
        let path = Self::desktop_file_path()?;
        Self::disable_at(&path)?;

        if let Err(e) = Self::remove_restart_policy() {
            tracing::warn!("Could not remove autostart restart policy: {}", e);
        }

        Ok(())
    }

    /// Remove the autostart entry at `path`, succeeding if it is already gone.
    fn disable_at(path: &std::path::Path) -> Result<(), String> {
        if path.exists() {
            fs::remove_file(path).map_err(|e| format!("Failed to remove desktop file: {}", e))?;
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
    /// Whether the crash-supervision drop-in is installed over the unit that
    /// `systemd-xdg-autostart-generator` synthesises.
    pub has_restart_policy: bool,
    pub restart_dropin_path: Option<PathBuf>,
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
        assert!(content.contains("Name=VPN Shroud"));
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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shroud.desktop");

        assert!(Autostart::disable_at(&path).is_ok());

        let result = Autostart::disable_at(&path);
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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("autostart").join("shroud.desktop");
        let parent = path.parent().unwrap();

        let result = Autostart::enable_at(&path);
        assert!(result.is_ok());
        assert!(parent.exists());
        assert!(path.exists());

        assert!(Autostart::disable_at(&path).is_ok());
        assert!(!path.exists());
    }
}

#[cfg(test)]
mod restart_policy_tests {
    use super::*;

    /// The drop-in only applies if its directory name matches the unit that
    /// `systemd-xdg-autostart-generator` synthesises from our desktop file.
    /// The generator derives the unit name from the desktop file's basename,
    /// so these two must never drift apart.
    #[test]
    fn test_generated_unit_name_matches_desktop_file_stem() {
        let desktop = Autostart::desktop_file_path().expect("config dir");
        let stem = desktop
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("desktop file stem");
        assert_eq!(
            Autostart::generated_unit_name(),
            format!("app-{stem}@autostart.service"),
            "drop-in would be installed for the wrong unit and silently do nothing"
        );
    }

    #[test]
    fn test_dropin_path_is_under_systemd_user_dropin_dir() {
        let path = Autostart::restart_dropin_path().expect("config dir");
        let as_str = path.to_string_lossy();
        assert!(as_str.contains("systemd/user"), "unexpected path: {as_str}");
        assert!(
            as_str.contains("app-shroud@autostart.service.d"),
            "drop-in must live in the unit's .d directory: {as_str}"
        );
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("conf"));
    }

    /// Guards the reliability fix itself: a kill-switch daemon must be
    /// restarted if it crashes, but must NOT be restarted after a clean quit.
    #[test]
    fn test_dropin_declares_on_failure_restart() {
        assert!(RESTART_DROPIN.contains("Restart=on-failure"));
        assert!(RESTART_DROPIN.contains("RestartSec="));
        assert!(
            !RESTART_DROPIN.contains("Restart=always"),
            "Restart=always would fight the tray's Quit action"
        );
    }

    /// Without a start limit, a binary that crashes on startup would be
    /// respawned forever.
    #[test]
    fn test_dropin_bounds_restart_storms() {
        assert!(RESTART_DROPIN.contains("StartLimitBurst="));
        assert!(RESTART_DROPIN.contains("StartLimitIntervalSec="));
    }

    /// systemd rejects the whole drop-in if the start-limit directives land in
    /// `[Service]` instead of `[Unit]`.
    #[test]
    fn test_start_limit_directives_are_in_unit_section() {
        let unit_section = RESTART_DROPIN
            .split("[Service]")
            .next()
            .expect("drop-in must have a [Service] section");
        assert!(unit_section.contains("[Unit]"));
        assert!(unit_section.contains("StartLimitBurst="));
        assert!(unit_section.contains("StartLimitIntervalSec="));
    }
}
