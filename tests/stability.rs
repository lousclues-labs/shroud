//! Stability tests for race conditions and event handling
//!
//! These tests verify fixes for issues found by E2E/chaos testing:
//! - Time jump detection debounce
//! - Health check suspension during wake
//! - D-Bus event deduplication
//! - Reconnect race prevention
//! - Kill switch toggle protection
//!
//! Note: These tests verify the patterns and algorithms used in the main code
//! without directly importing from the binary crate.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod common;

// ============================================================================
// Time Jump Debounce Tests
// ============================================================================

/// Simulates the time jump debounce pattern used in event_loop.rs
struct TimeJumpDetector {
    last_poll_time: Instant,
    last_wake_event: Option<Instant>,
    poll_interval_secs: u64,
    cooldown_secs: u64,
}

impl TimeJumpDetector {
    fn new(poll_interval_secs: u64, cooldown_secs: u64) -> Self {
        Self {
            last_poll_time: Instant::now(),
            last_wake_event: None,
            poll_interval_secs,
            cooldown_secs,
        }
    }

    fn check_for_time_jump(&mut self) -> Option<bool> {
        let elapsed = self.last_poll_time.elapsed();
        let threshold = Duration::from_secs(self.poll_interval_secs * 3);

        if elapsed > threshold {
            // Time jump detected - check cooldown
            let should_dispatch = match self.last_wake_event {
                Some(last) => last.elapsed().as_secs() >= self.cooldown_secs,
                None => true,
            };

            self.last_poll_time = Instant::now();

            if should_dispatch {
                self.last_wake_event = Some(Instant::now());
                return Some(true); // Dispatch wake event
            } else {
                return Some(false); // In cooldown, skip
            }
        }

        self.last_poll_time = Instant::now();
        None // No time jump detected
    }
}

#[test]
fn test_time_jump_detection_normal_operation() {
    let mut detector = TimeJumpDetector::new(2, 5);

    // Normal polling - no time jump
    std::thread::sleep(Duration::from_millis(10));
    assert!(detector.check_for_time_jump().is_none());
}

#[test]
fn test_time_jump_cooldown_prevents_thrashing() {
    let mut detector = TimeJumpDetector::new(2, 5);

    // Simulate first wake event
    detector.last_wake_event = Some(Instant::now());

    // Another "time jump" immediately - should be in cooldown
    detector.last_poll_time = Instant::now() - Duration::from_secs(10);
    let result = detector.check_for_time_jump();

    // Should be Some(false) - time jump detected but in cooldown
    assert_eq!(result, Some(false));
}

#[test]
fn test_time_jump_after_cooldown_expires() {
    let mut detector = TimeJumpDetector::new(2, 5);

    // Simulate wake event 6 seconds ago (past cooldown)
    detector.last_wake_event = Some(Instant::now() - Duration::from_secs(6));

    // Another time jump - should be allowed
    detector.last_poll_time = Instant::now() - Duration::from_secs(10);
    let result = detector.check_for_time_jump();

    assert_eq!(result, Some(true)); // Should dispatch
}

// ============================================================================
// Health Check Suspension Tests
// ============================================================================

/// Simulates the health checker suspension pattern
struct HealthCheckerSim {
    suspended_until: Option<Instant>,
    consecutive_failures: u32,
}

impl HealthCheckerSim {
    fn new() -> Self {
        Self {
            suspended_until: None,
            consecutive_failures: 0,
        }
    }

    fn suspend(&mut self, duration: Duration) {
        self.suspended_until = Some(Instant::now() + duration);
        self.consecutive_failures = 0;
    }

    fn is_suspended(&self) -> bool {
        if let Some(until) = self.suspended_until {
            Instant::now() < until
        } else {
            false
        }
    }

    fn resume(&mut self) {
        self.suspended_until = None;
    }

    fn reset(&mut self) {
        self.consecutive_failures = 0;
        self.suspended_until = None;
    }

    fn check(&mut self) -> bool {
        if self.is_suspended() {
            return true; // Return healthy while suspended
        }
        // Would do actual check here
        true
    }
}

#[test]
fn test_health_checker_suspension() {
    let mut checker = HealthCheckerSim::new();

    assert!(!checker.is_suspended());

    checker.suspend(Duration::from_secs(10));
    assert!(checker.is_suspended());

    checker.resume();
    assert!(!checker.is_suspended());
}

