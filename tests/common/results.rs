// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! JSON test result output for CI

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Serialize, Deserialize)]
pub struct TestSuiteResult {
    pub suite: String,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub duration_ms: u64,
    pub tests: Vec<TestResult>,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub result: TestOutcome,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestOutcome {
    Pass,
    Fail,
    Skip,
}

impl TestSuiteResult {
    pub fn new(suite: &str) -> Self {
        Self {
            suite: suite.to_string(),
            passed: 0,
            failed: 0,
            skipped: 0,
            duration_ms: 0,
            tests: Vec::new(),
            timestamp: format!(
                "{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
            ),
        }
    }

    pub fn add_pass(&mut self, name: &str, duration: Duration) {
        self.passed += 1;
        self.tests.push(TestResult {
            name: name.to_string(),
            result: TestOutcome::Pass,
            duration_ms: duration.as_millis() as u64,
            message: None,
        });
    }

    pub fn add_fail(&mut self, name: &str, duration: Duration, message: &str) {
        self.failed += 1;
        self.tests.push(TestResult {
            name: name.to_string(),
            result: TestOutcome::Fail,
            duration_ms: duration.as_millis() as u64,
            message: Some(message.to_string()),
        });
    }

    pub fn add_skip(&mut self, name: &str, reason: &str) {
        self.skipped += 1;
        self.tests.push(TestResult {
            name: name.to_string(),
            result: TestOutcome::Skip,
            duration_ms: 0,
            message: Some(reason.to_string()),
        });
    }

    pub fn write_to_file(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        let mut file = File::create(path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    pub fn total(&self) -> u32 {
        self.passed + self.failed + self.skipped
    }

    pub fn success(&self) -> bool {
        self.failed == 0
    }
}

/// Test timer for measuring test duration
pub struct TestTimer {
    start: Instant,
    name: String,
}

impl TestTimer {
    pub fn start(name: &str) -> Self {
        Self {
            start: Instant::now(),
            name: name.to_string(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn finish(self) -> (String, Duration) {
        let elapsed = self.start.elapsed();
        (self.name, elapsed)
    }
}

/// Macro for running a test and recording result
#[macro_export]
macro_rules! run_test {
    ($results:expr, $name:expr, $test:expr) => {{
        let timer = $crate::common::results::TestTimer::start($name);
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $test)) {
            Ok(Ok(())) => {
                let (name, duration) = timer.finish();
                $results.add_pass(&name, duration);
                println!("  ✓ {}", $name);
            }
            Ok(Err(e)) => {
                let (name, duration) = timer.finish();
                $results.add_fail(&name, duration, &e.to_string());
                println!("  ✗ {} - {}", $name, e);
            }
            Err(e) => {
                let (name, duration) = timer.finish();
                let msg = if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Test panicked".to_string()
                };
                $results.add_fail(&name, duration, &msg);
                println!("  ✗ {} - PANIC: {}", $name, msg);
            }
        }
    }};
}
