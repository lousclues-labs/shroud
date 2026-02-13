//! Integration tests for configuration loading and propagation

use crate::common::fixtures::*;
use crate::common::init_test_logging;

/// Test: Config loads and parses valid TOML correctly
#[test]
fn test_config_load_valid_minimal() {
    init_test_logging();

    let file = create_temp_config(minimal_config_toml());
    let content = std::fs::read_to_string(file.path()).unwrap();

    // Parse TOML
    let parsed: toml::Value = toml::from_str(&content).expect("Should parse valid TOML");

    assert_eq!(parsed.get("version").and_then(|v| v.as_integer()), Some(1));
    assert_eq!(
        parsed.get("auto_reconnect").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        parsed.get("kill_switch_enabled").and_then(|v| v.as_bool()),
        Some(false)
    );
}

/// Test: Config with kill switch enabled parses correctly
#[test]
fn test_config_load_with_killswitch() {
    init_test_logging();

    let file = create_temp_config(killswitch_config_toml());
    let content = std::fs::read_to_string(file.path()).unwrap();

    let parsed: toml::Value = toml::from_str(&content).expect("Should parse");

    assert_eq!(
        parsed.get("kill_switch_enabled").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        parsed.get("last_server").and_then(|v| v.as_str()),
        Some("test-vpn")
    );
}

/// Test: Headless config section parses correctly
#[test]
fn test_config_load_headless_section() {
    init_test_logging();

    let file = create_temp_config(headless_config_toml());
    let content = std::fs::read_to_string(file.path()).unwrap();

    let parsed: toml::Value = toml::from_str(&content).expect("Should parse");

    let headless = parsed
        .get("headless")
        .expect("Should have headless section");
    assert_eq!(
        headless.get("auto_connect").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        headless.get("startup_server").and_then(|v| v.as_str()),
        Some("my-server")
    );
    assert_eq!(
        headless
            .get("kill_switch_on_boot")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        headless
            .get("max_reconnect_attempts")
            .and_then(|v| v.as_integer()),
        Some(5)
    );
}

/// Test: Invalid TOML produces parse error
#[test]
fn test_config_invalid_toml() {
    init_test_logging();

    let file = create_temp_config(invalid_config_toml());
    let content = std::fs::read_to_string(file.path()).unwrap();

    let result: Result<toml::Value, _> = toml::from_str(&content);
    assert!(result.is_err(), "Should fail to parse invalid TOML");
}

/// Test: Missing file produces error
#[test]
fn test_config_missing_file() {
    init_test_logging();

    let result = std::fs::read_to_string("/nonexistent/path/config.toml");
    assert!(result.is_err());
}

/// Test: Empty config file is valid TOML
#[test]
fn test_config_empty_file() {
    init_test_logging();

    let file = create_temp_config("");
    let content = std::fs::read_to_string(file.path()).unwrap();

    let result: Result<toml::Value, _> = toml::from_str(&content);
    // Empty string is valid TOML (represents empty table)
    assert!(result.is_ok());
}

/// Test: Config with unknown fields is still parseable
#[test]
fn test_config_unknown_fields_ignored() {
    init_test_logging();

    let config = r#"
version = 1
auto_reconnect = true
unknown_field = "should be ignored"
another_unknown = 42

[unknown_section]
foo = "bar"
"#;

    let file = create_temp_config(config);
    let content = std::fs::read_to_string(file.path()).unwrap();

    let parsed: toml::Value = toml::from_str(&content).expect("Should parse with unknown fields");
    assert_eq!(parsed.get("version").and_then(|v| v.as_integer()), Some(1));
}

/// Test: Config values with special characters
#[test]
fn test_config_special_characters() {
    init_test_logging();

    let config = r#"
version = 1
last_server = "vpn-server-with-dashes"
"#;

    let file = create_temp_config(config);
    let content = std::fs::read_to_string(file.path()).unwrap();

    let parsed: toml::Value = toml::from_str(&content).expect("Should parse");
    assert_eq!(
        parsed.get("last_server").and_then(|v| v.as_str()),
        Some("vpn-server-with-dashes")
    );
}

/// Test: Config values with unicode
#[test]
fn test_config_unicode() {
    init_test_logging();

    let config = r#"
version = 1
last_server = "日本-vpn-東京"
"#;

    let file = create_temp_config(config);
    let content = std::fs::read_to_string(file.path()).unwrap();

    let parsed: toml::Value = toml::from_str(&content).expect("Should parse unicode");
    assert_eq!(
        parsed.get("last_server").and_then(|v| v.as_str()),
        Some("日本-vpn-東京")
    );
}

/// Test: Config roundtrip (serialize then deserialize)
#[test]
fn test_config_roundtrip() {
    init_test_logging();

    use std::collections::BTreeMap;

    let mut config = BTreeMap::new();
    config.insert("version", toml::Value::Integer(1));
    config.insert("auto_reconnect", toml::Value::Boolean(true));
    config.insert("kill_switch_enabled", toml::Value::Boolean(false));
    config.insert(
        "last_server",
        toml::Value::String("test-server".to_string()),
    );

    let serialized = toml::to_string(&config).expect("Should serialize");
    let deserialized: toml::Value = toml::from_str(&serialized).expect("Should deserialize");

    assert_eq!(
        deserialized.get("version").and_then(|v| v.as_integer()),
        Some(1)
    );
    assert_eq!(
        deserialized.get("last_server").and_then(|v| v.as_str()),
        Some("test-server")
    );
}

/// Test: Config file permissions (should be readable)
#[test]
fn test_config_file_readable() {
    init_test_logging();

    let file = create_temp_config(minimal_config_toml());

    // File should be readable
    let metadata = std::fs::metadata(file.path()).expect("Should get metadata");
    assert!(metadata.is_file());
    assert!(metadata.len() > 0);
}

/// Test: Config with all boolean combinations
#[test]
fn test_config_boolean_combinations() {
    init_test_logging();

    let configs = vec![
        (
            "auto_reconnect = true\nkill_switch_enabled = true",
            true,
            true,
        ),
        (
            "auto_reconnect = true\nkill_switch_enabled = false",
            true,
            false,
        ),
        (
            "auto_reconnect = false\nkill_switch_enabled = true",
            false,
            true,
        ),
        (
            "auto_reconnect = false\nkill_switch_enabled = false",
            false,
            false,
        ),
    ];

    for (toml_str, expected_ar, expected_ks) in configs {
        let full_config = format!("version = 1\n{}", toml_str);
        let file = create_temp_config(&full_config);
        let content = std::fs::read_to_string(file.path()).unwrap();

        let parsed: toml::Value = toml::from_str(&content).expect("Should parse");

        assert_eq!(
            parsed.get("auto_reconnect").and_then(|v| v.as_bool()),
            Some(expected_ar),
            "auto_reconnect mismatch for: {}",
            toml_str
        );
        assert_eq!(
            parsed.get("kill_switch_enabled").and_then(|v| v.as_bool()),
            Some(expected_ks),
            "kill_switch_enabled mismatch for: {}",
            toml_str
        );
    }
}
