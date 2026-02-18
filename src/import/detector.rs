// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VpnConfigType {
    WireGuard,
    OpenVpn,
}

impl std::fmt::Display for VpnConfigType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VpnConfigType::WireGuard => write!(f, "WireGuard"),
            VpnConfigType::OpenVpn => write!(f, "OpenVPN"),
        }
    }
}

/// Detect VPN config type from file path and contents
pub fn detect_config_type(path: &Path) -> Option<VpnConfigType> {
    let extension = path.extension()?.to_str()?;

    match extension.to_lowercase().as_str() {
        "ovpn" => Some(VpnConfigType::OpenVpn),
        "conf" => {
            // Read only the first 4KB — enough to detect [Interface] and PrivateKey
            // without loading potentially large non-WireGuard .conf files.
            let contents = {
                use std::io::Read;
                let mut buf = vec![0u8; 4096];
                let mut file = fs::File::open(path).ok()?;
                let n = file.read(&mut buf).ok()?;
                String::from_utf8_lossy(&buf[..n]).to_string()
            };
            if is_wireguard_config(&contents) {
                Some(VpnConfigType::WireGuard)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn is_wireguard_config(contents: &str) -> bool {
    let has_interface = contents
        .lines()
        .any(|l| l.trim().eq_ignore_ascii_case("[interface]"));
    let has_peer = contents
        .lines()
        .any(|l| l.trim().eq_ignore_ascii_case("[peer]"));
    let has_private_key = contents
        .lines()
        .any(|l| l.trim().to_lowercase().starts_with("privatekey"));

    has_interface && has_peer && has_private_key
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(contents: &str, ext: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::Builder::new().suffix(ext).tempfile().unwrap();
        write!(file, "{}", contents).unwrap();
        file
    }

    #[test]
    fn test_detect_openvpn_extension() {
        let file = write_temp("remote vpn.example.com 1194\n", ".ovpn");
        let detected = detect_config_type(file.path());
        assert_eq!(detected, Some(VpnConfigType::OpenVpn));
    }

    #[test]
    fn test_detect_wireguard_from_conf() {
        let file = write_temp(
            "[Interface]\nPrivateKey = abc\n[Peer]\nPublicKey = def\n",
            ".conf",
        );
        let detected = detect_config_type(file.path());
        assert_eq!(detected, Some(VpnConfigType::WireGuard));
    }

    #[test]
    fn test_detect_unknown_conf() {
        let file = write_temp("not a vpn", ".conf");
        let detected = detect_config_type(file.path());
        assert_eq!(detected, None);
    }

    #[test]
    fn test_detect_unknown_extension() {
        let file = write_temp("whatever", ".txt");
        let detected = detect_config_type(file.path());
        assert_eq!(detected, None);
    }
}
