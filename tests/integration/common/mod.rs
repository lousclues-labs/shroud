//! Common test infrastructure for integration tests

pub mod fixtures;

use std::sync::Once;
use std::time::Duration;
use tokio::time::timeout;

/// Default timeout for async operations in tests
pub const TEST_TIMEOUT: Duration = Duration::from_secs(5);

static INIT: Once = Once::new();

/// Initialize logging for tests (call once)
pub fn init_test_logging() {
    INIT.call_once(|| {
        // Use simple stderr logging for tests
        std::env::set_var("RUST_LOG", "debug");
    });
}

/// Run an async test with timeout
#[allow(dead_code)]
pub async fn with_timeout<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    timeout(TEST_TIMEOUT, future).await.expect("Test timed out")
}
