// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Unit tests for Health Checker module

#[cfg(test)]
mod health_tests {
    use crate::health::checker::{HealthChecker, HealthConfig, HealthResult};

    #[test]
    fn test_default_config_values() {
        let config = HealthConfig::default();
        assert!(!config.endpoints.is_empty());
        assert!(config.timeout_secs >= 5);
        assert!(config.degraded_threshold_ms >= 1000);
        assert!(config.failure_threshold >= 2);
    }

    #[test]
    fn test_custom_config() {
        let config = HealthConfig {
            endpoints: vec!["https://example.com".to_string()],
            timeout_secs: 5,
            degraded_threshold_ms: 1000,
            failure_threshold: 2,
            degraded_threshold: 1,
        };

        assert_eq!(config.endpoints.len(), 1);
        assert_eq!(config.timeout_secs, 5);
    }

    #[test]
    fn test_health_checker_new() {
        let mut checker = HealthChecker::new();
        checker.reset();
    }

    #[test]
    fn test_health_checker_with_config() {
        let config = HealthConfig {
            endpoints: vec!["https://test.com".to_string()],
            timeout_secs: 3,
            degraded_threshold_ms: 500,
            failure_threshold: 1,
            degraded_threshold: 1,
        };

        let mut checker = HealthChecker::with_config(config);
        checker.reset();
    }

    #[test]
    fn test_health_result_variants() {
        let healthy = HealthResult::Healthy;
        let degraded = HealthResult::Degraded { latency_ms: 3000 };
        let dead = HealthResult::Dead {
            reason: "timeout".to_string(),
        };

        assert_eq!(healthy, HealthResult::Healthy);
        assert_ne!(healthy, degraded);
        assert_ne!(degraded, dead);

        match degraded {
            HealthResult::Degraded { latency_ms } => assert_eq!(latency_ms, 3000),
            _ => panic!("Expected Degraded"),
        }

        match dead {
            HealthResult::Dead { reason } => assert_eq!(reason, "timeout"),
            _ => panic!("Expected Dead"),
        }
    }

    #[test]
    fn test_health_result_clone() {
        let result = HealthResult::Degraded { latency_ms: 1500 };
        let cloned = result.clone();
        assert_eq!(result, cloned);
    }

    #[test]
    fn test_degraded_threshold_logic() {
        let degraded_threshold_ms: u64 = 5000;
        let degraded_count_threshold: u32 = 2;

        struct MockChecker {
            consecutive_degraded: u32,
            threshold: u32,
        }

        impl MockChecker {
            fn check_latency(&mut self, latency_ms: u64, threshold_ms: u64) -> HealthResult {
                if latency_ms > threshold_ms {
                    self.consecutive_degraded += 1;
                    if self.consecutive_degraded >= self.threshold {
                        return HealthResult::Degraded { latency_ms };
                    }
                    return HealthResult::Healthy;
                }
                self.consecutive_degraded = 0;
                HealthResult::Healthy
            }
        }

        let mut checker = MockChecker {
            consecutive_degraded: 0,
            threshold: degraded_count_threshold,
        };

        let result1 = checker.check_latency(6000, degraded_threshold_ms);
        assert_eq!(result1, HealthResult::Healthy);

        let result2 = checker.check_latency(7000, degraded_threshold_ms);
        assert_eq!(result2, HealthResult::Degraded { latency_ms: 7000 });

        let result3 = checker.check_latency(100, degraded_threshold_ms);
        assert_eq!(result3, HealthResult::Healthy);
        assert_eq!(checker.consecutive_degraded, 0);
    }

    #[test]
    fn test_latency_edge_cases() {
        // Test threshold comparison logic
        fn is_degraded(latency: u64, threshold: u64) -> bool {
            latency > threshold
        }

        let threshold = 5000u64;
        assert!(!is_degraded(5000, threshold));
        assert!(is_degraded(5001, threshold));
        assert!(!is_degraded(4999, threshold));
        assert!(!is_degraded(0, threshold));
        assert!(is_degraded(10000, threshold));
    }
}
