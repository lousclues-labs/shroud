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

/// Validate a WireGuard config file
pub fn validate_wireguard(path: &Path) -> Result<(), ValidationError> {
    let contents = fs::read_to_string(path)?;

    if contents.trim().is_empty() {
        return Err(ValidationError::EmptyFile);
    }

    if !contents.to_lowercase().contains("[interface]") {
        return Err(ValidationError::MissingSection("Interface".into()));
    }

    if !contents.to_lowercase().contains("privatekey") {
        return Err(ValidationError::MissingField("PrivateKey".into()));
    }

    if !contents.to_lowercase().contains("[peer]") {
        return Err(ValidationError::MissingSection("Peer".into()));
    }

    let peer_section = contents.to_lowercase();
    let peer_start = peer_section.find("[peer]").unwrap();
    let peer_content = &contents[peer_start..];

    if !peer_content.to_lowercase().contains("publickey") {
        return Err(ValidationError::MissingField("Peer.PublicKey".into()));
    }

    if !peer_content.to_lowercase().contains("endpoint")
        && !peer_content.to_lowercase().contains("allowedips")
    {
        return Err(ValidationError::MissingField(
            "Peer.Endpoint or Peer.AllowedIPs".into(),
        ));
    }

    Ok(())
}

/// Validate an OpenVPN config file
pub fn validate_openvpn(path: &Path) -> Result<(), ValidationError> {
    let contents = fs::read_to_string(path)?;

    if contents.trim().is_empty() {
        return Err(ValidationError::EmptyFile);
    }

    let has_remote = contents.lines().any(|l| {
        let trimmed = l.trim();
        trimmed.starts_with("remote ") || trimmed.starts_with("<connection>")
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
        log::warn!("OpenVPN config may be missing authentication configuration");
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
