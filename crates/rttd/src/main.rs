mod backend;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Context, Result};
use backend::BackendRuntime;
use clap::{Args, Parser, Subcommand};
use rat_bridge_foxglove::{run_bridge, BridgeConfig};
use rat_config::{
    load_generated_or_default, load_or_default, BackendConfig, BackendType, FieldDef, PacketDef,
    RatitudeConfig, DEFAULT_CONFIG_PATH,
};
use rat_core::{spawn_jsonl_writer, spawn_listener, Hub, ListenerOptions};
use rat_protocol::{
    cobs_decode, DynamicFieldDef, DynamicPacketDef, ProtocolContext, ProtocolError, RatPacket,
};
use rat_sync::sync_packets;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "rttd", about = "Ratitude host runtime")]
struct Cli {
    #[arg(
        long = "no-auto-sync",
        global = true,
        default_value_t = false,
        help = "Disable startup auto `rttd sync` for server/foxglove runs",
        long_help = "By default `rttd server` and `rttd foxglove` run `rttd sync` before startup. Use --no-auto-sync to disable that behavior for this run."
    )]
    no_auto_sync: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Server(ServerArgs),
    Foxglove(FoxgloveArgs),
    Sync(SyncArgs),
}

#[derive(Args, Debug, Clone)]
struct ServerArgs {
    #[arg(long, help = "Path to rat.toml config file")]
    config: Option<String>,
    #[arg(long, help = "RTT TCP source address")]
    addr: Option<String>,
    #[arg(long, help = "JSONL output file path (default: stdout)")]
    log: Option<String>,
    #[arg(long = "text-id", help = "Text packet ID (hex like 0xFF or decimal)")]
    text_id: Option<String>,
    #[arg(long, help = "Reconnect interval, e.g. 1s")]
    reconnect: Option<String>,
    #[arg(long, help = "Frame channel buffer size")]
    buf: Option<usize>,
    #[arg(
        long = "reader-buf",
        help = "Zero-delimited frame reader buffer size (bytes)"
    )]
    reader_buf: Option<usize>,
    #[command(flatten)]
    backend: BackendArgs,
}

#[derive(Args, Debug, Clone)]
struct FoxgloveArgs {
    #[arg(long, help = "Path to rat.toml config file")]
    config: Option<String>,
    #[arg(long, help = "RTT TCP source address")]
    addr: Option<String>,
    #[arg(long = "ws-addr", help = "Foxglove websocket listen address")]
    ws_addr: Option<String>,
    #[arg(long, help = "Reconnect interval, e.g. 1s")]
    reconnect: Option<String>,
    #[arg(long, help = "Frame channel buffer size")]
    buf: Option<usize>,
    #[command(flatten)]
    backend: BackendArgs,
}

#[derive(Args, Debug, Clone, Default)]
struct BackendArgs {
    #[arg(long, help = "Backend type: none|openocd|jlink")]
    backend: Option<String>,
    #[arg(
        long = "auto-start-backend",
        default_value_t = false,
        help = "Auto-start backend process"
    )]
    auto_start_backend: bool,
    #[arg(
        long = "no-auto-start-backend",
        default_value_t = false,
        help = "Disable backend auto-start"
    )]
    no_auto_start_backend: bool,
    #[arg(
        long = "backend-timeout-ms",
        help = "Backend startup timeout in milliseconds"
    )]
    backend_timeout_ms: Option<u64>,
    #[arg(long = "openocd-elf", help = "OpenOCD ELF path override")]
    openocd_elf: Option<String>,
    #[arg(long = "openocd-symbol", help = "OpenOCD RTT symbol name override")]
    openocd_symbol: Option<String>,
    #[arg(long = "openocd-interface", help = "OpenOCD interface cfg override")]
    openocd_interface: Option<String>,
    #[arg(long = "openocd-target", help = "OpenOCD target cfg override")]
    openocd_target: Option<String>,
    #[arg(long = "openocd-transport", help = "OpenOCD transport override")]
    openocd_transport: Option<String>,
    #[arg(long = "openocd-speed", help = "OpenOCD adapter speed override")]
    openocd_speed: Option<u32>,
    #[arg(
        long = "openocd-polling",
        help = "OpenOCD RTT polling interval override"
    )]
    openocd_polling: Option<u32>,
    #[arg(
        long = "openocd-disable-debug-ports",
        help = "OpenOCD disable debug ports override"
    )]
    openocd_disable_debug_ports: Option<bool>,
    #[arg(long = "jlink-device", help = "J-Link device override")]
    jlink_device: Option<String>,
    #[arg(long = "jlink-if", help = "J-Link interface override")]
    jlink_if: Option<String>,
    #[arg(long = "jlink-speed", help = "J-Link speed override")]
    jlink_speed: Option<u32>,
    #[arg(long = "jlink-serial", help = "J-Link serial override")]
    jlink_serial: Option<String>,
    #[arg(long = "jlink-ip", help = "J-Link probe IP override")]
    jlink_ip: Option<String>,
    #[arg(long = "jlink-rtt-port", help = "J-Link RTT telnet port override")]
    jlink_rtt_port: Option<u16>,
}

