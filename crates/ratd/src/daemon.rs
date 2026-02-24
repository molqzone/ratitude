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
use crate::source_scan::{render_candidates, SourceCandidate};
use crate::source_state::{build_source_domain, SourceDomainState};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeDomainState {
    schema: RuntimeSchemaState,
}

impl RuntimeDomainState {
    fn new(schema: RuntimeSchemaState) -> Self {
        Self { schema }
    }

    pub(crate) fn schema(&self) -> &RuntimeSchemaState {
        &self.schema
    }

    pub(crate) fn schema_mut(&mut self) -> &mut RuntimeSchemaState {
        &mut self.schema
    }

    pub(crate) fn clear_schema(&mut self) {
        self.schema.clear();
    }
}

#[derive(Debug, Clone)]
pub struct DaemonState {
    config_path: String,
    config: RatitudeConfig,
    source: SourceDomainState,
    runtime: RuntimeDomainState,
}

impl DaemonState {
    pub(crate) fn new(
        config_path: String,
        config: RatitudeConfig,
        source: SourceDomainState,
    ) -> Self {
        Self {
            config_path,
            config,
            source,
            runtime: RuntimeDomainState::new(RuntimeSchemaState::default()),
        }
    }

    pub(crate) fn config_path(&self) -> &str {
        &self.config_path
    }

    pub(crate) fn config(&self) -> &RatitudeConfig {
        &self.config
    }

    pub(crate) fn replace_config(&mut self, config: RatitudeConfig) {
        self.config = config;
    }

    pub(crate) fn source(&self) -> &SourceDomainState {
        &self.source
    }

    pub(crate) fn source_mut(&mut self) -> &mut SourceDomainState {
        &mut self.source
    }

    pub(crate) fn runtime(&self) -> &RuntimeDomainState {
        &self.runtime
    }

    pub(crate) fn runtime_mut(&mut self) -> &mut RuntimeDomainState {
        &mut self.runtime
    }
}

pub async fn run_daemon(cli: Cli) -> Result<()> {
    let cfg = load_config(&cli.config).await?;
    let source = build_source_domain(&cfg.ratd.source).await?;
    let mut state = DaemonState::new(cli.config.clone(), cfg, source);
    render_candidates(state.source().candidates());
    let mut output_manager = OutputManager::from_config(state.config())?;
    let mut runtime = activate_runtime(None, &mut state, &mut output_manager).await?;
    let mut output_failure_rx = output_manager.subscribe_failures();

    println!(
        "ratd daemon started at source {}",
        state.source().active_addr()
    );
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
                            state.runtime().schema().packet_count(),
                            state
                                .runtime()
                                .schema()
                                .schema_hash()
                                .unwrap_or(schema_hash)
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
            sink_failure = output_failure_rx.recv() => {
                match sink_failure {
                    Ok(reason) => {
                        return Err(anyhow!("output sink failure: {reason}"));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(anyhow!("output sink failure channel closed"));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        return Err(anyhow!(
                            "output sink failure channel lagged (skipped {skipped} messages)"
                        ));
                    }
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
    use crate::source_state::{build_source_domain, select_active_source, SourceDomainState};

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
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr").to_string();

        let mut cfg = RatitudeConfig::default();
        cfg.project.scan_root = ".".to_string();
        cfg.ratd.source.auto_scan = false;
        cfg.ratd.source.last_selected_addr = addr.clone();
        ConfigStore::new(&config_path)
            .save(&cfg)
            .expect("save config");

        let source = build_source_domain(&cfg.ratd.source)
            .await
            .expect("build source");
        let state = DaemonState::new(config_path.to_string_lossy().to_string(), cfg, source);
        assert_eq!(state.source().active_addr(), addr);
        assert_eq!(state.config().ratd.source.last_selected_addr, addr);

        let raw = fs::read_to_string(&config_path).expect("read config");
        assert!(raw.contains("last_selected_addr ="));

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

        let mut state = DaemonState::new(
            String::new(),
            cfg.clone(),
            SourceDomainState::new(
                vec![
                    SourceCandidate {
                        addr: "127.0.0.1:19021".to_string(),
                        reachable: false,
                    },
                    SourceCandidate {
                        addr: "127.0.0.1:2331".to_string(),
                        reachable: false,
                    },
                ],
                addr.clone(),
            ),
        );
        let mut output_manager = OutputManager::from_config(&cfg).expect("build output manager");
        let action =
            handle_console_command(ConsoleCommand::SourceList, &mut state, &mut output_manager)
                .await
                .expect("source list");
        assert!(!action.should_quit);
        assert!(!action.restart_runtime);
        assert_eq!(state.source().candidates().len(), 1);
        assert_eq!(state.source().candidates()[0].addr, addr);
        assert!(state.source().candidates()[0].reachable);
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
        let mut state = DaemonState::new(
            String::new(),
            cfg.clone(),
            SourceDomainState::new(
                vec![
                    SourceCandidate {
                        addr: addr.clone(),
                        reachable: true,
                    },
                    SourceCandidate {
                        addr: "127.0.0.1:65535".to_string(),
                        reachable: true,
                    },
                ],
                original_active.clone(),
            ),
        );
        let mut output_manager = OutputManager::from_config(&cfg).expect("build output manager");
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
        assert_eq!(state.source().active_addr(), original_active);
        assert_eq!(state.source().candidates().len(), 1);
        assert_eq!(state.source().candidates()[0].addr, addr);
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

        let mut state = DaemonState::new(
            config_path_str.clone(),
            cfg.clone(),
            SourceDomainState::new(Vec::new(), "127.0.0.1:19021".to_string()),
        );
        let mut output_manager = OutputManager::from_config(&cfg).expect("build output manager");
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
    async fn load_config_fails_when_missing() {
        let dir = unique_temp_dir("ratd_load_config_missing");
        let config_path = dir.join("rat.toml");
        assert!(!config_path.exists());

        let err = load_config(&config_path.to_string_lossy())
            .await
            .expect_err("load should fail without rat.toml");
        assert!(err.to_string().contains("config file does not exist"));
        assert!(!config_path.exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ratd_manifest_keeps_protocol_dependency_indirect() {
        let manifest = include_str!("../Cargo.toml");
        assert!(
            !manifest.contains("rat-protocol"),
            "ratd must consume protocol via rat-core runtime only"
        );
        assert!(
            manifest.contains("rat-core"),
            "ratd must depend on rat-core runtime"
        );
    }
}
