use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rat_config::{load_generated_or_default, load_or_default, PacketDef, RatitudeConfig};
use rat_core::{
    start_ingest_runtime, IngestRuntime, IngestRuntimeConfig, ListenerOptions, RuntimeFieldDef,
    RuntimePacketDef, RuntimeSignal,
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::cli::Cli;
use crate::console::{print_help, spawn_console_reader, ConsoleCommand};
use crate::output_manager::OutputManager;
use crate::source_scan::{discover_sources, render_candidates, SourceCandidate};
use crate::sync_controller::{SyncController, SyncOutcome};

const UNKNOWN_PACKET_WINDOW: Duration = Duration::from_secs(5);
const UNKNOWN_PACKET_THRESHOLD: u32 = 20;

#[derive(Debug, Clone)]
pub struct DaemonState {
    pub config_path: String,
    pub config: RatitudeConfig,
    pub source_candidates: Vec<SourceCandidate>,
    pub active_source: String,
    pub packets: Vec<PacketDef>,
}

pub async fn run_daemon(cli: Cli) -> Result<()> {
    let mut cfg = load_config(&cli.config)?;
    let mut sync_controller =
        SyncController::new(cli.config.clone(), cfg.rttd.behavior.sync_debounce_ms);

    if cfg.rttd.behavior.auto_sync_on_start {
        let outcome = sync_controller.trigger("startup").await?;
        print_sync_outcome(&outcome);
        cfg = load_config(&cli.config)?;
    }

    let mut state = build_state(cli.config.clone(), cfg).await?;
    let mut output_manager = OutputManager::from_config(&state.config);
    let mut runtime = activate_runtime(None, &mut state, &mut output_manager).await?;

    println!("rttd daemon started at source {}", state.active_source);
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
                let action = handle_console_command(command, &mut state, &mut output_manager, &mut sync_controller).await?;
                if action.should_quit {
                    break;
                }
                if action.restart_runtime {
                    runtime = activate_runtime(Some(runtime), &mut state, &mut output_manager).await?;
                }
            }
            maybe_signal = runtime.recv_signal() => {
                match maybe_signal {
                    Some(RuntimeSignal::InitMagicVerified) => {
                        if state.config.rttd.behavior.auto_sync_on_reset {
                            let outcome = sync_controller.trigger("reset").await?;
                            print_sync_outcome(&outcome);
                            if outcome.changed {
                                runtime = activate_runtime(Some(runtime), &mut state, &mut output_manager).await?;
                            }
                        }
                    }
                    Some(RuntimeSignal::Fatal(err)) => {
                        return Err(anyhow!(err.to_string()));
                    }
                    None => {
                        return Err(anyhow!("ingest runtime signal channel closed"));
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

#[derive(Default)]
struct CommandAction {
    should_quit: bool,
    restart_runtime: bool,
}

async fn handle_console_command(
    command: ConsoleCommand,
    state: &mut DaemonState,
    output_manager: &mut OutputManager,
    sync_controller: &mut SyncController,
) -> Result<CommandAction> {
    let mut action = CommandAction::default();

    match command {
        ConsoleCommand::Help => {
            print_help();
        }
        ConsoleCommand::Status => {
            let output = output_manager.snapshot();
            println!("status:");
            println!("  source: {}", state.active_source);
            println!("  packets: {}", state.packets.len());
            println!(
                "  jsonl: {}",
                if output.jsonl_enabled { "on" } else { "off" }
            );
            println!(
                "  foxglove: {} ({})",
                if output.foxglove_enabled { "on" } else { "off" },
                output.foxglove_ws_addr
            );
        }
        ConsoleCommand::SourceList => {
            refresh_source_candidates(state, true).await;
        }
        ConsoleCommand::SourceUse(index) => {
            refresh_source_candidates(state, false).await;
            let Some(candidate) = state.source_candidates.get(index) else {
                println!("invalid source index: {}", index);
                render_candidates(&state.source_candidates);
                return Ok(action);
            };
            state.active_source = candidate.addr.clone();
            state.config.rttd.source.last_selected_addr = candidate.addr.clone();
            state.config.save(&state.config_path)?;
            println!("selected source: {}", state.active_source);
            action.restart_runtime = true;
        }
        ConsoleCommand::Sync => {
            let outcome = sync_controller.trigger("manual").await?;
            print_sync_outcome(&outcome);
            if outcome.changed {
                action.restart_runtime = true;
            }
        }
        ConsoleCommand::Foxglove(enabled) => {
            output_manager.set_foxglove(enabled, None);
            state.config.rttd.outputs.foxglove.enabled = enabled;
            state.config.save(&state.config_path)?;
            println!("foxglove output: {}", if enabled { "on" } else { "off" });
            action.restart_runtime = true;
        }
        ConsoleCommand::Jsonl { enabled, path } => {
            output_manager.set_jsonl(enabled, path.clone());
            state.config.rttd.outputs.jsonl.enabled = enabled;
            if let Some(path) = path {
                state.config.rttd.outputs.jsonl.path = path;
            }
            state.config.save(&state.config_path)?;
            println!("jsonl output: {}", if enabled { "on" } else { "off" });
            action.restart_runtime = true;
        }
        ConsoleCommand::PacketLookup {
            struct_name,
            field_name,
        } => {
            let packet = state
                .packets
                .iter()
                .find(|packet| packet.struct_name.eq_ignore_ascii_case(&struct_name));
            if let Some(packet) = packet {
                let field = packet
                    .fields
                    .iter()
                    .find(|field| field.name.eq_ignore_ascii_case(&field_name));
                if let Some(field) = field {
                    println!(
                        "packet {} field {} => type={}, offset={}, size={}",
                        packet.struct_name, field.name, field.c_type, field.offset, field.size
                    );
                } else {
                    println!("field not found: {}", field_name);
                }
            } else {
                println!("packet not found: {}", struct_name);
            }
        }
        ConsoleCommand::Quit => {
            action.should_quit = true;
        }
        ConsoleCommand::Unknown(raw) => {
            println!("unknown command: {}", raw);
        }
    }

    Ok(action)
}

async fn refresh_source_candidates(state: &mut DaemonState, render: bool) {
    state.source_candidates = discover_sources(&state.config.rttd.source).await;
    if render {
        render_candidates(&state.source_candidates);
    }
}

async fn activate_runtime(
    old_runtime: Option<IngestRuntime>,
    state: &mut DaemonState,
    output_manager: &mut OutputManager,
) -> Result<IngestRuntime> {
    if let Some(old_runtime) = old_runtime {
        output_manager.shutdown().await;
        old_runtime.shutdown(false).await;
    }

    state.config = load_config(&state.config_path)?;
    let runtime = start_runtime(&mut state.config, &state.active_source).await?;
    state.packets = state.config.packets.clone();
    output_manager
        .apply(runtime.hub(), state.packets.clone())
        .await?;
    info!(
        source = %state.active_source,
        packets = state.packets.len(),
        "ingest runtime started"
    );
    Ok(runtime)
}

fn load_config(config_path: &str) -> Result<RatitudeConfig> {
    let (cfg, _) = load_or_default(config_path)?;
    Ok(cfg)
}

async fn build_state(config_path: String, cfg: RatitudeConfig) -> Result<DaemonState> {
    let source_candidates = discover_sources(&cfg.rttd.source).await;
    render_candidates(&source_candidates);

    let active_source =
        select_active_source(&source_candidates, &cfg.rttd.source.last_selected_addr)?;

    Ok(DaemonState {
        config_path,
        config: cfg,
        source_candidates,
        active_source,
        packets: Vec::new(),
    })
}

fn select_active_source(
    candidates: &[SourceCandidate],
    last_selected_addr: &str,
) -> Result<String> {
    if let Some(candidate) = candidates
        .iter()
        .find(|candidate| candidate.reachable && candidate.addr == last_selected_addr)
    {
        return Ok(candidate.addr.clone());
    }
    if let Some(candidate) = candidates.iter().find(|candidate| candidate.reachable) {
        return Ok(candidate.addr.clone());
    }
    Err(anyhow!(
        "no reachable RTT source detected; start RTT endpoint first"
    ))
}

fn print_sync_outcome(outcome: &SyncOutcome) {
    if outcome.skipped {
        println!("sync skipped: {}", outcome.reason);
        return;
    }

    for warning in &outcome.warnings {
        warn!(warning = %warning, "sync warning");
        println!("sync warning: {}", warning);
    }

    println!(
        "sync done: changed={}, packets={}, reason={}",
        outcome.changed, outcome.packets, outcome.reason
    );
}

async fn start_runtime(cfg: &mut RatitudeConfig, addr: &str) -> Result<IngestRuntime> {
    let expected_fingerprint = load_generated_packets(cfg)?;
    let text_id = parse_text_id(cfg.rttd.text_id)?;
    let reconnect = parse_duration(&cfg.rttd.behavior.reconnect)?;
    let buf = cfg.rttd.behavior.buf;
    let reader_buf = cfg.rttd.behavior.reader_buf;
    let packets = map_runtime_packets(&cfg.packets);

    start_ingest_runtime(IngestRuntimeConfig {
        addr: addr.to_string(),
        listener: ListenerOptions {
            reconnect,
            reconnect_max: Duration::from_secs(30),
            dial_timeout: Duration::from_secs(5),
            reader_buf_bytes: reader_buf,
        },
        hub_buffer: buf,
        text_packet_id: text_id,
        expected_fingerprint,
        packets,
        unknown_window: UNKNOWN_PACKET_WINDOW,
        unknown_threshold: UNKNOWN_PACKET_THRESHOLD,
    })
    .await
    .map_err(|err| anyhow!(err.to_string()))
}

fn map_runtime_packets(packets: &[PacketDef]) -> Vec<RuntimePacketDef> {
    packets
        .iter()
        .map(|packet| RuntimePacketDef {
            id: packet.id,
            struct_name: packet.struct_name.clone(),
            packed: packet.packed,
            byte_size: packet.byte_size,
            fields: packet
                .fields
                .iter()
                .map(|field| RuntimeFieldDef {
                    name: field.name.clone(),
                    c_type: field.c_type.clone(),
                    offset: field.offset,
                    size: field.size,
                })
                .collect(),
        })
        .collect()
}

fn load_generated_packets(cfg: &mut RatitudeConfig) -> Result<u64> {
    let generated_path = cfg.generated_toml_path().to_path_buf();
    let (generated, exists) = load_generated_or_default(&generated_path)?;
    if !exists {
        return Err(anyhow!(
            "rat_gen.toml not found at {}; run sync before starting daemon",
            generated_path.display()
        ));
    }
    if generated.packets.is_empty() {
        return Err(anyhow!("rat_gen.toml has no packets"));
    }
    let expected_fingerprint = parse_generated_fingerprint(&generated.meta.fingerprint)
        .with_context(|| format!("invalid fingerprint in {}", generated_path.display()))?;

    cfg.packets = generated.to_packet_defs();
    cfg.validate()?;
    Ok(expected_fingerprint)
}

fn parse_text_id(value: u16) -> Result<u8> {
    if value > 0xFF {
        return Err(anyhow!("text id out of range: 0x{:X}", value));
    }
    Ok(value as u8)
}

fn parse_duration(raw: &str) -> Result<Duration> {
    humantime::parse_duration(raw).with_context(|| format!("invalid duration: {}", raw))
}

fn parse_generated_fingerprint(raw: &str) -> Result<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("generated fingerprint is empty"));
    }
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    u64::from_str_radix(hex, 16)
        .with_context(|| format!("invalid generated fingerprint value: {}", raw))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use tokio::net::TcpListener;

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
    fn parse_generated_fingerprint_rejects_empty() {
        assert!(parse_generated_fingerprint(" ").is_err());
    }

    #[test]
    fn parse_generated_fingerprint_supports_prefixed_hex() {
        let parsed = parse_generated_fingerprint("0xAA").expect("parse");
        assert_eq!(parsed, 0xAA);
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
        let dir = unique_temp_dir("rttd_build_state");
        let config_path = dir.join("rat.toml");

        let mut cfg = RatitudeConfig::default();
        cfg.project.scan_root = ".".to_string();
        cfg.rttd.source.auto_scan = false;
        cfg.rttd.source.last_selected_addr = "10.10.10.10:19021".to_string();
        cfg.save(&config_path).expect("save config");

        let state = build_state(config_path.to_string_lossy().to_string(), cfg)
            .await
            .expect("build state");
        assert_eq!(state.active_source, "10.10.10.10:19021");
        assert_eq!(
            state.config.rttd.source.last_selected_addr,
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
        cfg.rttd.source.auto_scan = false;
        cfg.rttd.source.scan_timeout_ms = 100;
        cfg.rttd.source.last_selected_addr = addr.clone();

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
            packets: Vec::new(),
        };
        let mut output_manager = OutputManager::from_config(&cfg);
        let mut sync_controller = SyncController::new("/tmp/non-existent-rat.toml".to_string(), 1);

        let action = handle_console_command(
            ConsoleCommand::SourceList,
            &mut state,
            &mut output_manager,
            &mut sync_controller,
        )
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
        cfg.rttd.source.auto_scan = false;
        cfg.rttd.source.scan_timeout_ms = 100;
        cfg.rttd.source.last_selected_addr = addr.clone();

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
            packets: Vec::new(),
        };
        let mut output_manager = OutputManager::from_config(&cfg);
        let mut sync_controller = SyncController::new("/tmp/non-existent-rat.toml".to_string(), 1);

        let action = handle_console_command(
            ConsoleCommand::SourceUse(1),
            &mut state,
            &mut output_manager,
            &mut sync_controller,
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

    #[test]
    fn load_config_does_not_rewrite_existing_file() {
        let dir = unique_temp_dir("rttd_load_config_read_only");
        let config_path = dir.join("rat.toml");

        let raw = r#"# keep comment and formatting
[project]
name = "demo"
scan_root = "."
recursive = true
extensions = [".c", ".h"]

[generation]
out_dir = "."
toml_name = "rat_gen.toml"
header_name = "rat_gen.h"

[rttd]
text_id = 255
"#;
        fs::write(&config_path, raw).expect("write config");

        let _cfg = load_config(&config_path.to_string_lossy()).expect("load");
        let after = fs::read_to_string(&config_path).expect("read config");
        assert_eq!(after, raw);

        let _ = fs::remove_file(&config_path);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_config_does_not_create_file_when_missing() {
        let dir = unique_temp_dir("rttd_load_config_missing");
        let config_path = dir.join("rat.toml");
        assert!(!config_path.exists());

        let _cfg = load_config(&config_path.to_string_lossy()).expect("load default");
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
