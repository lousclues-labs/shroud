use std::path::Path;

use log::{error, warn};

use crate::import::{
    check_networkmanager, detect_config_type, import_directory, import_file, ImportOptions,
    ImportSummary, VpnConfigType,
};
use crate::ipc::client::send_command;
use crate::ipc::protocol::IpcCommand;

/// Execute the import command
pub async fn execute(options: ImportOptions) -> i32 {
    if let Err(e) = check_networkmanager().await {
        if options.json {
            println!(r#"{{"success": false, "error": "{}"}}"#, e);
        } else {
            error!(
                "NetworkManager is not running. Start it with: sudo systemctl start NetworkManager"
            );
        }
        return 1;
    }

    let path = &options.path;

    let summary = if path.is_file() {
        import_single_file(&options).await
    } else if path.is_dir() {
        import_from_directory(&options).await
    } else {
        if options.json {
            println!(
                r#"{{"success": false, "error": "Path not found: {}"}}"#,
                path.display()
            );
        } else {
            error!("Path not found: {}", path.display());
        }
        return 1;
    };

    if options.json {
        print_json_summary(&summary);
    } else if !options.quiet {
        print_summary(&summary);
    }

    if options.connect && !summary.imported.is_empty() {
        let first_imported = &summary.imported[0];
        if !options.quiet {
            println!("\nConnecting to {}...", first_imported.name);
        }

        if let Err(e) = connect_to_vpn(&first_imported.name).await {
            if !options.quiet {
                error!("Failed to connect: {}", e);
            }
            return 1;
        }

        if !options.quiet {
            println!("✓ Connected to {}", first_imported.name);
        }
    }

    if summary.failed.is_empty() {
        0
    } else if summary.imported.is_empty() {
        1
    } else {
        2
    }
}

async fn import_single_file(options: &ImportOptions) -> ImportSummary {
    let mut summary = ImportSummary::default();

    if options.dry_run {
        if let Some(config_type) = options
            .config_type
            .or_else(|| detect_config_type(&options.path))
        {
            let name = options
                .name
                .as_deref()
                .or_else(|| options.path.file_stem().and_then(|s| s.to_str()))
                .unwrap_or("vpn");

            if !options.quiet && !options.json {
                println!(
                    "Would import: {} ({}) as '{}'",
                    options.path.display(),
                    config_type,
                    name
                );
            }

            summary.imported.push(crate::import::ImportResult {
                name: name.to_string(),
                config_type,
                path: options.path.clone(),
            });
        } else {
            summary
                .skipped
                .push((options.path.clone(), "Unknown config format".into()));
        }
        return summary;
    }

    match import_file(
        &options.path,
        options.name.as_deref(),
        options.force,
        options.config_type,
    )
    .await
    {
        Ok(result) => {
            if !options.quiet && !options.json {
                println!("✓ Imported: {} ({})", result.name, result.config_type);
            }
            summary.imported.push(result);
        }
        Err(e) => {
            if !options.quiet && !options.json {
                error!("✗ Failed: {} - {}", options.path.display(), e);
            }
            summary.failed.push((options.path.clone(), e));
        }
    }

    summary
}

async fn import_from_directory(options: &ImportOptions) -> ImportSummary {
    if options.name.is_some() {
        if !options.quiet && !options.json {
            warn!("--name is ignored when importing a directory");
        }
    }

    if options.dry_run {
        return dry_run_directory(
            &options.path,
            options.recursive,
            options.quiet,
            options.config_type,
        )
        .await;
    }

    let summary = import_directory(
        &options.path,
        options.recursive,
        options.force,
        options.config_type,
    )
    .await;

    if !options.quiet && !options.json {
        for result in &summary.imported {
            println!("✓ Imported: {} ({})", result.name, result.config_type);
        }
        for (path, reason) in &summary.skipped {
            println!("⊘ Skipped: {} - {}", path.display(), reason);
        }
        for (path, error) in &summary.failed {
            println!("✗ Failed: {} - {}", path.display(), error);
        }
    }

    summary
}

async fn dry_run_directory(
    dir: &Path,
    recursive: bool,
    quiet: bool,
    forced_type: Option<VpnConfigType>,
) -> ImportSummary {
    use walkdir::WalkDir;

    let mut summary = ImportSummary::default();

    let walker = if recursive {
        WalkDir::new(dir)
    } else {
        WalkDir::new(dir).max_depth(1)
    };

    if !quiet {
        println!("Would import:");
    }

    for entry in walker
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();

        if let Some(config_type) = forced_type.or_else(|| detect_config_type(path)) {
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("vpn");

            if !quiet {
                println!("  {} ({}) as '{}'", path.display(), config_type, name);
            }

            summary.imported.push(crate::import::ImportResult {
                name: name.to_string(),
                config_type,
                path: path.to_path_buf(),
            });
        }
    }

    if !quiet && summary.imported.is_empty() {
        println!("  (no VPN configs found)");
    }

    summary
}

fn print_summary(summary: &ImportSummary) {
    println!();
    println!(
        "Summary: {} imported, {} skipped, {} failed",
        summary.imported.len(),
        summary.skipped.len(),
        summary.failed.len()
    );
}

fn print_json_summary(summary: &ImportSummary) {
    use crate::import::types::*;

    let json = ImportResultJson {
        success: summary.failed.is_empty() && !summary.imported.is_empty(),
        imported: summary
            .imported
            .iter()
            .map(|r| ImportedConnection {
                name: r.name.clone(),
                config_type: r.config_type.to_string(),
                path: r.path.display().to_string(),
            })
            .collect(),
        skipped: summary
            .skipped
            .iter()
            .map(|(p, r)| SkippedFile {
                path: p.display().to_string(),
                reason: r.clone(),
            })
            .collect(),
        failed: summary
            .failed
            .iter()
            .map(|(p, e)| FailedFile {
                path: p.display().to_string(),
                error: e.to_string(),
            })
            .collect(),
        summary: ImportSummaryJson {
            total: summary.total_processed(),
            imported: summary.imported.len(),
            skipped: summary.skipped.len(),
            failed: summary.failed.len(),
        },
    };

    println!("{}", serde_json::to_string_pretty(&json).unwrap());
}

async fn connect_to_vpn(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let response = send_command(IpcCommand::Connect {
        name: name.to_string(),
    })
    .await?;

    if response.is_ok() {
        Ok(())
    } else {
        Err("Failed to connect".into())
    }
}
