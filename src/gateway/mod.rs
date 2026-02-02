//! VPN Gateway mode for Shroud.
//!
//! Gateway mode allows other devices on the network to route their
//! traffic through this machine's VPN tunnel.
//!
//! Architecture:
//! ```text
//! LAN Devices → Shroud Gateway → VPN Tunnel → Internet
//! ```

pub mod detect;
pub mod firewall;
pub mod forwarding;
pub mod nat;
pub mod status;

use crate::config::GatewayConfig;
use log::{debug, info};
use std::sync::atomic::{AtomicBool, Ordering};

pub use detect::{detect_lan_interface, detect_vpn_interface};
pub use firewall::{
    disable_forward_killswitch, disable_forward_rules, enable_forward_killswitch,
    enable_forward_rules,
};
pub use forwarding::{disable_forwarding, enable_forwarding, is_forwarding_enabled};
pub use nat::{disable_nat, enable_nat};

// Re-export for status display
#[allow(unused_imports)]
pub use status::GatewayStatus;

static GATEWAY_STATE: AtomicBool = AtomicBool::new(false);
static ORIGINAL_FORWARDING_STATE: AtomicBool = AtomicBool::new(false);

/// Error type for gateway operations
#[derive(Debug)]
pub enum GatewayError {
    Forwarding(String),
    Nat(String),
    Firewall(String),
    Detection(String),
    Config(String),
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GatewayError::Forwarding(s) => write!(f, "IP forwarding error: {}", s),
            GatewayError::Nat(s) => write!(f, "NAT error: {}", s),
            GatewayError::Firewall(s) => write!(f, "Firewall error: {}", s),
            GatewayError::Detection(s) => write!(f, "Interface detection error: {}", s),
            GatewayError::Config(s) => write!(f, "Configuration error: {}", s),
        }
    }
}

impl std::error::Error for GatewayError {}

/// Enable VPN gateway mode.
pub async fn enable(config: &GatewayConfig) -> Result<(), GatewayError> {
    info!("Enabling VPN gateway mode");

    // Store original forwarding state
    let was_forwarding = is_forwarding_enabled();
    ORIGINAL_FORWARDING_STATE.store(was_forwarding, Ordering::SeqCst);
    debug!("Original forwarding state: {}", was_forwarding);

    // Detect interfaces
    let lan_interface = config
        .lan_interface
        .clone()
        .or_else(detect_lan_interface)
        .ok_or_else(|| {
            GatewayError::Detection(
                "Could not detect LAN interface. Set gateway.lan_interface in config.".to_string(),
            )
        })?;

    let vpn_interface = detect_vpn_interface().ok_or_else(|| {
        GatewayError::Detection("Could not detect VPN interface. Is VPN connected?".to_string())
    })?;

    info!(
        "LAN interface: {}, VPN interface: {}",
        lan_interface, vpn_interface
    );

    // Enable IP forwarding
    enable_forwarding(config.enable_ipv6)?;

    // Enable NAT
    enable_nat(&vpn_interface)?;

    // Enable forward rules
    enable_forward_rules(&lan_interface, &vpn_interface, &config.allowed_clients)?;

    // Enable forward kill switch if configured
    if config.kill_switch_forwarding {
        enable_forward_killswitch(&lan_interface)?;
    }

    GATEWAY_STATE.store(true, Ordering::SeqCst);
    info!("VPN gateway mode enabled");

    Ok(())
}

/// Disable VPN gateway mode.
pub async fn disable() -> Result<(), GatewayError> {
    info!("Disabling VPN gateway mode");

    let _ = disable_forward_killswitch();
    let _ = disable_forward_rules();
    let _ = disable_nat();

    // Restore original forwarding state
    let was_forwarding = ORIGINAL_FORWARDING_STATE.load(Ordering::SeqCst);
    if !was_forwarding {
        let _ = disable_forwarding();
    }

    GATEWAY_STATE.store(false, Ordering::SeqCst);
    info!("VPN gateway mode disabled");

    Ok(())
}

/// Check if gateway mode is currently enabled.
pub fn is_enabled() -> bool {
    GATEWAY_STATE.load(Ordering::SeqCst)
}

/// Update gateway when VPN interface changes.
pub async fn update_vpn_interface(new_interface: &str) -> Result<(), GatewayError> {
    if !is_enabled() {
        return Ok(());
    }

    info!("Updating gateway for new VPN interface: {}", new_interface);

    let _ = disable_nat();
    enable_nat(new_interface)?;

    Ok(())
}