#[derive(Args, Debug, Clone)]
struct SyncArgs {
    #[arg(
        long,
        default_value = DEFAULT_CONFIG_PATH,
        help = "Path to rat.toml config file"
    )]
    config: String,
    #[arg(long = "scan-root", help = "Override project.scan_root for this run")]
    scan_root: Option<String>,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    init_tracing();

    let exit = run().await;
    if let Err(err) = exit {
        error!(error = %err, "rttd exited with error");
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    maybe_auto_sync(&cli)?;
    match cli.command {
        Some(Command::Server(args)) => run_server(args).await,
        Some(Command::Foxglove(args)) => run_foxglove(args).await,
        Some(Command::Sync(args)) => run_sync(args),
        None => {
            run_server(ServerArgs {
                config: None,
                addr: None,
                log: None,
                text_id: None,
                reconnect: None,
                buf: None,
                reader_buf: None,
                backend: BackendArgs::default(),
            })
            .await
        }
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

fn maybe_auto_sync(cli: &Cli) -> Result<()> {
    if cli.no_auto_sync {
        debug!("auto packet sync disabled by --no-auto-sync");
        return Ok(());
    }

    match cli.command.as_ref() {
        Some(Command::Sync(_)) => Ok(()),
        Some(Command::Server(args)) => auto_sync_for_config(args.config.as_deref()),
        Some(Command::Foxglove(args)) => auto_sync_for_config(args.config.as_deref()),
        None => auto_sync_for_config(None),
    }
}

fn auto_sync_for_config(config_override: Option<&str>) -> Result<()> {
    let config_path = config_override.unwrap_or(DEFAULT_CONFIG_PATH);
    let result = sync_packets(config_path, None)?;
    for warning_message in &result.layout_warnings {
        warn!(config = %config_path, warning = %warning_message, "rat-sync layout warning");
    }
    debug!(
        config = %config_path,
        changed = result.changed,
        packets = result.config.packets.len(),
        "auto packet sync completed"
    );
    Ok(())
}

fn run_config_path(config_override: Option<String>) -> String {
    if let Some(path) = config_override {
        return path;
    }
    DEFAULT_CONFIG_PATH.to_string()
}

struct IngestRuntime {
    shutdown: CancellationToken,
    hub: Hub,
    listener_task: JoinHandle<()>,
    consume_task: JoinHandle<Result<()>>,
    backend_runtime: BackendRuntime,
}

async fn start_ingest_runtime(
    addr: String,
    reconnect: Duration,
    buf: usize,
    reader_buf: usize,
    backend_cfg: &BackendConfig,
    protocol: Arc<ProtocolContext>,
    expected_fingerprint: u64,
) -> Result<IngestRuntime> {
    let shutdown = CancellationToken::new();
    let hub = Hub::new(buf.max(1));
    let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>(buf.max(1));

    let backend_runtime = BackendRuntime::start(backend_cfg, &addr).await?;
    let listener_task = spawn_listener(
        shutdown.clone(),
        addr,
        frame_tx,
        ListenerOptions {
            reconnect,
            reconnect_max: Duration::from_secs(30),
            dial_timeout: Duration::from_secs(5),
            strip_jlink_banner: matches!(backend_cfg.backend_type, BackendType::Jlink),
            reader_buf_bytes: reader_buf,
        },
    );

    let consume_task = spawn_frame_consumer(
        frame_rx,
        hub.clone(),
        protocol,
        shutdown.clone(),
        expected_fingerprint,
    );

    Ok(IngestRuntime {
        shutdown,
        hub,
        listener_task,
        consume_task,
        backend_runtime,
    })
}

async fn wait_runtime_stop(
    mode: &str,
    consume_task: &mut JoinHandle<Result<()>>,
) -> Result<Option<anyhow::Error>> {
    tokio::select! {
        ctrl = tokio::signal::ctrl_c() => {
            ctrl.context("failed to wait ctrl-c")?;
            info!(mode, "received ctrl-c, shutting down runtime");
            Ok(None)
        }
        result = consume_task => {
            Ok(Some(frame_consumer_failure(result)))
        }
    }
}

async fn shutdown_ingest_runtime(runtime: IngestRuntime, join_consumer: bool) {
    let IngestRuntime {
        shutdown,
        hub: _hub,
        listener_task,
        consume_task,
        mut backend_runtime,
    } = runtime;

    shutdown.cancel();
    listener_task.abort();
    let _ = listener_task.await;
    if join_consumer {
        let _ = consume_task.await;
    }
    backend_runtime.shutdown().await;
}

fn run_sync(args: SyncArgs) -> Result<()> {
    let scan_root_override = args.scan_root.as_deref().map(std::path::Path::new);
    let result = sync_packets(&args.config, scan_root_override)?;
    for warning_message in &result.layout_warnings {
        warn!(config = %args.config, warning = %warning_message, "rat-sync layout warning");
        eprintln!("[Sync][Warn] {warning_message}");
    }
    info!(config = %args.config, changed = result.changed, packets = result.config.packets.len(), "manual packet sync finished");
    if result.changed {
        println!(
            "[Sync] Updated {} with {} packet(s)",
            args.config,
            result.config.packets.len()
        );
    } else {
        println!(
            "[Sync] No packet changes in {} ({} packet(s))",
            args.config,
            result.config.packets.len()
        );
    }
    Ok(())
}

fn load_generated_packets(cfg: &mut RatitudeConfig) -> Result<u64> {
    let generated_path = cfg.generated_toml_path().to_path_buf();
    let (generated, exists) = load_generated_or_default(&generated_path)?;
    if !exists {
        return Err(anyhow!(
            "rat_gen.toml not found at {}; run `rttd sync --config <path>` first",
            generated_path.display()
        ));
    }
    if generated.packets.is_empty() {
        return Err(anyhow!(
            "rat_gen.toml has no packets; server and foxglove modes require generated declarations"
        ));
    }
    let expected_fingerprint = parse_generated_fingerprint(&generated.meta.fingerprint)
        .with_context(|| format!("invalid fingerprint in {}", generated_path.display()))?;

    cfg.packets = generated.to_packet_defs();
    cfg.validate()?;
    Ok(expected_fingerprint)
}

async fn run_server(args: ServerArgs) -> Result<()> {
    let config_path = run_config_path(args.config.clone());
    let (mut cfg, _) = load_or_default(&config_path)?;
    let expected_fingerprint = load_generated_packets(&mut cfg)?;

    let addr = args.addr.unwrap_or_else(|| cfg.rttd.server.addr.clone());
    let text_id = parse_u8_id(args.text_id.as_deref(), cfg.rttd.text_id)?;
    let reconnect = parse_duration(args.reconnect.as_deref(), &cfg.rttd.server.reconnect)?;
    let buf = args.buf.unwrap_or(cfg.rttd.server.buf);
    let reader_buf = args.reader_buf.unwrap_or(cfg.rttd.server.reader_buf);

    let backend_cfg = merge_backend_config(&cfg.rttd.server.backend, &args.backend, &addr)?;
    info!(
        mode = "server",
        config = %config_path,
        addr = %addr,
        text_id = format!("0x{:02X}", text_id),
        expected_fingerprint = format!("0x{:016X}", expected_fingerprint),
        backend = ?backend_cfg.backend_type,
        auto_start_backend = backend_cfg.auto_start,
        "starting server runtime"
    );

    let mut protocol = ProtocolContext::new();
    protocol.set_text_packet_id(text_id);
    register_dynamic_packets(&mut protocol, &cfg.packets)?;
    let protocol = Arc::new(protocol);

    let mut runtime = start_ingest_runtime(
        addr,
        reconnect,
        buf,
        reader_buf,
        &backend_cfg,
        protocol,
        expected_fingerprint,
    )
    .await?;

    let writer: Box<dyn Write + Send> = if let Some(log_path) = args.log {
        Box::new(File::create(log_path).context("failed to open log file")?)
    } else {
        Box::new(io::stdout())
    };
    let writer = Arc::new(Mutex::new(writer));
    let log_task = spawn_jsonl_writer(runtime.hub.subscribe(), writer);
    let stop_error = wait_runtime_stop("server", &mut runtime.consume_task).await?;
    shutdown_ingest_runtime(runtime, stop_error.is_none()).await;
    let _ = log_task.await;
    if let Some(err) = stop_error {
        return Err(err);
    }
    Ok(())
}

async fn run_foxglove(args: FoxgloveArgs) -> Result<()> {
    let config_path = run_config_path(args.config.clone());
    let (mut cfg, _) = load_or_default(&config_path)?;
    let expected_fingerprint = load_generated_packets(&mut cfg)?;

    let addr = args.addr.unwrap_or_else(|| cfg.rttd.server.addr.clone());
    let reconnect = parse_duration(args.reconnect.as_deref(), &cfg.rttd.server.reconnect)?;
    let buf = args.buf.unwrap_or(cfg.rttd.server.buf);
    let reader_buf = cfg.rttd.server.reader_buf;

    let backend_cfg = merge_backend_config(&cfg.rttd.server.backend, &args.backend, &addr)?;

    info!(
        mode = "foxglove",
        config = %config_path,
        addr = %addr,
        packets = cfg.packets.len(),
        expected_fingerprint = format!("0x{:016X}", expected_fingerprint),
        backend = ?backend_cfg.backend_type,
        auto_start_backend = backend_cfg.auto_start,
        "starting foxglove runtime"
    );

    let mut protocol = ProtocolContext::new();
    register_dynamic_packets(&mut protocol, &cfg.packets)?;
    let protocol = Arc::new(protocol);

    let mut runtime = start_ingest_runtime(
        addr,
        reconnect,
        buf,
        reader_buf,
        &backend_cfg,
        protocol,
        expected_fingerprint,
    )
    .await?;

    let bridge_cfg = BridgeConfig {
        ws_addr: args
            .ws_addr
            .unwrap_or_else(|| cfg.rttd.foxglove.ws_addr.clone()),
    };

    let bridge_task = tokio::spawn(run_bridge(
        bridge_cfg,
        cfg.packets.clone(),
        runtime.hub.clone(),
        runtime.shutdown.clone(),
    ));
    let stop_error = wait_runtime_stop("foxglove", &mut runtime.consume_task).await?;
    shutdown_ingest_runtime(runtime, stop_error.is_none()).await;

    let bridge_result = match bridge_task.await {
        Ok(result) => result,
        Err(err) => Err(anyhow!("foxglove task failed: {err}")),
    };

    if let Some(err) = stop_error {
        return Err(err);
    }
    bridge_result
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

fn parse_backend_type(raw: &str) -> Result<BackendType> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(BackendType::None),
        "openocd" => Ok(BackendType::Openocd),
        "jlink" => Ok(BackendType::Jlink),
        _ => Err(anyhow!("invalid backend type: {}", raw)),
    }
}

fn parse_port_from_addr(addr: &str) -> Option<u16> {
    addr.parse::<std::net::SocketAddr>()
        .ok()
        .map(|socket| socket.port())
        .or_else(|| {
            addr.rsplit_once(':')
                .and_then(|(_, p)| p.parse::<u16>().ok())
        })
}

fn merge_backend_config(
    base: &BackendConfig,
    cli: &BackendArgs,
    transport_addr: &str,
) -> Result<BackendConfig> {
    if cli.auto_start_backend && cli.no_auto_start_backend {
        return Err(anyhow!(
            "--auto-start-backend and --no-auto-start-backend are mutually exclusive"
        ));
    }

    let mut merged = base.clone();

    if let Some(raw) = cli.backend.as_deref() {
        merged.backend_type = parse_backend_type(raw)?;
    }

    if cli.auto_start_backend {
        merged.auto_start = true;
    }
    if cli.no_auto_start_backend {
        merged.auto_start = false;
    }

    if let Some(timeout_ms) = cli.backend_timeout_ms {
        merged.startup_timeout_ms = timeout_ms;
    }

    if let Some(value) = &cli.openocd_elf {
        merged.openocd.elf = value.clone();
    }
    if let Some(value) = &cli.openocd_symbol {
        merged.openocd.symbol = value.clone();
    }
    if let Some(value) = &cli.openocd_interface {
        merged.openocd.interface = value.clone();
    }
    if let Some(value) = &cli.openocd_target {
        merged.openocd.target = value.clone();
    }
    if let Some(value) = &cli.openocd_transport {
        merged.openocd.transport = value.clone();
    }
    if let Some(value) = cli.openocd_speed {
        merged.openocd.speed = value;
    }
    if let Some(value) = cli.openocd_polling {
        merged.openocd.polling = value;
    }
    if let Some(value) = cli.openocd_disable_debug_ports {
        merged.openocd.disable_debug_ports = value;
    }

    if let Some(value) = &cli.jlink_device {
        merged.jlink.device = value.clone();
    }
    if let Some(value) = &cli.jlink_if {
        merged.jlink.interface = value.clone();
    }
    if let Some(value) = cli.jlink_speed {
        merged.jlink.speed = value;
    }
    if let Some(value) = &cli.jlink_serial {
        merged.jlink.serial = value.clone();
    }
    if let Some(value) = &cli.jlink_ip {
        merged.jlink.ip = value.clone();
    }
    if let Some(value) = cli.jlink_rtt_port {
        merged.jlink.rtt_telnet_port = value;
    } else if let Some(port) = parse_port_from_addr(transport_addr) {
        merged.jlink.rtt_telnet_port = port;
    }

    if merged.startup_timeout_ms == 0 {
        return Err(anyhow!("backend timeout must be > 0"));
    }

    Ok(merged)
}

fn parse_u8_id(raw: Option<&str>, fallback: u16) -> Result<u8> {
    let value = raw
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("0x{:02X}", fallback));
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16).with_context(|| format!("invalid id: {}", value))?
    } else {
        u16::from_str(&value).with_context(|| format!("invalid id: {}", value))?
    };

    if parsed > 0xFF {
        return Err(anyhow!("id out of range: 0x{:X}", parsed));
    }
    Ok(parsed as u8)
}