#[test]
fn test_health_checker_suspension_expiry() {
    let mut checker = HealthCheckerSim::new();

    checker.suspend(Duration::from_millis(50));
    assert!(checker.is_suspended());

    std::thread::sleep(Duration::from_millis(60));
    assert!(!checker.is_suspended());
}

#[test]
fn test_health_checker_reset_clears_suspension() {
    let mut checker = HealthCheckerSim::new();

    checker.suspend(Duration::from_secs(10));
    assert!(checker.is_suspended());

    checker.reset();
    assert!(!checker.is_suspended());
}

#[test]
fn test_health_check_returns_healthy_when_suspended() {
    let mut checker = HealthCheckerSim::new();
    checker.suspend(Duration::from_secs(10));

    assert!(checker.check());
}

// ============================================================================
// D-Bus Event Deduplication Tests
// ============================================================================

/// Simulates the D-Bus event deduplication pattern
struct EventDeduplicator {
    recent_events: HashMap<(String, String), Instant>,
    dedup_window_ms: u64,
}

impl EventDeduplicator {
    fn new(dedup_window_ms: u64) -> Self {
        Self {
            recent_events: HashMap::new(),
            dedup_window_ms,
        }
    }

    fn should_process_event(&mut self, vpn_name: &str, event_type: &str) -> bool {
        // Filter out "unknown" VPN names
        if vpn_name == "unknown" {
            return false;
        }

        let key = (vpn_name.to_string(), event_type.to_string());
        let now = Instant::now();

        if let Some(last_time) = self.recent_events.get(&key) {
            if now.duration_since(*last_time).as_millis() < self.dedup_window_ms as u128 {
                return false; // Duplicate, skip
            }
        }

        // Clean up old entries
        let cleanup_threshold = self.dedup_window_ms * 2;
        self.recent_events
            .retain(|_, v| now.duration_since(*v).as_millis() < cleanup_threshold as u128);

        self.recent_events.insert(key, now);
        true
    }
}

#[test]
fn test_event_dedup_allows_first_event() {
    let mut dedup = EventDeduplicator::new(500);

    assert!(dedup.should_process_event("my-vpn", "activated"));
}

#[test]
fn test_event_dedup_blocks_immediate_duplicate() {
    let mut dedup = EventDeduplicator::new(500);

    assert!(dedup.should_process_event("my-vpn", "activated"));
    assert!(!dedup.should_process_event("my-vpn", "activated"));
}

#[test]
fn test_event_dedup_allows_different_events() {
    let mut dedup = EventDeduplicator::new(500);

    assert!(dedup.should_process_event("my-vpn", "activated"));
    assert!(dedup.should_process_event("my-vpn", "deactivated"));
}

#[test]
fn test_event_dedup_allows_different_vpns() {
    let mut dedup = EventDeduplicator::new(500);

    assert!(dedup.should_process_event("vpn-1", "activated"));
    assert!(dedup.should_process_event("vpn-2", "activated"));
}

#[test]
fn test_event_dedup_filters_unknown_vpn() {
    let mut dedup = EventDeduplicator::new(500);

    assert!(!dedup.should_process_event("unknown", "activated"));
}

#[test]
fn test_event_dedup_allows_after_window_expires() {
    let mut dedup = EventDeduplicator::new(50);

    assert!(dedup.should_process_event("my-vpn", "activated"));

    std::thread::sleep(Duration::from_millis(60));

    assert!(dedup.should_process_event("my-vpn", "activated"));
}

// ============================================================================
// Reconnect Race Prevention Tests
// ============================================================================

#[test]
fn test_reconnect_atomic_lock() {
    static IN_PROGRESS: AtomicBool = AtomicBool::new(false);

    // First attempt should succeed
    let first = !IN_PROGRESS.swap(true, Ordering::SeqCst);
    assert!(first);

    // Second attempt should fail
    let second = !IN_PROGRESS.swap(true, Ordering::SeqCst);
    assert!(!second);

    // Release and try again
    IN_PROGRESS.store(false, Ordering::SeqCst);
    let third = !IN_PROGRESS.swap(true, Ordering::SeqCst);
    assert!(third);
}

