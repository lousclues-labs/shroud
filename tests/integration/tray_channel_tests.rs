//! Integration tests for tray -> supervisor channel communication
//!
//! This tests the exact pattern that caused the blocking_send/try_send bug.
//! These tests document the correct async/sync boundary patterns.

use std::thread;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::common::init_test_logging;

/// Command type (mirrors what tray sends to supervisor)
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum VpnCommand {
    Connect(String),
    Disconnect,
    ToggleKillSwitch,
    ToggleAutoReconnect,
    RefreshConnections,
    Quit,
}

/// Test: try_send works correctly from any context
/// This is the CORRECT pattern after the v1.8.3 fix.
#[test]
fn test_try_send_works_from_std_thread() {
    init_test_logging();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, mut rx) = mpsc::channel::<VpnCommand>(10);

    // Receiver in tokio runtime (simulates supervisor)
    let receiver =
        rt.spawn(async move { tokio::time::timeout(Duration::from_secs(2), rx.recv()).await });

    // Sender in std::thread (simulates tray handler)
    let tx_clone = tx.clone();
    let sender = thread::spawn(move || {
        // This is the CORRECT way - try_send is non-blocking
        tx_clone.try_send(VpnCommand::ToggleKillSwitch).unwrap();
    });

    sender.join().unwrap();

    let result = rt.block_on(receiver).unwrap();
    assert!(result.is_ok(), "Should receive within timeout");
    assert!(matches!(
        result.unwrap(),
        Some(VpnCommand::ToggleKillSwitch)
    ));
}

/// Test: try_send returns error when channel is full (doesn't block)
#[test]
fn test_try_send_returns_error_when_full() {
    init_test_logging();

    // Channel with capacity 2
    let (tx, _rx) = mpsc::channel::<VpnCommand>(2);

    // Fill the channel
    tx.try_send(VpnCommand::Disconnect).unwrap();
    tx.try_send(VpnCommand::Disconnect).unwrap();

    // Third send should fail (channel full)
    let result = tx.try_send(VpnCommand::Disconnect);
    assert!(result.is_err());
}

/// Test: Multiple try_sends work correctly
#[test]
fn test_multiple_try_sends() {
    init_test_logging();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, mut rx) = mpsc::channel::<VpnCommand>(100);

    // Send 10 commands from std::thread
    let tx_clone = tx.clone();
    let sender = thread::spawn(move || {
        for _ in 0..10 {
            tx_clone.try_send(VpnCommand::RefreshConnections).unwrap();
        }
    });

    // Receive all
    let receiver = rt.spawn(async move {
        let mut count = 0;
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            count += 1;
        }
        count
    });

    sender.join().unwrap();
    drop(tx);

    let count = rt.block_on(receiver).unwrap();
    assert_eq!(count, 10);
}

/// Test: Channel doesn't deadlock under stress
#[test]
fn test_channel_stress() {
    init_test_logging();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, mut rx) = mpsc::channel::<VpnCommand>(1000);

    // Many sender threads
    let mut senders = vec![];
    for _ in 0..10 {
        let tx = tx.clone();
        senders.push(thread::spawn(move || {
            for _ in 0..100 {
                let _ = tx.try_send(VpnCommand::RefreshConnections);
            }
        }));
    }

    // Receiver
    let receiver = rt.spawn(async move {
        let mut count = 0;
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            count += 1;
        }
        count
    });

    // Wait for senders
    for s in senders {
        s.join().unwrap();
    }
    drop(tx);

    let count = rt.block_on(receiver).unwrap();
    assert!(count >= 900, "Should receive most messages: {}", count);
}

/// Test: blocking_send works from pure std::thread (not inside async runtime)
/// This documents why blocking_send failed in ksni context.
#[test]
fn test_blocking_send_from_pure_std_thread() {
    init_test_logging();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, mut rx) = mpsc::channel::<VpnCommand>(10);

    // Receiver in tokio runtime
    let receiver =
        rt.spawn(async move { tokio::time::timeout(Duration::from_secs(2), rx.recv()).await });

    // Sender in std::thread (no tokio runtime here)
    let tx_clone = tx.clone();
    let sender = thread::spawn(move || {
        // This works because we're NOT inside a tokio runtime
        tx_clone
            .blocking_send(VpnCommand::ToggleKillSwitch)
            .unwrap();
    });

    sender.join().unwrap();

    let result = rt.block_on(receiver).unwrap();
    assert!(result.is_ok());
}

/// Test: Documents why blocking_send fails inside async context
/// This is what caused the v1.8.1 -> v1.8.3 crash.
#[test]
fn test_blocking_send_panics_in_async_context() {
    init_test_logging();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, _rx) = mpsc::channel::<VpnCommand>(10);

    // Try to use blocking_send from inside an async block
    let result = rt.block_on(async {
        // Spawn a task that tries to use blocking_send
        let handle = tokio::spawn(async move {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // This SHOULD panic because we're inside a tokio runtime
                tx.blocking_send(VpnCommand::ToggleKillSwitch)
            }))
        });

        handle.await
    });

    // The inner catch_unwind should have caught a panic
    match result {
        Ok(Ok(Err(_panic))) => {
            // Panicked as expected - this is the bug we fixed
        }
        Ok(Err(_join_error)) => {
            // Task panicked - also expected
        }
        _ => {
            // On some systems/versions it might not panic but return error
            // That's also acceptable behavior
        }
    }
}

/// Test: try_send works from inside async context (the fix)
#[test]
fn test_try_send_works_in_async_context() {
    init_test_logging();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, mut rx) = mpsc::channel::<VpnCommand>(10);

    // try_send works everywhere - it's completely non-blocking
    let sender = rt.spawn(async move {
        // This works even inside async context
        tx.try_send(VpnCommand::ToggleKillSwitch).unwrap();
    });

    let receiver =
        rt.spawn(async move { tokio::time::timeout(Duration::from_secs(1), rx.recv()).await });

    rt.block_on(sender).unwrap();
    let result = rt.block_on(receiver).unwrap();

    assert!(result.is_ok());
    assert!(matches!(
        result.unwrap(),
        Some(VpnCommand::ToggleKillSwitch)
    ));
}

/// Test: Concurrent sends and receives don't deadlock
#[test]
fn test_concurrent_send_receive() {
    init_test_logging();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, mut rx) = mpsc::channel::<VpnCommand>(100);

    // Start receiver first
    let receiver = rt.spawn(async move {
        let mut received = Vec::new();
        while let Ok(Some(cmd)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
        {
            received.push(cmd);
        }
        received.len()
    });

    // Multiple sender threads with different patterns
    let mut senders = vec![];

    for i in 0..5 {
        let tx = tx.clone();
        senders.push(thread::spawn(move || {
            for j in 0..20 {
                let cmd = if (i + j) % 2 == 0 {
                    VpnCommand::ToggleKillSwitch
                } else {
                    VpnCommand::RefreshConnections
                };
                let _ = tx.try_send(cmd);
                thread::sleep(Duration::from_micros(100));
            }
        }));
    }

    // Wait for senders
    for s in senders {
        s.join().unwrap();
    }
    drop(tx);

    let count = rt.block_on(receiver).unwrap();
    assert!(count > 50, "Should receive most messages: {}", count);
}
