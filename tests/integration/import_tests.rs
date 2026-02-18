// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Commercial
// Copyright (C) 2026 Louis Nelson Jr. <https://lousclues.com>

//! Integration tests for VPN config import.
//!
//! These tests require NetworkManager and are ignored by default.

use std::io::Write;

#[tokio::test]
#[ignore = "requires NetworkManager and nmcli"]
async fn test_import_wireguard_config() {
    let mut file = tempfile::Builder::new().suffix(".conf").tempfile().unwrap();
    writeln!(
        file,
        "[Interface]\nPrivateKey = abc\n[Peer]\nPublicKey = def\nEndpoint = 1.2.3.4:51820\n"
    )
    .unwrap();

    let _ = std::process::Command::new("./target/debug/shroud")
        .args(["import", file.path().to_str().unwrap()])
        .status();
}

#[tokio::test]
#[ignore = "requires NetworkManager and nmcli"]
async fn test_import_openvpn_config() {
    let mut file = tempfile::Builder::new().suffix(".ovpn").tempfile().unwrap();
    writeln!(file, "remote vpn.example.com 1194\n<ca>abc</ca>\n").unwrap();

    let _ = std::process::Command::new("./target/debug/shroud")
        .args(["import", file.path().to_str().unwrap()])
        .status();
}

#[tokio::test]
#[ignore = "requires NetworkManager and nmcli"]
async fn test_import_directory() {
    let dir = tempfile::tempdir().unwrap();
    let wg_path = dir.path().join("wg.conf");
    let ovpn_path = dir.path().join("vpn.ovpn");

    std::fs::write(
        &wg_path,
        "[Interface]\nPrivateKey = abc\n[Peer]\nPublicKey = def\nEndpoint = 1.2.3.4:51820\n",
    )
    .unwrap();
    std::fs::write(&ovpn_path, "remote vpn.example.com 1194\n<ca>abc</ca>\n").unwrap();

    let _ = std::process::Command::new("./target/debug/shroud")
        .args(["import", dir.path().to_str().unwrap()])
        .status();
}

#[tokio::test]
#[ignore = "requires NetworkManager and nmcli"]
async fn test_import_dry_run() {
    let mut file = tempfile::Builder::new().suffix(".ovpn").tempfile().unwrap();
    writeln!(file, "remote vpn.example.com 1194\n").unwrap();

    let _ = std::process::Command::new("./target/debug/shroud")
        .args(["import", file.path().to_str().unwrap(), "--dry-run"])
        .status();
}

#[tokio::test]
#[ignore = "requires NetworkManager and nmcli"]
async fn test_import_force_overwrite() {
    let mut file = tempfile::Builder::new().suffix(".ovpn").tempfile().unwrap();
    writeln!(file, "remote vpn.example.com 1194\n").unwrap();

    let _ = std::process::Command::new("./target/debug/shroud")
        .args(["import", file.path().to_str().unwrap(), "--force"])
        .status();
}
