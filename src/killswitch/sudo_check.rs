//! Sudo access validation for kill switch operations.

use log::{debug, error, warn};
use std::process::{Command, Stdio};

use super::paths::{ip6tables, iptables, nft};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SudoAccessStatus {
    Ok,
    RequiresPassword,
    SudoNotFound,
    BinaryNotFound(String),
}

pub fn check_sudo_access() -> SudoAccessStatus {
    let iptables_path = iptables();

    if !std::path::Path::new(iptables_path).exists() {
        return SudoAccessStatus::BinaryNotFound(iptables_path.to_string());
    }

    let output = Command::new("sudo")
        .args(["-n", iptables_path, "-L", "-n"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                debug!("Sudo access check passed for {}", iptables_path);
                SudoAccessStatus::Ok
            } else {
                let stderr = String::from_utf8_lossy(&result.stderr);
                if stderr.contains("password is required") || stderr.contains("Password") {
                    warn!("Sudo requires password for {}", iptables_path);
                    SudoAccessStatus::RequiresPassword
                } else {
                    debug!("Sudo check failed: {}", stderr.trim());
                    SudoAccessStatus::RequiresPassword
                }
            }
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                error!("sudo command not found");
                SudoAccessStatus::SudoNotFound
            } else {
                warn!("Failed to run sudo check: {}", e);
                SudoAccessStatus::RequiresPassword
            }
        }
    }
}

#[allow(dead_code)]
pub fn check_sudo_access_with_message() -> Result<(), String> {
    match check_sudo_access() {
        SudoAccessStatus::Ok => Ok(()),
        SudoAccessStatus::RequiresPassword => Err(format!(
            "Permission denied. Kill switch requires sudo access.\n\n\
Detected binary paths:\n  iptables:  {}\n  ip6tables: {}\n  nft:       {}\n\n\
To fix, run:\n  ./setup.sh --install-sudoers\n\n\
Or manually add these to /etc/sudoers.d/shroud:\n  %wheel ALL=(ALL) NOPASSWD: {}\n  %wheel ALL=(ALL) NOPASSWD: {}\n  %wheel ALL=(ALL) NOPASSWD: {}",
            iptables(),
            ip6tables(),
            nft(),
            iptables(),
            ip6tables(),
            nft()
        )),
        SudoAccessStatus::SudoNotFound => Err("sudo command not found. Please install sudo.".to_string()),
        SudoAccessStatus::BinaryNotFound(path) => Err(format!(
            "Firewall binary not found: {}\nPlease install iptables or nftables.",
            path
        )),
    }
}

pub fn validate_sudoers_on_startup() {
    match check_sudo_access() {
        SudoAccessStatus::Ok => {
            debug!("Sudoers configuration validated successfully");
        }
        SudoAccessStatus::RequiresPassword => {
            warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            warn!("KILL SWITCH CONFIGURATION ISSUE");
            warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            warn!("");
            warn!("Sudo requires a password for iptables. The kill switch");
            warn!("will not work until this is fixed.");
            warn!("");
            warn!("Detected binary paths:");
            warn!("  iptables:  {}", iptables());
            warn!("  ip6tables: {}", ip6tables());
            warn!("  nft:       {}", nft());
            warn!("");
            warn!("To fix, run: ./setup.sh --install-sudoers");
            warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        }
        SudoAccessStatus::SudoNotFound => {
            error!("sudo command not found - kill switch will not work");
        }
        SudoAccessStatus::BinaryNotFound(path) => {
            error!(
                "Firewall binary not found: {} - kill switch will not work",
                path
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sudo_access_check_returns_valid_status() {
        let status = check_sudo_access();
        match status {
            SudoAccessStatus::Ok
            | SudoAccessStatus::RequiresPassword
            | SudoAccessStatus::SudoNotFound
            | SudoAccessStatus::BinaryNotFound(_) => {}
        }
    }

    #[test]
    fn test_sudo_access_status_equality() {
        assert_eq!(SudoAccessStatus::Ok, SudoAccessStatus::Ok);
        assert_eq!(
            SudoAccessStatus::RequiresPassword,
            SudoAccessStatus::RequiresPassword
        );
        assert_eq!(
            SudoAccessStatus::SudoNotFound,
            SudoAccessStatus::SudoNotFound
        );
        assert_eq!(
            SudoAccessStatus::BinaryNotFound("/usr/sbin/iptables".into()),
            SudoAccessStatus::BinaryNotFound("/usr/sbin/iptables".into())
        );
    }

    #[test]
    fn test_sudo_access_status_inequality() {
        assert_ne!(SudoAccessStatus::Ok, SudoAccessStatus::RequiresPassword);
        assert_ne!(SudoAccessStatus::Ok, SudoAccessStatus::SudoNotFound);
        assert_ne!(
            SudoAccessStatus::BinaryNotFound("a".into()),
            SudoAccessStatus::BinaryNotFound("b".into())
        );
    }

    #[test]
    fn test_sudo_access_status_clone() {
        let status = SudoAccessStatus::BinaryNotFound("/sbin/iptables".into());
        let cloned = status.clone();
        assert_eq!(status, cloned);
    }

    #[test]
    fn test_sudo_access_status_debug() {
        let debug = format!("{:?}", SudoAccessStatus::Ok);
        assert!(debug.contains("Ok"));

        let debug = format!("{:?}", SudoAccessStatus::BinaryNotFound("x".into()));
        assert!(debug.contains("BinaryNotFound"));
    }

    #[test]
    fn test_check_sudo_access_with_message_returns_result() {
        let result = check_sudo_access_with_message();
        // Just verify it doesn't panic and returns a Result
        match result {
            Ok(()) => {} // sudo access works
            Err(msg) => {
                assert!(!msg.is_empty());
            }
        }
    }
}
