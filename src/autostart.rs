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

    /// Find the installed shroud binary with absolute path
    fn find_binary() -> Result<PathBuf, String> {
        let candidates = [
            dirs::home_dir().map(|h| h.join(".cargo/bin/shroud")),
            dirs::home_dir().map(|h| h.join(".local/bin/shroud")),
            Some(PathBuf::from("/usr/local/bin/shroud")),
            Some(PathBuf::from("/usr/bin/shroud")),
        ];

        for candidate in candidates.into_iter().flatten() {
            if candidate.exists() && is_executable(&candidate) {
                return Ok(candidate);
            }
        }

        std::env::current_exe().map_err(|e| format!("Could not determine binary path: {}", e))
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
fn is_executable(path: &PathBuf) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &PathBuf) -> bool {
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
