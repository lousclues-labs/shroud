use rand::Rng;
use std::time::Duration;

/// Linear backoff capped at max_secs.
pub fn linear_backoff_secs(base_secs: u64, max_secs: u64, attempt: u32) -> Duration {
    let delay = base_secs.saturating_mul(attempt as u64);
    Duration::from_secs(std::cmp::min(delay, max_secs))
}

/// Optional jitter helper.
pub fn jitter_millis(max_ms: u64) -> Duration {
    if max_ms == 0 {
        Duration::from_millis(0)
    } else {
        let j = rand::rng().random_range(0..max_ms);
        Duration::from_millis(j)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_attempt_no_delay() {
        // attempt 0 → base * 0 = 0s
        assert_eq!(linear_backoff_secs(2, 30, 0), Duration::from_secs(0));
    }

    #[test]
    fn test_linear_growth() {
        assert_eq!(linear_backoff_secs(2, 100, 1), Duration::from_secs(2));
        assert_eq!(linear_backoff_secs(2, 100, 2), Duration::from_secs(4));
        assert_eq!(linear_backoff_secs(2, 100, 4), Duration::from_secs(8));
    }

    #[test]
    fn test_capped_at_max() {
        assert_eq!(linear_backoff_secs(2, 10, 100), Duration::from_secs(10));
    }

    #[test]
    fn test_saturating_mul_no_panic() {
        // u64::MAX attempt should saturate, not panic
        let result = linear_backoff_secs(2, 30, u32::MAX);
        assert_eq!(result, Duration::from_secs(30));
    }

    #[test]
    fn test_zero_base() {
        assert_eq!(linear_backoff_secs(0, 30, 5), Duration::from_secs(0));
    }

    #[test]
    fn test_jitter_zero() {
        assert_eq!(jitter_millis(0), Duration::from_millis(0));
    }

    #[test]
    fn test_jitter_bounded() {
        for _ in 0..100 {
            let j = jitter_millis(500);
            assert!(j < Duration::from_millis(500));
        }
    }
}
