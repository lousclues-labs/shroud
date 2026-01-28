//! Health checker implementation
//!
//! Verifies VPN tunnel connectivity by making HTTP requests through the tunnel
//! and checking for expected responses.

use log::{debug, warn};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

/// Result of a health check
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthResult {
    /// Health check passed - tunnel is working
    Healthy,
    /// Health check showed degraded connectivity (high latency, packet loss)
    Degraded { latency_ms: u64 },
    /// Health check failed - tunnel appears dead
    Dead { reason: String },
}

/// Configuration for health checks
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// Endpoints to check (in order of preference)
    pub endpoints: Vec<String>,
    /// Timeout for each check attempt
    pub timeout_secs: u64,
    /// Latency threshold above which connection is considered degraded (ms)
    pub degraded_threshold_ms: u64,
    /// Number of consecutive failures before declaring dead
    pub failure_threshold: u32,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            endpoints: vec![
                "https://1.1.1.1/cdn-cgi/trace".to_string(),
                "https://ifconfig.me/ip".to_string(),
                "https://api.ipify.org".to_string(),
            ],
            timeout_secs: 10,
            degraded_threshold_ms: 2000,
            failure_threshold: 3,
        }
    }
}

/// Health checker for VPN connectivity
pub struct HealthChecker {
    config: HealthConfig,
    consecutive_failures: u32,
}

impl HealthChecker {
    /// Create a new health checker with default configuration
    pub fn new() -> Self {
        Self::with_config(HealthConfig::default())
    }

    /// Create a new health checker with custom configuration
    pub fn with_config(config: HealthConfig) -> Self {
        Self {
            config,
            consecutive_failures: 0,
        }
    }

    /// Reset failure counter (call after successful connection)
    pub fn reset(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Perform a health check
    ///
    /// Returns the health status of the VPN tunnel.
    pub async fn check(&mut self) -> HealthResult {
        for endpoint in &self.config.endpoints {
            match self.check_endpoint(endpoint).await {
                Ok(latency_ms) => {
                    self.consecutive_failures = 0;

                    if latency_ms > self.config.degraded_threshold_ms {
                        debug!(
                            "Health check passed but degraded: {}ms > {}ms threshold",
                            latency_ms, self.config.degraded_threshold_ms
                        );
                        return HealthResult::Degraded { latency_ms };
                    }

                    debug!("Health check passed: {}ms", latency_ms);
                    return HealthResult::Healthy;
                }
                Err(e) => {
                    debug!("Health check failed for {}: {}", endpoint, e);
                    continue;
                }
            }
        }

        // All endpoints failed
        self.consecutive_failures += 1;

        if self.consecutive_failures >= self.config.failure_threshold {
            warn!(
                "Health check dead: {} consecutive failures",
                self.consecutive_failures
            );
            HealthResult::Dead {
                reason: format!(
                    "{} consecutive failures across all endpoints",
                    self.consecutive_failures
                ),
            }
        } else {
            warn!(
                "Health check degraded: {} failures (threshold: {})",
                self.consecutive_failures, self.config.failure_threshold
            );
            HealthResult::Degraded {
                latency_ms: self.config.timeout_secs * 1000,
            }
        }
    }

    /// Check a single endpoint using curl
    ///
    /// Returns latency in milliseconds on success.
    async fn check_endpoint(&self, endpoint: &str) -> Result<u64, String> {
        let start = std::time::Instant::now();

        let result = timeout(
            Duration::from_secs(self.config.timeout_secs),
            Command::new("curl")
                .args([
                    "-s",
                    "-o",
                    "/dev/null",
                    "-w",
                    "%{http_code}",
                    "--connect-timeout",
                    "5",
                    "--max-time",
                    &self.config.timeout_secs.to_string(),
                    endpoint,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let elapsed = start.elapsed().as_millis() as u64;
                let status = String::from_utf8_lossy(&output.stdout);

                if status.starts_with('2') || status.starts_with('3') {
                    Ok(elapsed)
                } else {
                    Err(format!("HTTP status: {}", status))
                }
            }
            Ok(Err(e)) => Err(format!("curl error: {}", e)),
            Err(_) => Err("timeout".to_string()),
        }
    }


}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_config_default() {
        let config = HealthConfig::default();
        assert!(!config.endpoints.is_empty());
        assert!(config.timeout_secs > 0);
        assert!(config.degraded_threshold_ms > 0);
        assert!(config.failure_threshold > 0);
    }

    #[test]
    fn test_health_checker_reset() {
        let mut checker = HealthChecker::new();
        checker.consecutive_failures = 5;
        checker.reset();
        assert_eq!(checker.consecutive_failures, 0);
    }

    #[test]
    fn test_health_config_custom() {
        let config = HealthConfig {
            endpoints: vec!["https://example.com".to_string()],
            timeout_secs: 5,
            degraded_threshold_ms: 1000,
            failure_threshold: 5,
        };
        let checker = HealthChecker::with_config(config.clone());
        assert_eq!(checker.config.timeout_secs, 5);
        assert_eq!(checker.config.endpoints.len(), 1);
        assert_eq!(checker.config.failure_threshold, 5);
    }
}
