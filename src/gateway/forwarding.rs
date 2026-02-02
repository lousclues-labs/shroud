//! IP forwarding control.
//!
//! Manages the kernel's IP forwarding settings via /proc/sys/net/ipv4/ip_forward

use super::GatewayError;
use log::{debug, info};
use std::fs;

const IPV4_FORWARD_PATH: &str = "/proc/sys/net/ipv4/ip_forward";
const IPV6_FORWARD_PATH: &str = "/proc/sys/net/ipv6/conf/all/forwarding";

/// Enable IP forwarding.
pub fn enable_forwarding(include_ipv6: bool) -> Result<(), GatewayError> {
    info!("Enabling IP forwarding");

    write_sysctl(IPV4_FORWARD_PATH, "1")?;
    debug!("IPv4 forwarding enabled");

    if include_ipv6 {
        write_sysctl(IPV6_FORWARD_PATH, "1")?;
        debug!("IPv6 forwarding enabled");
    } else {
        let _ = write_sysctl(IPV6_FORWARD_PATH, "0");
        debug!("IPv6 forwarding disabled (leak prevention)");
    }

    Ok(())
}

/// Disable IP forwarding.
pub fn disable_forwarding() -> Result<(), GatewayError> {
    info!("Disabling IP forwarding");

    write_sysctl(IPV4_FORWARD_PATH, "0")?;
    let _ = write_sysctl(IPV6_FORWARD_PATH, "0");

    Ok(())
}

/// Check if IP forwarding is currently enabled.
pub fn is_forwarding_enabled() -> bool {
    read_sysctl(IPV4_FORWARD_PATH)
        .map(|v| v.trim() == "1")
        .unwrap_or(false)
}

fn write_sysctl(path: &str, value: &str) -> Result<(), GatewayError> {
    fs::write(path, value).map_err(|e| {
        GatewayError::Forwarding(format!(
            "Failed to write to {}: {}. Try running as root.",
            path, e
        ))
    })
}

fn read_sysctl(path: &str) -> Result<String, GatewayError> {
    fs::read_to_string(path)
        .map_err(|e| GatewayError::Forwarding(format!("Failed to read {}: {}", path, e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_sysctl_paths_exist() {
        // These paths should exist on any Linux system
        assert!(Path::new(IPV4_FORWARD_PATH).exists());
    }
}