#[test]
fn test_reconnect_debounce_pattern() {
    let mut last_reconnect: Option<Instant> = None;
    let debounce_secs = 5u64;

    // First reconnect should proceed
    let should_proceed_1 = match last_reconnect {
        Some(last) => last.elapsed().as_secs() >= debounce_secs,
        None => true,
    };
    assert!(should_proceed_1);
    last_reconnect = Some(Instant::now());

    // Immediate second attempt should be blocked
    let should_proceed_2 = match last_reconnect {
        Some(last) => last.elapsed().as_secs() >= debounce_secs,
        None => true,
    };
    assert!(!should_proceed_2);
}

#[test]
fn test_concurrent_reconnect_prevention() {
    let in_progress = Arc::new(AtomicBool::new(false));
    let successful_starts = Arc::new(AtomicU32::new(0));
    let blocked_count = Arc::new(AtomicU32::new(0));

    let mut handles = vec![];

    for _ in 0..5 {
        let in_progress = Arc::clone(&in_progress);
        let successful_starts = Arc::clone(&successful_starts);
        let blocked_count = Arc::clone(&blocked_count);

        handles.push(std::thread::spawn(move || {
            if !in_progress.swap(true, Ordering::SeqCst) {
                successful_starts.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(50));
                in_progress.store(false, Ordering::SeqCst);
            } else {
                blocked_count.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // With concurrent attempts, some should be blocked
    let blocked = blocked_count.load(Ordering::SeqCst);
    let started = successful_starts.load(Ordering::SeqCst);

    assert!(started >= 1, "At least one should have started");
    assert!(
        blocked + started == 5,
        "All 5 attempts should be accounted for"
    );
}

// ============================================================================
// Kill Switch Toggle Protection Tests
// ============================================================================

#[test]
fn test_toggle_atomic_lock() {
    static TOGGLE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

    let first = !TOGGLE_IN_PROGRESS.swap(true, Ordering::SeqCst);
    assert!(first);

    let second = !TOGGLE_IN_PROGRESS.swap(true, Ordering::SeqCst);
    assert!(!second);

    TOGGLE_IN_PROGRESS.store(false, Ordering::SeqCst);

    let third = !TOGGLE_IN_PROGRESS.swap(true, Ordering::SeqCst);
    assert!(third);
}

#[test]
fn test_toggle_cooldown_pattern() {
    let mut last_toggle: Option<Instant> = None;
    let cooldown_ms = 500u64;

    // First toggle should proceed
    let can_toggle_1 = match last_toggle {
        Some(last) => last.elapsed().as_millis() >= cooldown_ms as u128,
        None => true,
    };
    assert!(can_toggle_1);
    last_toggle = Some(Instant::now());

    // Immediate second toggle should be blocked
    let can_toggle_2 = match last_toggle {
        Some(last) => last.elapsed().as_millis() >= cooldown_ms as u128,
        None => true,
    };
    assert!(!can_toggle_2);
}

// ============================================================================
// Scopeguard Pattern Tests
// ============================================================================

#[test]
fn test_scopeguard_cleanup_on_normal_exit() {
    let cleanup_called = Arc::new(AtomicBool::new(false));

    {
        let cleanup_called = Arc::clone(&cleanup_called);
        let _guard = scopeguard::guard((), move |_| {
            cleanup_called.store(true, Ordering::SeqCst);
        });
        // Normal exit from scope
    }

    assert!(cleanup_called.load(Ordering::SeqCst));
}

#[test]
fn test_scopeguard_cleanup_on_early_return() {
    let cleanup_called = Arc::new(AtomicBool::new(false));

    fn inner(cleanup_called: Arc<AtomicBool>) -> bool {
        let _guard = scopeguard::guard((), move |_| {
            cleanup_called.store(true, Ordering::SeqCst);
        });

        // Early return
        return true;
    }

    inner(Arc::clone(&cleanup_called));
    assert!(cleanup_called.load(Ordering::SeqCst));
}

// ============================================================================
// State Consistency Tests
// ============================================================================

#[test]
fn test_instant_tracking() {
    let mut last_event: Option<Instant> = None;

    assert!(last_event.is_none());

    last_event = Some(Instant::now());
    assert!(last_event.is_some());

    std::thread::sleep(Duration::from_millis(10));
    let elapsed = last_event.unwrap().elapsed();
    assert!(elapsed.as_millis() >= 10);
}

#[test]
fn test_duration_calculations() {
    let secs = Duration::from_secs(5);
    let millis = Duration::from_millis(500);

    assert_eq!(secs.as_millis(), 5000);
    assert_eq!(millis.as_secs(), 0);
    assert_eq!(millis.as_millis(), 500);
}
