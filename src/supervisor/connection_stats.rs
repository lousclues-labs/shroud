//! Connection statistics tracking — pure state, no I/O.

use std::time::Instant;

/// Tracks connection lifecycle statistics.
#[derive(Debug, Clone, Default)]
pub struct ConnectionStats {
    pub total_connects: u64,
    pub total_disconnects: u64,
    pub failed_connects: u64,
    pub reconnects: u64,
    pub total_uptime_secs: u64,
    pub current_session_start: Option<Instant>,
}

impl ConnectionStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_connect(&mut self) {
        self.total_connects += 1;
        self.current_session_start = Some(Instant::now());
    }

    pub fn record_disconnect(&mut self) {
        self.total_disconnects += 1;
        if let Some(start) = self.current_session_start.take() {
            self.total_uptime_secs += start.elapsed().as_secs();
        }
    }

    pub fn record_failed_connect(&mut self) {
        self.failed_connects += 1;
    }

    pub fn record_reconnect(&mut self) {
        self.reconnects += 1;
    }

    pub fn current_session_duration(&self) -> Option<std::time::Duration> {
        self.current_session_start.map(|s| s.elapsed())
    }

    pub fn success_rate(&self) -> f64 {
        let total = self.total_connects + self.failed_connects;
        if total == 0 {
            return 1.0;
        }
        self.total_connects as f64 / total as f64
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_new() {
        let s = ConnectionStats::new();
        assert_eq!(s.total_connects, 0);
        assert_eq!(s.total_disconnects, 0);
        assert_eq!(s.failed_connects, 0);
        assert_eq!(s.reconnects, 0);
        assert_eq!(s.total_uptime_secs, 0);
        assert!(s.current_session_start.is_none());
    }

    #[test]
    fn test_record_connect() {
        let mut s = ConnectionStats::new();
        s.record_connect();
        assert_eq!(s.total_connects, 1);
        assert!(s.current_session_start.is_some());
    }

    #[test]
    fn test_record_disconnect_accumulates_uptime() {
        let mut s = ConnectionStats::new();
        s.record_connect();
        std::thread::sleep(Duration::from_millis(10));
        s.record_disconnect();
        assert_eq!(s.total_disconnects, 1);
        assert!(s.current_session_start.is_none());
        // uptime may be 0 if < 1s, that's fine
    }

    #[test]
    fn test_disconnect_without_connect() {
        let mut s = ConnectionStats::new();
        s.record_disconnect();
        assert_eq!(s.total_disconnects, 1);
        assert_eq!(s.total_uptime_secs, 0);
    }

    #[test]
    fn test_record_failed() {
        let mut s = ConnectionStats::new();
        s.record_failed_connect();
        s.record_failed_connect();
        assert_eq!(s.failed_connects, 2);
    }

    #[test]
    fn test_record_reconnect() {
        let mut s = ConnectionStats::new();
        s.record_reconnect();
        assert_eq!(s.reconnects, 1);
    }

    #[test]
    fn test_current_session_duration_none() {
        let s = ConnectionStats::new();
        assert!(s.current_session_duration().is_none());
    }

    #[test]
    fn test_current_session_duration_some() {
        let mut s = ConnectionStats::new();
        s.record_connect();
        std::thread::sleep(Duration::from_millis(20));
        let d = s.current_session_duration();
        assert!(d.is_some());
        assert!(d.unwrap() >= Duration::from_millis(20));
    }

    #[test]
    fn test_success_rate_no_attempts() {
        let s = ConnectionStats::new();
        assert!((s.success_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_success_rate_all_success() {
        let mut s = ConnectionStats::new();
        s.record_connect();
        s.record_connect();
        assert!((s.success_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_success_rate_mixed() {
        let mut s = ConnectionStats::new();
        s.record_connect();
        s.record_connect();
        s.record_failed_connect();
        // 2 / (2+1) = 0.666…
        assert!((s.success_rate() - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_success_rate_all_failed() {
        let mut s = ConnectionStats::new();
        s.record_failed_connect();
        s.record_failed_connect();
        assert!((s.success_rate()).abs() < f64::EPSILON);
    }

    #[test]
    fn test_lifecycle() {
        let mut s = ConnectionStats::new();
        s.record_connect();
        s.record_disconnect();
        s.record_failed_connect();
        s.record_connect();
        s.record_reconnect();
        s.record_disconnect();

        assert_eq!(s.total_connects, 2);
        assert_eq!(s.total_disconnects, 2);
        assert_eq!(s.failed_connects, 1);
        assert_eq!(s.reconnects, 1);
    }
}
