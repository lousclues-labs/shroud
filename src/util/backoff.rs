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
