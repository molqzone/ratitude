use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Context, Result};
use rat_config::{load_generated_or_default, load_or_default, FieldDef, PacketDef, RatitudeConfig};
use rat_core::{spawn_listener, Hub, ListenerOptions};
use rat_protocol::{
    cobs_decode, DynamicFieldDef, DynamicPacketDef, ProtocolContext, ProtocolError, RatPacket,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

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

#[derive(Debug, Clone, Copy)]
enum RuntimeEvent {
    InitMagicVerified,
}

struct IngestRuntime {
    shutdown: CancellationToken,
    hub: Hub,
    listener_task: JoinHandle<()>,
    consume_task: JoinHandle<Result<()>>,
    events_rx: mpsc::Receiver<RuntimeEvent>,
}

#[derive(Clone, Debug)]
struct UnknownPacketWindowReport {
    count: u32,
    unique_ids: usize,
}

#[derive(Clone, Debug)]
struct UnknownPacketObservation {
    total_count: u64,
    window_count: u32,
    threshold_crossed: bool,
    rolled_over: Option<UnknownPacketWindowReport>,
}

#[derive(Clone, Debug)]
struct UnknownPacketMonitor {
    window: Duration,
    threshold: u32,
    window_started_at: Instant,
    window_count: u32,
    total_count: u64,
    per_window_ids: BTreeMap<u8, u32>,
}

impl UnknownPacketMonitor {
    fn new(window: Duration, threshold: u32) -> Self {
        Self {
            window,
            threshold: threshold.max(1),
            window_started_at: Instant::now(),
            window_count: 0,
            total_count: 0,
            per_window_ids: BTreeMap::new(),
        }
    }

    fn record(&mut self, packet_id: u8) -> UnknownPacketObservation {
        self.record_at(packet_id, Instant::now())
    }

    fn record_at(&mut self, packet_id: u8, now: Instant) -> UnknownPacketObservation {
        let mut rolled_over = None;
        if now.duration_since(self.window_started_at) >= self.window {
            if self.window_count > 0 {
                rolled_over = Some(UnknownPacketWindowReport {
                    count: self.window_count,
                    unique_ids: self.per_window_ids.len(),
                });
            }
            self.window_started_at = now;
            self.window_count = 0;
            self.per_window_ids.clear();
        }

        self.window_count = self.window_count.saturating_add(1);
        self.total_count = self.total_count.saturating_add(1);
        *self.per_window_ids.entry(packet_id).or_insert(0) += 1;

        UnknownPacketObservation {
            total_count: self.total_count,
            window_count: self.window_count,
            threshold_crossed: self.window_count == self.threshold,
            rolled_over,
        }
    }
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
            maybe_event = runtime.events_rx.recv() => {
                if let Some(RuntimeEvent::InitMagicVerified) = maybe_event {
                    if state.config.rttd.behavior.auto_sync_on_reset {
                        let outcome = sync_controller.trigger("reset").await?;
                        print_sync_outcome(&outcome);
                        if outcome.changed {
                            runtime = activate_runtime(Some(runtime), &mut state, &mut output_manager).await?;
                        }
                    }
                }
            }
            result = &mut runtime.consume_task => {
                let err = frame_consumer_failure(result);
                return Err(err);
            }
        }
    }

    console_shutdown.cancel();
    output_manager.shutdown().await;
    shutdown_ingest_runtime(runtime, true).await;
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
        shutdown_ingest_runtime(old_runtime, false).await;
    }

    state.config = load_config(&state.config_path)?;
    let runtime = start_runtime(&mut state.config, &state.active_source).await?;
    state.packets = state.config.packets.clone();
    output_manager
        .apply(runtime.hub.clone(), state.packets.clone())
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

    let mut protocol = ProtocolContext::new();
    let text_id = parse_text_id(cfg.rttd.text_id)?;
    protocol.set_text_packet_id(text_id);
    register_dynamic_packets(&mut protocol, &cfg.packets)?;
    let protocol = Arc::new(protocol);

    let reconnect = parse_duration(&cfg.rttd.behavior.reconnect)?;
    let buf = cfg.rttd.behavior.buf;
    let reader_buf = cfg.rttd.behavior.reader_buf;

    start_ingest_runtime(
        addr.to_string(),
        reconnect,
        buf,
        reader_buf,
        protocol,
        expected_fingerprint,
    )
    .await
}

