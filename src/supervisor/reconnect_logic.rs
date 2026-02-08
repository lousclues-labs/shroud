//! Reconnect decision logic — pure functions, easily testable.
//!
//! Extracts the delay calculation and reconnect-decision logic out of
//! the async `attempt_reconnect` method so it can be unit-tested
//! without spawning a supervisor.

use std::time::{Duration, Instant};

/// Reconnect configuration (mirrors the constants in `super`).
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    pub enabled: bool,
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 10,
            base_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// Tracks the progress of a reconnection cycle.
#[derive(Debug, Clone)]
pub struct ReconnectTracker {
    pub attempt: u32,
    pub last_attempt: Option<Instant>,
    pub last_vpn: Option<String>,
    pub total_attempts: u64,
    pub successful_reconnects: u64,
}

impl ReconnectTracker {
    pub fn new() -> Self {
        Self {
            attempt: 0,
            last_attempt: None,
            last_vpn: None,
            total_attempts: 0,
            successful_reconnects: 0,
        }
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
        self.last_attempt = None;
    }

    pub fn record_attempt(&mut self, vpn: &str) {
        self.attempt += 1;
        self.last_attempt = Some(Instant::now());
        self.last_vpn = Some(vpn.to_string());
        self.total_attempts += 1;
    }

    pub fn record_success(&mut self) {
        self.successful_reconnects += 1;
        self.reset();
    }

    pub fn success_rate(&self) -> f64 {
        if self.total_attempts == 0 {
            return 1.0;
        }
        self.successful_reconnects as f64 / self.total_attempts as f64
    }
}

impl Default for ReconnectTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate the backoff delay for a given attempt.
///
/// Uses linear backoff: `base_delay * (attempt + 1)`, capped at `max_delay`.
/// This matches the actual supervisor implementation.
pub fn calculate_delay(config: &ReconnectConfig, attempt: u32) -> Duration {
    let delay_secs = config.base_delay.as_secs() * (attempt as u64 + 1);
    let capped = delay_secs.min(config.max_delay.as_secs());
    Duration::from_secs(capped)
}

/// Reconnect decision result.
#[derive(Debug, Clone, PartialEq)]
pub enum ReconnectDecision {
    /// Attempt reconnect now.
    ReconnectNow,
    /// Wait before next attempt.
    WaitFor(Duration),
    /// Max attempts reached — give up.
    GiveUp,
    /// Reconnect is disabled.
    Disabled,
}

