use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use rat_config::{ConfigStore, DEFAULT_CONFIG_PATH};
use rat_sync::sync_packets_fs;

#[derive(Parser, Debug, Clone)]
#[command(name = "ratsync", about = "Ratitude schema/header sync tool")]
struct Cli {
    #[arg(
        long,
        default_value = DEFAULT_CONFIG_PATH,
        help = "Path to rat.toml config file"
    )]
    config: String,

    #[arg(long, help = "Optional source scan root override")]
    scan_root: Option<String>,
}

#[derive(Debug)]
struct SyncSummary {
    header_path: PathBuf,
    packet_count: usize,
    schema_hash: String,
    warnings: Vec<String>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let summary = execute(&cli)?;
    render_summary(&summary);
    Ok(())
}

fn execute(cli: &Cli) -> Result<SyncSummary> {
    let scan_root = cli.scan_root.as_deref().map(Path::new);
    let result = sync_packets_fs(&cli.config, scan_root)
        .with_context(|| format!("failed to sync packets using config {}", cli.config))?;

    let header_path = ConfigStore::new(&cli.config)
        .paths_for(&result.config)
        .generated_header_path()
        .to_path_buf();

    Ok(SyncSummary {
        header_path,
        packet_count: result.packet_defs.len(),
        schema_hash: result.generated.meta.schema_hash,
        warnings: result.layout_warnings,
    })
}

fn render_summary(summary: &SyncSummary) {
    println!("ratsync completed");
    println!("  header: {}", summary.header_path.display());
    println!("  packets: {}", summary.packet_count);
    println!("  schema_hash: {}", summary.schema_hash);
    if summary.warnings.is_empty() {
        println!("  warnings: 0");
    } else {
        println!("  warnings: {}", summary.warnings.len());
        for warning in &summary.warnings {
            println!("    - {}", warning);
        }
    }
}

#[cfg(test)]
fn parse_cli_from<I, T>(args: I) -> Result<Cli>
where
    I: IntoIterator<Item = T>,
    T: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<String>>();
    Cli::try_parse_from(args).map_err(|err| anyhow::anyhow!(err.to_string()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}_{unique}"));
        fs::create_dir_all(&dir).expect("mkdir temp dir");
        dir
    }

    #[test]
    fn cli_uses_default_config_path() {
        let cli = parse_cli_from(["ratsync"]).expect("parse");
        assert_eq!(cli.config, "rat.toml");
        assert!(cli.scan_root.is_none());
    }

    #[test]
    fn cli_accepts_scan_root_override() {
        let cli = parse_cli_from([
            "ratsync",
            "--config",
            "examples/mock/rat.toml",
            "--scan-root",
            "Core",
        ])
        .expect("parse");
        assert_eq!(cli.config, "examples/mock/rat.toml");
        assert_eq!(cli.scan_root.as_deref(), Some("Core"));
    }

    #[test]
    fn execute_uses_scan_root_override() {
        let dir = unique_temp_dir("ratsync_scan_root_override");
        fs::create_dir_all(dir.join("ignored")).expect("mkdir ignored");
        fs::create_dir_all(dir.join("actual")).expect("mkdir actual");
        let config_path = dir.join("rat.toml");

        let config = r#"
[project]
name = "sync_test"
scan_root = "ignored"
recursive = true
extensions = [".h", ".c"]

[generation]
out_dir = "."
header_name = "rat_gen.h"
"#;
        fs::write(&config_path, config).expect("write config");

        let source = r#"
// @rat, plot
typedef struct {
  int32_t value;
  uint32_t tick;
} RatSample;
"#;
        fs::write(dir.join("actual").join("sample.c"), source).expect("write source");

        let cli = Cli {
            config: config_path.to_string_lossy().into_owned(),
            scan_root: Some("actual".to_string()),
        };
        let summary = execute(&cli).expect("sync should pass");
        assert_eq!(summary.packet_count, 1);
        assert!(summary.header_path.exists());
        let header_raw = fs::read_to_string(&summary.header_path).expect("read header");
        assert!(header_raw.contains(&format!(
            "#define RAT_GEN_SCHEMA_HASH {}ULL",
            summary.schema_hash
        )));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn execute_rejects_legacy_generation_toml_name() {
        let dir = unique_temp_dir("ratsync_legacy_generation_toml_name");
        let config_path = dir.join("rat.toml");
        let config = r#"
[project]
name = "demo"
scan_root = "."

[generation]
out_dir = "."
toml_name = "rat_gen.toml"
header_name = "rat_gen.h"
"#;
        fs::write(&config_path, config).expect("write config");

        let cli = Cli {
            config: config_path.to_string_lossy().into_owned(),
            scan_root: None,
        };

        let err = execute(&cli).expect_err("legacy key should fail");
        let msg = format!("{err:#}");
        assert!(msg.contains("generation.toml_name"));
        assert!(msg.contains("rat_gen.toml is no longer generated"));

        let _ = fs::remove_dir_all(&dir);
    }
}