async fn start_ingest_runtime(
    addr: String,
    reconnect: Duration,
    buf: usize,
    reader_buf: usize,
    protocol: Arc<ProtocolContext>,
    expected_fingerprint: u64,
) -> Result<IngestRuntime> {
    let shutdown = CancellationToken::new();
    let hub = Hub::new(buf.max(1));
    let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>(buf.max(1));
    let (event_tx, event_rx) = mpsc::channel::<RuntimeEvent>(8);

    let listener_task = spawn_listener(
        shutdown.clone(),
        addr,
        frame_tx,
        ListenerOptions {
            reconnect,
            reconnect_max: Duration::from_secs(30),
            dial_timeout: Duration::from_secs(5),
            reader_buf_bytes: reader_buf,
        },
    );

    let consume_task = spawn_frame_consumer(
        frame_rx,
        hub.clone(),
        protocol,
        shutdown.clone(),
        expected_fingerprint,
        event_tx,
    );

    Ok(IngestRuntime {
        shutdown,
        hub,
        listener_task,
        consume_task,
        events_rx: event_rx,
    })
}

async fn shutdown_ingest_runtime(runtime: IngestRuntime, join_consumer: bool) {
    let IngestRuntime {
        shutdown,
        hub: _hub,
        listener_task,
        consume_task,
        events_rx: _events_rx,
    } = runtime;

    shutdown.cancel();
    listener_task.abort();
    let _ = listener_task.await;
    if join_consumer {
        let _ = consume_task.await;
    }
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

fn register_dynamic_packets(protocol: &mut ProtocolContext, packets: &[PacketDef]) -> Result<()> {
    protocol.clear_dynamic_registry();
    debug!(
        packets = packets.len(),
        "registering dynamic packet definitions"
    );
    for packet in packets {
        if packet.id > 0xFF {
            return Err(anyhow!("packet id out of range: 0x{:X}", packet.id));
        }

        let fields = packet
            .fields
            .iter()
            .map(map_field)
            .collect::<Vec<DynamicFieldDef>>();

        protocol
            .register_dynamic(
                packet.id as u8,
                DynamicPacketDef {
                    id: packet.id as u8,
                    struct_name: packet.struct_name.clone(),
                    packed: packet.packed,
                    byte_size: packet.byte_size,
                    fields,
                },
            )
            .with_context(|| {
                format!(
                    "register packet 0x{:02X} ({})",
                    packet.id, packet.struct_name
                )
            })?;
    }
    Ok(())
}

fn map_field(field: &FieldDef) -> DynamicFieldDef {
    DynamicFieldDef {
        name: field.name.clone(),
        c_type: field.c_type.clone(),
        offset: field.offset,
        size: field.size,
    }
}

fn decode_init_magic_packet(id: u8, payload: &[u8]) -> Option<u64> {
    if id != 0x00 || payload.len() != 12 {
        return None;
    }
    if payload.get(0..4) != Some(b"RATI") {
        return None;
    }

    let mut fingerprint = 0_u64;
    for (idx, byte) in payload[4..12].iter().enumerate() {
        fingerprint |= (*byte as u64) << (idx * 8);
    }
    Some(fingerprint)
}

fn frame_consumer_failure(
    result: std::result::Result<Result<()>, tokio::task::JoinError>,
) -> anyhow::Error {
    match result {
        Ok(Ok(())) => anyhow!("frame consumer stopped before shutdown"),
        Ok(Err(err)) => err,
        Err(err) => anyhow!("frame consumer task failed: {err}"),
    }
}

fn spawn_frame_consumer(
    mut receiver: mpsc::Receiver<Vec<u8>>,
    hub: Hub,
    protocol: Arc<ProtocolContext>,
    shutdown: CancellationToken,
    expected_fingerprint: u64,
    events: mpsc::Sender<RuntimeEvent>,
) -> JoinHandle<Result<()>> {
    tokio::spawn(async move {
        let mut unknown_monitor =
            UnknownPacketMonitor::new(UNKNOWN_PACKET_WINDOW, UNKNOWN_PACKET_THRESHOLD);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                maybe_frame = receiver.recv() => {
                    let Some(frame) = maybe_frame else { break; };
                    let decoded = match cobs_decode(&frame) {
                        Ok(decoded) => decoded,
                        Err(err) => {
                            debug!(error = %err, frame_len = frame.len(), "dropping invalid COBS frame");
                            continue;
                        }
                    };
                    if decoded.is_empty() {
                        continue;
                    }
                    let id = decoded[0];
                    let payload = decoded[1..].to_vec();
                    if let Some(fingerprint) = decode_init_magic_packet(id, &payload) {
                        if fingerprint != expected_fingerprint {
                            let error = anyhow!(
                                "init magic fingerprint mismatch: firmware=0x{:016X}, generated=0x{:016X}",
                                fingerprint,
                                expected_fingerprint
                            );
                            error!(
                                firmware_fingerprint = format!("0x{:016X}", fingerprint),
                                generated_fingerprint = format!("0x{:016X}", expected_fingerprint),
                                "librat init magic fingerprint mismatch"
                            );
                            shutdown.cancel();
                            return Err(error);
                        }
                        let _ = events.try_send(RuntimeEvent::InitMagicVerified);
                        info!(
                            fingerprint = format!("0x{:016X}", fingerprint),
                            "received librat init magic packet (fingerprint verified)"
                        );
                        continue;
                    }
                    let data = match protocol.parse_packet(id, &payload) {
                        Ok(data) => data,
                        Err(ProtocolError::UnknownPacketId(unknown_id)) => {
                            let observation = unknown_monitor.record(unknown_id);
                            if let Some(report) = observation.rolled_over {
                                warn!(
                                    window_secs = unknown_monitor.window.as_secs(),
                                    dropped = report.count,
                                    unique_ids = report.unique_ids,
                                    "unknown packets dropped in previous window"
                                );
                            }
                            if observation.threshold_crossed {
                                error!(
                                    packet_id = format!("0x{:02X}", unknown_id),
                                    window_secs = unknown_monitor.window.as_secs(),
                                    threshold = unknown_monitor.threshold,
                                    window_count = observation.window_count,
                                    total_unknown = observation.total_count,
                                    "unknown packet flood detected (not declared in rat_gen.toml)"
                                );
                            } else {
                                warn!(
                                    packet_id = format!("0x{:02X}", unknown_id),
                                    window_count = observation.window_count,
                                    total_unknown = observation.total_count,
                                    "dropping unknown packet id (not declared in rat_gen.toml)"
                                );
                            }
                            continue;
                        }
                        Err(err) => {
                            warn!(packet_id = format!("0x{:02X}", id), error = %err, payload_len = payload.len(), "dropping undecodable packet");
                            continue;
                        }
                    };
                    hub.publish(RatPacket {
                        id,
                        timestamp: SystemTime::now(),
                        payload,
                        data,
                    });
                }
            }
        }
        Ok(())
    })
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

    #[test]
    fn decode_init_magic_packet_extracts_fingerprint() {
        let payload = [
            b'R', b'A', b'T', b'I', 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
        ];
        let fp = decode_init_magic_packet(0x00, &payload).expect("decode");
        assert_eq!(fp, 0x1122_3344_5566_7788);
    }

    #[test]
    fn unknown_packet_monitor_triggers_threshold_once_per_window() {
        let mut monitor = UnknownPacketMonitor::new(Duration::from_secs(10), 3);
        let start = Instant::now();

        let first = monitor.record_at(0x10, start);
        assert!(!first.threshold_crossed);

        let second = monitor.record_at(0x10, start + Duration::from_millis(1));
        assert!(!second.threshold_crossed);

        let third = monitor.record_at(0x10, start + Duration::from_millis(2));
        assert!(third.threshold_crossed);

        let fourth = monitor.record_at(0x10, start + Duration::from_millis(3));
        assert!(!fourth.threshold_crossed);
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
}
