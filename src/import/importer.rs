use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::process::Command;
use walkdir::WalkDir;

use super::detector::{detect_config_type, VpnConfigType};
use super::validator::{validate, ValidationError};

#[derive(Error, Debug)]
pub enum ImportError {
    #[error("File not found: {0}")]
    NotFound(PathBuf),

    #[error("Unknown config format: {0}")]
    UnknownFormat(PathBuf),

    #[error("Validation failed: {0}")]
    ValidationFailed(#[from] ValidationError),

    #[error("NetworkManager import failed: {0}")]
    NmcliError(String),

    #[error("Connection already exists: {0}")]
    AlreadyExists(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("NetworkManager not running")]
    NetworkManagerNotRunning,
}

#[derive(Debug, Clone)]
pub struct ImportResult {
    pub name: String,
    pub config_type: VpnConfigType,
    pub path: PathBuf,
}

#[derive(Debug, Default)]
pub struct ImportSummary {
    pub imported: Vec<ImportResult>,
    pub skipped: Vec<(PathBuf, String)>,
    pub failed: Vec<(PathBuf, ImportError)>,
}

impl ImportSummary {
    pub fn total_processed(&self) -> usize {
        self.imported.len() + self.skipped.len() + self.failed.len()
    }
}

fn nmcli_command() -> Command {
    if let Ok(path) = std::env::var("SHROUD_NMCLI") {
        Command::new(path)
    } else {
        Command::new("nmcli")
    }
}

async fn nmcli_output(args: &[&str]) -> std::io::Result<std::process::Output> {
    if let Ok(path) = std::env::var("SHROUD_NMCLI") {
        let output = Command::new(&path).args(args).output().await;
        match output {
            Ok(out) => Ok(out),
            Err(_) => Command::new("sh").arg(path).args(args).output().await,
        }
    } else {
        Command::new("nmcli").args(args).output().await
    }
}

async fn nmcli_output_with_path(
    args: &[&str],
    path: &Path,
) -> std::io::Result<std::process::Output> {
    if let Ok(cmd_path) = std::env::var("SHROUD_NMCLI") {
        let output = Command::new(&cmd_path).args(args).arg(path).output().await;
        match output {
            Ok(out) => Ok(out),
            Err(_) => {
                Command::new("sh")
                    .arg(cmd_path)
                    .args(args)
                    .arg(path)
                    .output()
                    .await
            }
        }
    } else {
        Command::new("nmcli").args(args).arg(path).output().await
    }
}

/// Check if NetworkManager is running
pub async fn check_networkmanager() -> Result<(), ImportError> {
    let output = nmcli_output(&["general", "status"])
        .await
        .map_err(|_| ImportError::NetworkManagerNotRunning)?;

    if !output.status.success() {
        return Err(ImportError::NetworkManagerNotRunning);
    }

    Ok(())
}

/// Check if a connection with this name already exists
pub async fn connection_exists(name: &str) -> bool {
    let output = nmcli_output(&["-t", "-f", "NAME", "connection", "show"]).await;

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.lines().any(|l| l.trim() == name)
        }
        Err(_) => false,
    }
}

/// Import a single config file
pub async fn import_file(
    path: &Path,
    custom_name: Option<&str>,
    force: bool,
    forced_type: Option<VpnConfigType>,
) -> Result<ImportResult, ImportError> {
    if !path.exists() {
        return Err(ImportError::NotFound(path.to_path_buf()));
    }

    let config_type = match forced_type {
        Some(t) => t,
        None => detect_config_type(path)
            .ok_or_else(|| ImportError::UnknownFormat(path.to_path_buf()))?,
    };

    validate(path, config_type)?;

    let default_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("vpn")
        .to_string();

    let conn_name = custom_name.unwrap_or(&default_name);

    if connection_exists(conn_name).await && !force {
        return Err(ImportError::AlreadyExists(conn_name.to_string()));
    }

    if force && connection_exists(conn_name).await {
        let _ = nmcli_command()
            .args(["connection", "delete", conn_name])
            .output()
            .await;
    }

    let nmcli_type = match config_type {
        VpnConfigType::WireGuard => "wireguard",
        VpnConfigType::OpenVpn => "openvpn",
    };

    let output =
        nmcli_output_with_path(&["connection", "import", "type", nmcli_type, "file"], path)
            .await
            .map_err(|e| ImportError::NmcliError(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ImportError::NmcliError(stderr.trim().to_string()));
    }

    if let Some(name) = custom_name {
        if name != default_name {
            let _ =
                nmcli_output(&["connection", "modify", &default_name, "connection.id", name]).await;
        }
    }

    Ok(ImportResult {
        name: custom_name.unwrap_or(&default_name).to_string(),
        config_type,
        path: path.to_path_buf(),
    })
}

/// Import all configs from a directory
pub async fn import_directory(
    dir: &Path,
    recursive: bool,
    force: bool,
    forced_type: Option<VpnConfigType>,
) -> ImportSummary {
    let mut summary = ImportSummary::default();

    let walker = if recursive {
        WalkDir::new(dir)
    } else {
        WalkDir::new(dir).max_depth(1)
    };

    for entry in walker
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();

        let detected = forced_type.or_else(|| detect_config_type(path));
        if detected.is_none() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "conf" || ext == "ovpn" {
                summary.skipped.push((
                    path.to_path_buf(),
                    "Could not detect VPN config type".into(),
                ));
            }
            continue;
        }

        match import_file(path, None, force, forced_type).await {
            Ok(result) => summary.imported.push(result),
            Err(ImportError::AlreadyExists(name)) => {
                summary.skipped.push((
                    path.to_path_buf(),
                    format!(
                        "Connection '{}' already exists (use --force to overwrite)",
                        name
                    ),
                ));
            }
            Err(e) => summary.failed.push((path.to_path_buf(), e)),
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::OnceLock;
    use tokio::sync::Mutex;

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn write_temp(contents: &str, ext: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::Builder::new().suffix(ext).tempfile().unwrap();
        write!(file, "{}", contents).unwrap();
        file
    }

    fn make_nmcli_stub(output: &str) -> tempfile::NamedTempFile {
        let base_dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("test-bin");
        std::fs::create_dir_all(&base_dir).unwrap();
        let mut file = tempfile::Builder::new()
            .prefix("nmcli-mock-")
            .tempfile_in(base_dir)
            .unwrap();
        writeln!(file, "#!/bin/sh").unwrap();
        writeln!(file, "echo \"{}\"", output.replace('"', "\\\"")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        file
    }

    #[tokio::test]
    async fn test_connection_exists() {
        let lock = ENV_LOCK.get_or_init(|| Mutex::new(()));
        let _guard = lock.lock().await;
        let stub = make_nmcli_stub("demo-vpn");
        std::env::set_var("SHROUD_NMCLI", stub.path());
        assert!(connection_exists("demo-vpn").await);
        assert!(!connection_exists("other").await);
        std::env::remove_var("SHROUD_NMCLI");
    }

    #[tokio::test]
    async fn test_import_file_with_force() {
        let lock = ENV_LOCK.get_or_init(|| Mutex::new(()));
        let _guard = lock.lock().await;
        let stub = make_nmcli_stub("");
        std::env::set_var("SHROUD_NMCLI", stub.path());

        let file = write_temp(
            "[Interface]\nPrivateKey = abc\n[Peer]\nPublicKey = def\nEndpoint = 1.2.3.4:51820\n",
            ".conf",
        );

        let result = import_file(file.path(), Some("demo"), true, None).await;
        assert!(result.is_ok());
        std::env::remove_var("SHROUD_NMCLI");
    }
}
