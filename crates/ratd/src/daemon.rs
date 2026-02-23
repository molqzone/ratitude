use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rat_config::RatitudeConfig;
use rat_core::RuntimeSignal;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::cli::Cli;
use crate::command_loop::handle_console_command;
use crate::config_io::load_config;
use crate::console::spawn_console_reader;
use crate::output_manager::OutputManager;
use crate::runtime_lifecycle::{activate_runtime, apply_schema_ready};
use crate::runtime_schema::RuntimeSchemaState;
use crate::source_scan::SourceCandidate;
use crate::source_state::build_state;

#[derive(Debug, Clone)]
pub struct DaemonState {
    pub config_path: String,
    pub config: RatitudeConfig,
    pub source_candidates: Vec<SourceCandidate>,
    pub active_source: String,
    pub runtime_schema: RuntimeSchemaState,
}

pub async fn run_daemon(cli: Cli) -> Result<()> {
    let cfg = load_config(&cli.config).await?;
    let mut state = build_state(cli.config.clone(), cfg).await?;
    let mut output_manager = OutputManager::from_config(&state.config);
    let mut runtime = activate_runtime(None, &mut state, &mut output_manager).await?;
    let mut output_health_tick = tokio::time::interval(Duration::from_millis(200));
    output_health_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    println!("ratd daemon started at source {}", state.active_source);
    println!("type `$help` to show available commands");

    let console_shutdown = CancellationToken::new();
    let mut command_rx = spawn_console_reader(console_shutdown.clone());

    loop {
        tokio::select! {
            ctrl = tokio::signal::ctrl_c() => {
                ctrl.context("failed to wait ctrl-c")?;
                info!("received ctrl-c, stopping daemon");
                break;
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
                    break;
                };
                let action = handle_console_command(command, &mut state, &mut output_manager).await?;
                if action.should_quit {
                    break;
                }
                if action.restart_runtime {
                    runtime = activate_runtime(Some(runtime), &mut state, &mut output_manager).await?;
                }
            }
            maybe_signal = runtime.recv_signal() => {
                match maybe_signal {
                    Some(RuntimeSignal::SchemaReady { schema_hash, packets }) => {
                        apply_schema_ready(
                            &mut state,
                            &mut output_manager,
                            &runtime,
                            schema_hash,
                            packets,
                        )
                        .await?;
                        println!(
                            "runtime schema ready: packets={}, hash=0x{:016X}",
                            state.runtime_schema.packet_count(),
                            state.runtime_schema.schema_hash().unwrap_or(schema_hash)
                        );
                    }
                    Some(RuntimeSignal::Fatal(err)) => {
                        return Err(anyhow!(err.to_string()));
                    }
                    None => {
                        return Err(anyhow!("ingest runtime signal channel closed"));
                    }
                }
            }
            _ = output_health_tick.tick() => {
                if let Some(err) = output_manager.poll_failure() {
                    return Err(anyhow!("output sink failure: {err}"));
                }
            }
        }
    }

    console_shutdown.cancel();
    output_manager.shutdown().await;
    runtime.shutdown(true).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rat_config::ConfigStore;
    use tokio::net::TcpListener;

    use super::*;
    use crate::command_loop::handle_console_command;
    use crate::config_io::load_config;
    use crate::console::ConsoleCommand;
    use crate::source_state::{build_state, select_active_source};

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
    fn select_active_source_prefers_reachable_last_selected() {
        let candidates = vec![
            SourceCandidate {
                addr: "127.0.0.1:19021".to_string(),
                reachable: true,
            },
            SourceCandidate {
                addr: "127.0.0.1:2331".to_string(),
                reachable: true,
            },
        ];
        let selected = select_active_source(&candidates, "127.0.0.1:2331").expect("select");
        assert_eq!(selected, "127.0.0.1:2331");
    }

    #[test]
    fn select_active_source_fast_fails_without_reachable_candidate() {
        let candidates = vec![
            SourceCandidate {
                addr: "127.0.0.1:19021".to_string(),
                reachable: false,
            },
            SourceCandidate {
                addr: "127.0.0.1:2331".to_string(),
                reachable: false,
            },
        ];
        let err = select_active_source(&candidates, "127.0.0.1:19021").expect_err("must fail");
        assert!(err
            .to_string()
            .contains("no reachable RTT source detected; start RTT endpoint first"));
    }

    #[tokio::test]
    async fn build_state_does_not_overwrite_last_selected_addr_on_startup() {
        let dir = unique_temp_dir("ratd_build_state");
        let config_path = dir.join("rat.toml");

        let mut cfg = RatitudeConfig::default();
        cfg.project.scan_root = ".".to_string();
        cfg.ratd.source.auto_scan = false;
        cfg.ratd.source.last_selected_addr = "10.10.10.10:19021".to_string();
        ConfigStore::new(&config_path)
            .save(&cfg)
            .expect("save config");

        let state = build_state(config_path.to_string_lossy().to_string(), cfg)
            .await
            .expect("build state");
        assert_eq!(state.active_source, "10.10.10.10:19021");
        assert_eq!(
            state.config.ratd.source.last_selected_addr,
            "10.10.10.10:19021"
        );

        let raw = fs::read_to_string(&config_path).expect("read config");
        assert!(raw.contains("last_selected_addr = \"10.10.10.10:19021\""));

        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn source_list_refreshes_candidates_before_render() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr").to_string();

        let mut cfg = RatitudeConfig::default();
        cfg.ratd.source.auto_scan = false;
        cfg.ratd.source.scan_timeout_ms = 100;
        cfg.ratd.source.last_selected_addr = addr.clone();

        let mut state = DaemonState {
            config_path: String::new(),
            config: cfg.clone(),
            source_candidates: vec![
                SourceCandidate {
                    addr: "127.0.0.1:19021".to_string(),
                    reachable: false,
                },
                SourceCandidate {
                    addr: "127.0.0.1:2331".to_string(),
                    reachable: false,
                },
            ],
            active_source: addr.clone(),
            runtime_schema: RuntimeSchemaState::default(),
        };
        let mut output_manager = OutputManager::from_config(&cfg);
        let action =
            handle_console_command(ConsoleCommand::SourceList, &mut state, &mut output_manager)
                .await
                .expect("source list");
        assert!(!action.should_quit);
        assert!(!action.restart_runtime);
        assert_eq!(state.source_candidates.len(), 1);
        assert_eq!(state.source_candidates[0].addr, addr);
        assert!(state.source_candidates[0].reachable);
    }

    #[tokio::test]
    async fn source_use_revalidates_index_against_refreshed_candidates() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr").to_string();

        let mut cfg = RatitudeConfig::default();
        cfg.ratd.source.auto_scan = false;
        cfg.ratd.source.scan_timeout_ms = 100;
        cfg.ratd.source.last_selected_addr = addr.clone();

        let original_active = addr.clone();
        let mut state = DaemonState {
            config_path: String::new(),
            config: cfg.clone(),
            source_candidates: vec![
                SourceCandidate {
                    addr: addr.clone(),
                    reachable: true,
                },
                SourceCandidate {
                    addr: "127.0.0.1:65535".to_string(),
                    reachable: true,
                },
            ],
            active_source: original_active.clone(),
            runtime_schema: RuntimeSchemaState::default(),
        };
        let mut output_manager = OutputManager::from_config(&cfg);
        let action = handle_console_command(
            ConsoleCommand::SourceUse(1),
            &mut state,
            &mut output_manager,
        )
        .await
        .expect("source use");

        assert!(!action.should_quit);
        assert!(
            !action.restart_runtime,
            "index 1 should be invalid after refresh"
        );
        assert_eq!(state.active_source, original_active);
        assert_eq!(state.source_candidates.len(), 1);
        assert_eq!(state.source_candidates[0].addr, addr);
    }

    #[tokio::test]
    async fn output_commands_apply_without_runtime_restart() {
        let dir = unique_temp_dir("ratd_output_command_apply");
        let config_path = dir.join("rat.toml");
        let config_path_str = config_path.to_string_lossy().to_string();

        let cfg = RatitudeConfig::default();
        ConfigStore::new(&config_path)
            .save(&cfg)
            .expect("save config");

        let mut state = DaemonState {
            config_path: config_path_str.clone(),
            config: cfg.clone(),
            source_candidates: Vec::new(),
            active_source: "127.0.0.1:19021".to_string(),
            runtime_schema: RuntimeSchemaState::default(),
        };
        let mut output_manager = OutputManager::from_config(&cfg);
        let foxglove_action = handle_console_command(
            ConsoleCommand::Foxglove(true),
            &mut state,
            &mut output_manager,
        )
        .await
        .expect("foxglove command");
        assert!(!foxglove_action.restart_runtime);

        let jsonl_action = handle_console_command(
            ConsoleCommand::Jsonl {
                enabled: true,
                path: Some(String::new()),
            },
            &mut state,
            &mut output_manager,
        )
        .await
        .expect("jsonl command");
        assert!(!jsonl_action.restart_runtime);

        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn load_config_does_not_rewrite_existing_file() {
        let dir = unique_temp_dir("ratd_load_config_read_only");
        let config_path = dir.join("rat.toml");

        let raw = r#"# keep comment and formatting
[project]
name = "demo"
scan_root = "."
recursive = true
extensions = [".c", ".h"]

[generation]
out_dir = "."
header_name = "rat_gen.h"

[ratd]
text_id = 255
"#;
        fs::write(&config_path, raw).expect("write config");

        let _cfg = load_config(&config_path.to_string_lossy())
            .await
            .expect("load");
        let after = fs::read_to_string(&config_path).expect("read config");
        assert_eq!(after, raw);

        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn load_config_does_not_create_file_when_missing() {
        let dir = unique_temp_dir("ratd_load_config_missing");
        let config_path = dir.join("rat.toml");
        assert!(!config_path.exists());

        let _cfg = load_config(&config_path.to_string_lossy())
            .await
            .expect("load default");
        assert!(!config_path.exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn daemon_module_no_longer_contains_protocol_runtime_details() {
        let source = include_str!("daemon.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("split production source");
        assert!(!production.contains("cobs_decode"));
        assert!(!production.contains("ProtocolContext"));
        assert!(!production.contains("ProtocolError"));
    }
}