fn parse_duration(raw: Option<&str>, fallback: &str) -> Result<Duration> {
    let value = raw.unwrap_or(fallback);
    humantime::parse_duration(value).with_context(|| format!("invalid duration: {}", value))
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

fn frame_consumer_failure(
    result: std::result::Result<Result<()>, tokio::task::JoinError>,
) -> anyhow::Error {
    match result {
        Ok(Ok(())) => anyhow!("frame consumer stopped before shutdown"),
        Ok(Err(err)) => err,
        Err(err) => anyhow!("frame consumer task failed: {err}"),
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

const UNKNOWN_PACKET_WINDOW: Duration = Duration::from_secs(5);
const UNKNOWN_PACKET_THRESHOLD: u32 = 20;

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

fn spawn_frame_consumer(
    mut receiver: mpsc::Receiver<Vec<u8>>,
    hub: Hub,
    protocol: Arc<ProtocolContext>,
    shutdown: CancellationToken,
    expected_fingerprint: u64,
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

    use clap::Parser;

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

    fn test_config_toml() -> &'static str {
        r#"
[project]
name = "demo"
scan_root = "."

[generation]
out_dir = "."
toml_name = "rat_gen.toml"
header_name = "rat_gen.h"

[rttd.server]
addr = "127.0.0.1:19021"
reconnect = "1s"
buf = 16
reader_buf = 1024

[rttd.server.backend]
type = "none"
auto_start = false
startup_timeout_ms = 1000

[rttd.server.backend.openocd]
elf = ""
symbol = "_SEGGER_RTT"
interface = "interface/cmsis-dap.cfg"
target = "target/stm32f4x.cfg"
transport = "swd"
speed = 8000
polling = 1
disable_debug_ports = true

[rttd.server.backend.jlink]
device = "STM32F407ZG"
interface = "SWD"
speed = 4000
serial = ""
ip = ""
rtt_telnet_port = 19021

[rttd.foxglove]
ws_addr = "127.0.0.1:8765"
"#
    }

    fn cobs_encode_for_test(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + 2);
        let mut code_index = 0usize;
        out.push(0);
        let mut code: u8 = 1;

        for byte in data {
            if *byte == 0 {
                out[code_index] = code;
                code_index = out.len();
                out.push(0);
                code = 1;
                continue;
            }

            out.push(*byte);
            code = code.saturating_add(1);
            if code == 0xFF {
                out[code_index] = code;
                code_index = out.len();
                out.push(0);
                code = 1;
            }
        }

        out[code_index] = code;
        out
    }

    fn encode_init_magic_frame(fingerprint: u64) -> Vec<u8> {
        let mut decoded = Vec::with_capacity(13);
        decoded.push(0x00);
        decoded.extend_from_slice(b"RATI");
        decoded.extend_from_slice(&fingerprint.to_le_bytes());
        cobs_encode_for_test(&decoded)
    }

    #[tokio::test]
    async fn run_server_rejects_missing_generated_packets_file() {
        let dir = unique_temp_dir("rttd_server_missing_gen");
        let config_path = dir.join("rat.toml");
        fs::write(&config_path, test_config_toml()).expect("write config");

        let args = ServerArgs {
            config: Some(config_path.to_string_lossy().to_string()),
            addr: None,
            log: None,
            text_id: None,
            reconnect: None,
            buf: None,
            reader_buf: None,
            backend: BackendArgs::default(),
        };

        let err = run_server(args)
            .await
            .expect_err("missing generated should fail");
        assert!(err.to_string().contains("rat_gen.toml not found at"));

        let _ = fs::remove_file(config_path);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn run_server_rejects_empty_generated_packets() {
        let dir = unique_temp_dir("rttd_server_empty_gen");
        let config_path = dir.join("rat.toml");
        let generated_path = dir.join("rat_gen.toml");

        fs::write(&config_path, test_config_toml()).expect("write config");
        fs::write(
            &generated_path,
            "packets = []\n\n[meta]\nproject = \"demo\"\nfingerprint = \"0x1\"\n",
        )
        .expect("write generated");

        let args = ServerArgs {
            config: Some(config_path.to_string_lossy().to_string()),
            addr: None,
            log: None,
            text_id: None,
            reconnect: None,
            buf: None,
            reader_buf: None,
            backend: BackendArgs::default(),
        };

        let err = run_server(args)
            .await
            .expect_err("empty packets should fail");
        assert!(err.to_string().contains(
            "rat_gen.toml has no packets; server and foxglove modes require generated declarations"
        ));

        let _ = fs::remove_file(config_path);
        let _ = fs::remove_file(generated_path);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn run_foxglove_rejects_empty_generated_packets() {
        let dir = unique_temp_dir("rttd_foxglove_empty_gen");
        let config_path = dir.join("rat.toml");
        let generated_path = dir.join("rat_gen.toml");

        fs::write(&config_path, test_config_toml()).expect("write config");
        fs::write(
            &generated_path,
            "packets = []\n\n[meta]\nproject = \"demo\"\nfingerprint = \"0x1\"\n",
        )
        .expect("write generated");

        let args = FoxgloveArgs {
            config: Some(config_path.to_string_lossy().to_string()),
            addr: None,
            ws_addr: None,
            reconnect: None,
            buf: None,
            backend: BackendArgs::default(),
        };

        let err = run_foxglove(args)
            .await
            .expect_err("empty packets should fail");
        assert!(err.to_string().contains(
            "rat_gen.toml has no packets; server and foxglove modes require generated declarations"
        ));

        let _ = fs::remove_file(config_path);
        let _ = fs::remove_file(generated_path);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_generated_fingerprint_supports_prefixed_hex() {
        let parsed = parse_generated_fingerprint("0x5C5ABDD5C9E65DA4").expect("parse hex");
        assert_eq!(parsed, 0x5C5ABDD5C9E65DA4);
    }

    #[test]
    fn parse_generated_fingerprint_rejects_empty() {
        assert!(parse_generated_fingerprint("   ").is_err());
    }

    #[test]
    fn cli_rejects_removed_auto_sync_flag() {
        let parsed = Cli::try_parse_from(["rttd", "server", "--auto-sync"]);
        assert!(parsed.is_err(), "--auto-sync should be rejected");
    }

    #[test]
    fn maybe_auto_sync_skips_sync_subcommand() {
        let cli = Cli::try_parse_from([
            "rttd",
            "sync",
            "--config",
            "/definitely/missing/path/rat.toml",
        ])
        .expect("parse cli");
        maybe_auto_sync(&cli).expect("sync command should skip startup auto-sync");
    }

    #[test]
    fn maybe_auto_sync_honors_no_auto_sync_flag() {
        let cli = Cli::try_parse_from([
            "rttd",
            "--no-auto-sync",
            "server",
            "--config",
            "/definitely/missing/path/rat.toml",
        ])
        .expect("parse cli");
        maybe_auto_sync(&cli).expect("--no-auto-sync should disable startup auto-sync");
    }

    #[test]
    fn parse_failure_does_not_touch_generated_outputs() {
        let dir = unique_temp_dir("rttd_parse_fail");
        let config_path = dir.join("rat.toml");
        let generated_path = dir.join("rat_gen.toml");

        fs::write(&config_path, test_config_toml()).expect("write config");
        assert!(
            !generated_path.exists(),
            "generated file should not exist before parse test"
        );

        let parsed = Cli::try_parse_from([
            "rttd",
            "server",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--unknown-flag",
        ]);
        assert!(parsed.is_err(), "parse should fail on unknown argument");
        assert!(
            !generated_path.exists(),
            "parse failure must not trigger auto sync side effects"
        );

        let _ = fs::remove_file(config_path);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_packet_monitor_triggers_threshold_once_per_window() {
        let mut monitor = UnknownPacketMonitor::new(Duration::from_secs(10), 3);
        let start = Instant::now();

        let first = monitor.record_at(0x10, start);
        assert!(!first.threshold_crossed);
        assert_eq!(first.window_count, 1);

        let second = monitor.record_at(0x10, start + Duration::from_millis(1));
        assert!(!second.threshold_crossed);
        assert_eq!(second.window_count, 2);

        let third = monitor.record_at(0x10, start + Duration::from_millis(2));
        assert!(third.threshold_crossed);
        assert_eq!(third.window_count, 3);

        let fourth = monitor.record_at(0x10, start + Duration::from_millis(3));
        assert!(!fourth.threshold_crossed);
        assert_eq!(fourth.window_count, 4);
    }

    #[test]
    fn unknown_packet_monitor_rolls_window_and_reports_previous_counts() {
        let mut monitor = UnknownPacketMonitor::new(Duration::from_millis(50), 10);
        let start = Instant::now();

        let _ = monitor.record_at(0x10, start);
        let _ = monitor.record_at(0x11, start + Duration::from_millis(1));
        let observation = monitor.record_at(0x12, start + Duration::from_millis(80));

        let report = observation.rolled_over.expect("window report");
        assert_eq!(report.count, 2);
        assert_eq!(report.unique_ids, 2);
        assert_eq!(observation.window_count, 1);
        assert_eq!(observation.total_count, 3);
    }

    #[test]
    fn load_generated_packets_rejects_invalid_fingerprint() {
        let dir = unique_temp_dir("rttd_invalid_fingerprint");
        let config_path = dir.join("rat.toml");
        let generated_path = dir.join("rat_gen.toml");

        fs::write(&config_path, test_config_toml()).expect("write config");
        fs::write(
            &generated_path,
            r#"[meta]
project = "demo"
fingerprint = "not-a-hex"

[[packets]]
id = 16
signature_hash = "0x1"
struct_name = "Demo"
type = "plot"
packed = true
byte_size = 4
source = "Src/main.c"

[[packets.fields]]
name = "value"
c_type = "int32_t"
offset = 0
size = 4
"#,
        )
        .expect("write generated");

        let (mut cfg, _) = load_or_default(&config_path).expect("load config");
        let err = load_generated_packets(&mut cfg).expect_err("invalid fingerprint should fail");
        assert!(err.to_string().contains("invalid fingerprint in"));

        let _ = fs::remove_file(config_path);
        let _ = fs::remove_file(generated_path);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn frame_consumer_rejects_mismatched_init_magic_fingerprint() {
        let shutdown = CancellationToken::new();
        let hub = Hub::new(8);
        let protocol = Arc::new(ProtocolContext::new());
        let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>(8);

        let expected_fingerprint = 0x1122_3344_5566_7788_u64;
        let firmware_fingerprint = 0x8877_6655_4433_2211_u64;
        let consume_task = spawn_frame_consumer(
            frame_rx,
            hub,
            protocol,
            shutdown.clone(),
            expected_fingerprint,
        );

        frame_tx
            .send(encode_init_magic_frame(firmware_fingerprint))
            .await
            .expect("send init frame");
        drop(frame_tx);

        let err = consume_task
            .await
            .expect("join consumer")
            .expect_err("mismatched fingerprint should fail");
        assert!(
            err.to_string().contains("init magic fingerprint mismatch"),
            "error: {err:#}"
        );
    }

    #[test]
    fn foxglove_cli_rejects_removed_flags() {
        let parsed = Cli::try_parse_from([
            "rttd",
            "foxglove",
            "--config",
            "firmware/example/stm32f4_rtt/rat.toml",
            "--quat-id",
            "16",
        ]);
        assert!(parsed.is_err());
    }

    #[test]
    fn parse_backend_type_supports_known_values() {
        assert!(matches!(
            parse_backend_type("none").expect("none"),
            BackendType::None
        ));
        assert!(matches!(
            parse_backend_type("openocd").expect("openocd"),
            BackendType::Openocd
        ));
        assert!(matches!(
            parse_backend_type("jlink").expect("jlink"),
            BackendType::Jlink
        ));
        assert!(parse_backend_type("bad").is_err());
    }

    #[test]
    fn merge_backend_config_prefers_cli_over_toml() {
        let mut base = BackendConfig::default();
        base.backend_type = BackendType::Openocd;
        base.auto_start = false;

        let args = BackendArgs {
            backend: Some("jlink".to_string()),
            auto_start_backend: true,
            no_auto_start_backend: false,
            backend_timeout_ms: Some(9_000),
            openocd_elf: None,
            openocd_symbol: None,
            openocd_interface: None,
            openocd_target: None,
            openocd_transport: None,
            openocd_speed: None,
            openocd_polling: None,
            openocd_disable_debug_ports: None,
            jlink_device: Some("STM32F407ZG".to_string()),
            jlink_if: Some("SWD".to_string()),
            jlink_speed: Some(8_000),
            jlink_serial: Some("12345678".to_string()),
            jlink_ip: None,
            jlink_rtt_port: Some(19029),
        };

        let merged = merge_backend_config(&base, &args, "127.0.0.1:19021").expect("merge config");
        assert!(matches!(merged.backend_type, BackendType::Jlink));
        assert!(merged.auto_start);
        assert_eq!(merged.startup_timeout_ms, 9_000);
        assert_eq!(merged.jlink.speed, 8_000);
        assert_eq!(merged.jlink.rtt_telnet_port, 19029);
    }
}