/// Decide whether to reconnect and how long to wait.
pub fn decide_reconnect(config: &ReconnectConfig, tracker: &ReconnectTracker) -> ReconnectDecision {
    if !config.enabled {
        return ReconnectDecision::Disabled;
    }
    if tracker.attempt >= config.max_attempts {
        return ReconnectDecision::GiveUp;
    }
    if let Some(last) = tracker.last_attempt {
        let required = calculate_delay(config, tracker.attempt);
        let elapsed = last.elapsed();
        if elapsed < required {
            return ReconnectDecision::WaitFor(required - elapsed);
        }
    }
    ReconnectDecision::ReconnectNow
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod config {
        use super::*;

        #[test]
        fn test_defaults() {
            let c = ReconnectConfig::default();
            assert!(c.enabled);
            assert_eq!(c.max_attempts, 10);
            assert_eq!(c.base_delay, Duration::from_secs(2));
            assert_eq!(c.max_delay, Duration::from_secs(30));
        }
    }

    mod tracker {
        use super::*;

        #[test]
        fn test_new() {
            let t = ReconnectTracker::new();
            assert_eq!(t.attempt, 0);
            assert!(t.last_attempt.is_none());
            assert!(t.last_vpn.is_none());
            assert_eq!(t.total_attempts, 0);
            assert_eq!(t.successful_reconnects, 0);
        }

        #[test]
        fn test_record_attempt() {
            let mut t = ReconnectTracker::new();
            t.record_attempt("vpn1");
            assert_eq!(t.attempt, 1);
            assert!(t.last_attempt.is_some());
            assert_eq!(t.last_vpn.as_deref(), Some("vpn1"));
            assert_eq!(t.total_attempts, 1);
        }

        #[test]
        fn test_multiple_attempts() {
            let mut t = ReconnectTracker::new();
            t.record_attempt("vpn1");
            t.record_attempt("vpn1");
            t.record_attempt("vpn1");
            assert_eq!(t.attempt, 3);
            assert_eq!(t.total_attempts, 3);
        }

        #[test]
        fn test_reset_keeps_totals() {
            let mut t = ReconnectTracker::new();
            t.record_attempt("vpn1");
            t.record_attempt("vpn1");
            t.reset();
            assert_eq!(t.attempt, 0);
            assert!(t.last_attempt.is_none());
            assert_eq!(t.total_attempts, 2);
        }

        #[test]
        fn test_record_success() {
            let mut t = ReconnectTracker::new();
            t.record_attempt("vpn1");
            t.record_attempt("vpn1");
            t.record_success();
            assert_eq!(t.attempt, 0);
            assert_eq!(t.successful_reconnects, 1);
            assert_eq!(t.total_attempts, 2);
        }

        #[test]
        fn test_success_rate_no_attempts() {
            let t = ReconnectTracker::new();
            assert!((t.success_rate() - 1.0).abs() < f64::EPSILON);
        }

        #[test]
        fn test_success_rate_mixed() {
            let mut t = ReconnectTracker::new();
            t.total_attempts = 4;
            t.successful_reconnects = 2;
            assert!((t.success_rate() - 0.5).abs() < f64::EPSILON);
        }

        #[test]
        fn test_default_impl() {
            let t = ReconnectTracker::default();
            assert_eq!(t.attempt, 0);
        }
    }

    mod delay {
        use super::*;

        #[test]
        fn test_first_attempt() {
            let c = ReconnectConfig::default();
            // base(2) * (0+1) = 2
            assert_eq!(calculate_delay(&c, 0), Duration::from_secs(2));
        }

        #[test]
        fn test_linear_growth() {
            let c = ReconnectConfig {
                base_delay: Duration::from_secs(2),
                max_delay: Duration::from_secs(100),
                ..Default::default()
            };
            assert_eq!(calculate_delay(&c, 0), Duration::from_secs(2));
            assert_eq!(calculate_delay(&c, 1), Duration::from_secs(4));
            assert_eq!(calculate_delay(&c, 2), Duration::from_secs(6));
            assert_eq!(calculate_delay(&c, 4), Duration::from_secs(10));
        }

        #[test]
        fn test_capped_at_max() {
            let c = ReconnectConfig {
                base_delay: Duration::from_secs(2),
                max_delay: Duration::from_secs(10),
                ..Default::default()
            };
            assert_eq!(calculate_delay(&c, 100), Duration::from_secs(10));
        }
    }

    mod decision {
        use super::*;

        #[test]
        fn test_disabled() {
            let c = ReconnectConfig {
                enabled: false,
                ..Default::default()
            };
            let t = ReconnectTracker::new();
            assert_eq!(decide_reconnect(&c, &t), ReconnectDecision::Disabled);
        }

        #[test]
        fn test_give_up() {
            let c = ReconnectConfig {
                max_attempts: 3,
                ..Default::default()
            };
            let mut t = ReconnectTracker::new();
            t.attempt = 3;
            assert_eq!(decide_reconnect(&c, &t), ReconnectDecision::GiveUp);
        }

        #[test]
        fn test_reconnect_now_fresh() {
            let c = ReconnectConfig::default();
            let t = ReconnectTracker::new();
            assert_eq!(decide_reconnect(&c, &t), ReconnectDecision::ReconnectNow);
        }

        #[test]
        fn test_wait_for_recent_attempt() {
            let c = ReconnectConfig {
                base_delay: Duration::from_secs(10),
                max_delay: Duration::from_secs(60),
                ..Default::default()
            };
            let mut t = ReconnectTracker::new();
            t.last_attempt = Some(Instant::now());
            t.attempt = 0;
            match decide_reconnect(&c, &t) {
                ReconnectDecision::WaitFor(d) => {
                    assert!(d <= Duration::from_secs(10));
                }
                other => panic!("Expected WaitFor, got {:?}", other),
            }
        }

        #[test]
        fn test_reconnect_now_after_delay() {
            let c = ReconnectConfig {
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(1),
                ..Default::default()
            };
            let mut t = ReconnectTracker::new();
            t.last_attempt = Some(Instant::now() - Duration::from_secs(1));
            t.attempt = 0;
            assert_eq!(decide_reconnect(&c, &t), ReconnectDecision::ReconnectNow);
        }

        #[test]
        fn test_below_max_attempts() {
            let c = ReconnectConfig {
                max_attempts: 5,
                ..Default::default()
            };
            let mut t = ReconnectTracker::new();
            t.attempt = 4;
            // No last_attempt → reconnect now
            assert_eq!(decide_reconnect(&c, &t), ReconnectDecision::ReconnectNow);
        }

        #[test]
        fn test_at_max_attempts() {
            let c = ReconnectConfig {
                max_attempts: 5,
                ..Default::default()
            };
            let mut t = ReconnectTracker::new();
            t.attempt = 5;
            assert_eq!(decide_reconnect(&c, &t), ReconnectDecision::GiveUp);
        }
    }
}
