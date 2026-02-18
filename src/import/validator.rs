// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

use std::fs;
use std::path::Path;

use thiserror::Error;

use super::detector::VpnConfigType;

#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("Cannot read file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Missing required section: [{0}]")]
    MissingSection(String),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("File is empty")]
    EmptyFile,
}

/// Maximum config file size (1 MB). WireGuard configs are typically <1KB,
/// OpenVPN configs <100KB. Anything larger is not a VPN config.
const MAX_CONFIG_SIZE: u64 = 1_048_576;

/// Validate a WireGuard config file
pub fn validate_wireguard(path: &Path) -> Result<(), ValidationError> {
    let meta = fs::metadata(path)?;
    if meta.len() > MAX_CONFIG_SIZE {
        return Err(ValidationError::MissingField(
            "file too large for VPN config (>1MB)".into(),
        ));
    }

    let contents = fs::read_to_string(path)?;

    if contents.trim().is_empty() {
        return Err(ValidationError::EmptyFile);
    }

    let lower = contents.to_lowercase();

    if !lower.contains("[interface]") {
        return Err(ValidationError::MissingSection("Interface".into()));
    }

    if !lower.contains("privatekey") {
        return Err(ValidationError::MissingField("PrivateKey".into()));
    }

    if !lower.contains("[peer]") {
        return Err(ValidationError::MissingSection("Peer".into()));
    }

    // Use the lowercased copy for all searches to avoid byte-index
    // misalignment between original and lowercased strings on multi-byte UTF-8.
    let peer_start = lower.find("[peer]").expect("contains check above");
    let peer_content = &lower[peer_start..];

    if !peer_content.contains("publickey") {
        return Err(ValidationError::MissingField("Peer.PublicKey".into()));
    }

    if !peer_content.contains("endpoint") && !peer_content.contains("allowedips") {
        return Err(ValidationError::MissingField(
            "Peer.Endpoint or Peer.AllowedIPs".into(),
        ));
    }

    Ok(())
}

/// Validate an OpenVPN config file
pub fn validate_openvpn(path: &Path) -> Result<(), ValidationError> {
    let meta = fs::metadata(path)?;
    if meta.len() > MAX_CONFIG_SIZE {
        return Err(ValidationError::MissingField(
            "file too large for VPN config (>1MB)".into(),
        ));
    }

    let contents = fs::read_to_string(path)?;

    if contents.trim().is_empty() {
        return Err(ValidationError::EmptyFile);
    }

    // Require an actual 'remote' directive. The <connection> tag is a container
    // that should contain 'remote' inside it — accepting <connection> alone would
    // pass validation for configs with no server to connect to.
    let has_remote = contents.lines().any(|l| {
        let trimmed = l.trim();
        trimmed.starts_with("remote ")
    });

    if !has_remote {
        return Err(ValidationError::MissingField("remote".into()));
    }

    let has_auth = contents.contains("auth-user-pass")
        || contents.contains("<ca>")
        || contents.contains("ca ")
        || contents.contains("<cert>")
        || contents.contains("pkcs12");

    if !has_auth {
        tracing::warn!("OpenVPN config may be missing authentication configuration");
    }

    Ok(())
}

/// Validate config based on detected type
pub fn validate(path: &Path, config_type: VpnConfigType) -> Result<(), ValidationError> {
    match config_type {
        VpnConfigType::WireGuard => validate_wireguard(path),
        VpnConfigType::OpenVpn => validate_openvpn(path),
    }
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
    fn test_wireguard_valid() {
        let file = write_temp(
            "[Interface]\nPrivateKey = abc\n[Peer]\nPublicKey = def\nEndpoint = 1.2.3.4:51820\n",
            ".conf",
        );
        assert!(validate_wireguard(file.path()).is_ok());
    }

    #[test]
    fn test_wireguard_missing_interface() {
        let file = write_temp("PrivateKey = abc\n", ".conf");
        let err = validate_wireguard(file.path()).unwrap_err();
        assert!(matches!(err, ValidationError::MissingSection(_)));
    }

    #[test]
    fn test_wireguard_missing_private_key() {
        let file = write_temp("[Interface]\n[Peer]\nPublicKey = def\n", ".conf");
        let err = validate_wireguard(file.path()).unwrap_err();
        assert!(matches!(err, ValidationError::MissingField(_)));
    }

    #[test]
    fn test_openvpn_valid() {
        let file = write_temp("remote vpn.example.com 1194\n<ca>\nabc\n</ca>\n", ".ovpn");
        assert!(validate_openvpn(file.path()).is_ok());
    }

    #[test]
    fn test_openvpn_missing_remote() {
        let file = write_temp("dev tun\n", ".ovpn");
        let err = validate_openvpn(file.path()).unwrap_err();
        assert!(matches!(err, ValidationError::MissingField(_)));
    }
}
