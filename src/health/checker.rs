//! Health checker implementation
//!
//! Verifies VPN tunnel connectivity by making HTTP requests through the tunnel
//! and checking for expected responses.

use std::time::{Duration, Instant};
use tokio::task::spawn_blocking;
use tokio::time::timeout;
use tracing::{debug, warn};
use ureq;

/// Result of a health check
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthResult {
    /// Health check passed - tunnel is working
    Healthy,
    /// Health check showed degraded connectivity (high latency, packet loss)
    Degraded { latency_ms: u64 },
    /// Health check failed - tunnel appears dead
    Dead { reason: String },
    /// Health checks are suspended (e.g., during system wake)
    /// Callers should leave state unchanged — neither affirm health nor declare failure.
    Suspended,
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
    /// Number of consecutive degraded checks before warning
    pub degraded_threshold: u32,
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
            // Increased from 2000ms - builds/updates can cause temporary latency
            degraded_threshold_ms: 5000,
            failure_threshold: 3,
            // Require 2 consecutive degraded checks before warning
            degraded_threshold: 2,
        }
    }
}

/// Health checker for VPN connectivity
pub struct HealthChecker {
    config: HealthConfig,
    consecutive_failures: u32,
    consecutive_degraded: u32,
    /// When set, health checks are suspended until this instant
    suspended_until: Option<std::time::Instant>,
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
            consecutive_degraded: 0,
            suspended_until: None,
        }
    }

    /// Reset failure counter (call after successful connection)
    pub fn reset(&mut self) {
        self.consecutive_failures = 0;
        self.consecutive_degraded = 0;
        self.suspended_until = None;
    }

    /// Suspend health checks for a duration
    ///
    /// Used during system wake or other events that may cause transient failures.
    /// Health checks will return Suspended while suspended — callers should
    /// leave state unchanged (not affirm health, not declare failure).
    pub fn suspend(&mut self, duration: Duration) {
        let until = std::time::Instant::now() + duration;
        debug!("Suspending health checks for {:?}", duration);
        self.suspended_until = Some(until);
        // Do NOT reset failure counters — preserve them for post-suspension check
    }

    /// Resume health checks (cancel suspension)
    #[allow(dead_code)]
    pub fn resume(&mut self) {
        if self.suspended_until.is_some() {
            debug!("Resuming health checks");
            self.suspended_until = None;
        }
    }

    /// Check if health checks are currently suspended
    pub fn is_suspended(&self) -> bool {
        if let Some(until) = self.suspended_until {
            std::time::Instant::now() < until
        } else {
            false
        }
    }

    /// Perform a health check
    ///
    /// Returns the health status of the VPN tunnel.
    /// Only returns Degraded after consecutive_degraded threshold is reached
    /// to avoid false positives during temporary system load (builds, updates).
    /// Returns Healthy immediately if checks are suspended.
    pub async fn check(&mut self) -> HealthResult {
        // Check if suspended (e.g., during system wake)
        if self.is_suspended() {
            debug!("Health check skipped - suspended");
            return HealthResult::Suspended;
        }

        for endpoint in &self.config.endpoints {
            match self.check_endpoint(endpoint).await {
                Ok(latency_ms) => {
                    self.consecutive_failures = 0;

                    if latency_ms > self.config.degraded_threshold_ms {
                        self.consecutive_degraded += 1;
                        debug!(
                            "Health check high latency: {}ms (degraded {}/{})",
                            latency_ms, self.consecutive_degraded, self.config.degraded_threshold
                        );

                        // Only report degraded after consecutive threshold
                        if self.consecutive_degraded >= self.config.degraded_threshold {
                            return HealthResult::Degraded { latency_ms };
                        }
                        // Below threshold - treat as healthy but track
                        return HealthResult::Healthy;
                    }

                    // Good latency - reset degraded counter
                    self.consecutive_degraded = 0;
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
        // Also count as degraded
        self.consecutive_degraded += 1;

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

    /// Check a single endpoint using native HTTP (ureq)
    ///
    /// Returns latency in milliseconds on success.
    async fn check_endpoint(&self, endpoint: &str) -> Result<u64, String> {
        let url = endpoint.to_string();
        let timeout_secs = self.config.timeout_secs;

        let result = timeout(
            Duration::from_secs(timeout_secs + 2), // outer safety timeout
            spawn_blocking(move || {
                let start = Instant::now();

                let config = ureq::Agent::config_builder()
                    .timeout_global(Some(std::time::Duration::from_secs(timeout_secs)))
                    .timeout_connect(Some(std::time::Duration::from_secs(5)))
                    .max_redirects(0) // SECURITY: Do not follow redirects (SHROUD-VULN-013)
                    .build();
                let agent = ureq::Agent::new_with_config(config);

                match agent.get(&url).call() {
                    Ok(resp) => {
                        let status = resp.status().as_u16();
                        // Consume body to complete request
                        let _ = resp.into_body().read_to_string();
                        if (200..400).contains(&status) {
                            Ok(start.elapsed().as_millis() as u64)
                        } else {
                            Err(format!("HTTP status: {}", status))
                        }
                    }
                    Err(e) => Err(format!("HTTP error: {}", e)),
                }
            }),
        )
        .await;

        match result {
            Ok(Ok(inner)) => inner,
            Ok(Err(e)) => Err(format!("spawn_blocking error: {}", e)),
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
            degraded_threshold: 2,
        };
        let checker = HealthChecker::with_config(config.clone());
        assert_eq!(checker.config.timeout_secs, 5);
        assert_eq!(checker.config.endpoints.len(), 1);
        assert_eq!(checker.config.failure_threshold, 5);
    }

    // ----- Reset behaviour -----

    #[test]
    fn test_reset_clears_all_counters() {
        let mut checker = HealthChecker::new();
        checker.consecutive_failures = 5;
        checker.consecutive_degraded = 3;
        checker.suspended_until = Some(std::time::Instant::now() + Duration::from_secs(60));

        checker.reset();

        assert_eq!(checker.consecutive_failures, 0);
        assert_eq!(checker.consecutive_degraded, 0);
        assert!(checker.suspended_until.is_none());
    }

    #[test]
    fn test_reset_is_idempotent() {
        let mut checker = HealthChecker::new();
        checker.reset();
        checker.reset();
        assert_eq!(checker.consecutive_failures, 0);
        assert_eq!(checker.consecutive_degraded, 0);
    }

    // ----- Suspension behaviour -----

    #[test]
    fn test_suspend_sets_until() {
        let mut checker = HealthChecker::new();
        checker.suspend(Duration::from_secs(30));
        assert!(checker.suspended_until.is_some());
        assert!(checker.is_suspended());
    }

    #[test]
    fn test_suspend_preserves_counters() {
        let mut checker = HealthChecker::new();
        checker.consecutive_failures = 5;
        checker.consecutive_degraded = 3;

        checker.suspend(Duration::from_secs(10));

        // SECURITY: Counters are preserved during suspension so post-wake
        // checks can detect ongoing failures (SHROUD-VULN-017).
        assert_eq!(checker.consecutive_failures, 5);
        assert_eq!(checker.consecutive_degraded, 3);
    }

    #[test]
    fn test_suspend_expired_not_suspended() {
        let mut checker = HealthChecker::new();
        // Set suspension to the past
        checker.suspended_until = Some(std::time::Instant::now() - Duration::from_secs(1));
        assert!(!checker.is_suspended());
    }

    #[test]
    fn test_resume_clears_suspension() {
        let mut checker = HealthChecker::new();
        checker.suspend(Duration::from_secs(300));
        assert!(checker.is_suspended());

        checker.resume();
        assert!(!checker.is_suspended());
        assert!(checker.suspended_until.is_none());
    }

    #[test]
    fn test_resume_when_not_suspended() {
        let mut checker = HealthChecker::new();
        // Should not panic
        checker.resume();
        assert!(!checker.is_suspended());
    }

    // ----- Threshold logic -----

    #[test]
    fn test_failure_counter_increments() {
        let mut checker = HealthChecker::new();
        assert_eq!(checker.consecutive_failures, 0);

        checker.consecutive_failures += 1;
        assert_eq!(checker.consecutive_failures, 1);

        checker.consecutive_failures += 1;
        assert_eq!(checker.consecutive_failures, 2);
    }

    #[test]
    fn test_degraded_counter_increments() {
        let mut checker = HealthChecker::new();
        assert_eq!(checker.consecutive_degraded, 0);

        checker.consecutive_degraded += 1;
        assert_eq!(checker.consecutive_degraded, 1);
    }

    #[test]
    fn test_failure_threshold_boundary() {
        let config = HealthConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let mut checker = HealthChecker::with_config(config);

        // Below threshold
        checker.consecutive_failures = 2;
        assert!(checker.consecutive_failures < checker.config.failure_threshold);

        // At threshold
        checker.consecutive_failures = 3;
        assert!(checker.consecutive_failures >= checker.config.failure_threshold);
    }

    #[test]
    fn test_degraded_threshold_boundary() {
        let config = HealthConfig {
            degraded_threshold: 2,
            ..Default::default()
        };
        let mut checker = HealthChecker::with_config(config);

        // Below threshold
        checker.consecutive_degraded = 1;
        assert!(checker.consecutive_degraded < checker.config.degraded_threshold);

        // At threshold
        checker.consecutive_degraded = 2;
        assert!(checker.consecutive_degraded >= checker.config.degraded_threshold);
    }

    // ----- HealthResult equality -----

    #[test]
    fn test_health_result_equality() {
        assert_eq!(HealthResult::Healthy, HealthResult::Healthy);
        assert_ne!(
            HealthResult::Healthy,
            HealthResult::Dead { reason: "x".into() }
        );

        assert_eq!(
            HealthResult::Degraded { latency_ms: 100 },
            HealthResult::Degraded { latency_ms: 100 }
        );
        assert_ne!(
            HealthResult::Degraded { latency_ms: 100 },
            HealthResult::Degraded { latency_ms: 200 }
        );
    }

    #[test]
    fn test_health_result_debug() {
        let healthy = format!("{:?}", HealthResult::Healthy);
        assert!(healthy.contains("Healthy"));

        let dead = format!(
            "{:?}",
            HealthResult::Dead {
                reason: "timeout".into()
            }
        );
        assert!(dead.contains("Dead"));
        assert!(dead.contains("timeout"));
    }

    #[test]
    fn test_health_result_clone() {
        let result = HealthResult::Degraded { latency_ms: 500 };
        let cloned = result.clone();
        assert_eq!(result, cloned);
    }

    // ----- Config edge cases -----

    #[test]
    fn test_default_config_has_multiple_endpoints() {
        let config = HealthConfig::default();
        assert!(
            config.endpoints.len() >= 2,
            "Should have fallback endpoints"
        );
    }

    #[test]
    fn test_config_with_single_endpoint() {
        let config = HealthConfig {
            endpoints: vec!["https://example.com".into()],
            ..Default::default()
        };
        let checker = HealthChecker::with_config(config);
        assert_eq!(checker.config.endpoints.len(), 1);
    }

    #[test]
    fn test_config_clone() {
        let config = HealthConfig::default();
        let cloned = config.clone();
        assert_eq!(config.timeout_secs, cloned.timeout_secs);
        assert_eq!(config.endpoints.len(), cloned.endpoints.len());
    }

    #[test]
    fn test_default_impl() {
        let checker = HealthChecker::default();
        assert_eq!(checker.consecutive_failures, 0);
        assert_eq!(checker.consecutive_degraded, 0);
        assert!(checker.suspended_until.is_none());
    }
}
