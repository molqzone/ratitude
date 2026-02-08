use std::env;
mod backend;
use std::fs::File;
use std::io::{self, Write};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context, Result};
use backend::BackendRuntime;
use clap::{Args, Parser, Subcommand};
use rat_bridge_foxglove::{run_bridge, BridgeConfig};
use rat_config::{
    load_generated_or_default, load_or_default, BackendConfig, BackendType, FieldDef, PacketDef,
    RatitudeConfig, DEFAULT_CONFIG_PATH,
};
use rat_core::{spawn_jsonl_writer, spawn_listener, Hub, ListenerOptions};
use rat_protocol::{cobs_decode, DynamicFieldDef, DynamicPacketDef, ProtocolContext, RatPacket};
use rat_sync::sync_packets;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "rttd", about = "Ratitude host runtime")]
struct Cli {
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
    #[arg(long)]
    config: Option<String>,
    #[arg(long)]
    addr: Option<String>,
    #[arg(long)]
    log: Option<String>,
    #[arg(long = "text-id")]
    text_id: Option<String>,
    #[arg(long)]
    reconnect: Option<String>,
    #[arg(long)]
    buf: Option<usize>,
    #[arg(long = "reader-buf")]
    reader_buf: Option<usize>,
    #[command(flatten)]
    backend: BackendArgs,
}

#[derive(Args, Debug, Clone)]
struct FoxgloveArgs {
    #[arg(long)]
    config: Option<String>,
    #[arg(long)]
    addr: Option<String>,
    #[arg(long = "ws-addr")]
    ws_addr: Option<String>,
    #[arg(long = "text-id")]
    text_id: Option<String>,
    #[arg(long = "quat-id")]
    quat_id: Option<String>,
    #[arg(long = "temp-id")]
    temp_id: Option<String>,
    #[arg(long)]
    reconnect: Option<String>,
    #[arg(long)]
    buf: Option<usize>,
    #[arg(long = "reader-buf")]
    reader_buf: Option<usize>,
    #[arg(long)]
    topic: Option<String>,
    #[arg(long = "schema-name")]
    schema_name: Option<String>,
    #[arg(long = "marker-topic")]
    marker_topic: Option<String>,
    #[arg(long = "parent-frame")]
    parent_frame: Option<String>,
    #[arg(long = "frame-id")]
    frame_id: Option<String>,
    #[arg(long = "image-path")]
    image_path: Option<String>,
    #[arg(long = "image-frame")]
    image_frame: Option<String>,
    #[arg(long = "image-format")]
    image_format: Option<String>,
    #[arg(long = "log-topic")]
    log_topic: Option<String>,
    #[arg(long = "log-name")]
    log_name: Option<String>,
    #[arg(long)]
    mock: bool,
    #[arg(long = "mock-hz")]
    mock_hz: Option<u32>,
    #[arg(long = "mock-id")]
    mock_id: Option<String>,
    #[command(flatten)]
    backend: BackendArgs,
}

#[derive(Args, Debug, Clone, Default)]
struct BackendArgs {
    #[arg(long)]
    backend: Option<String>,
    #[arg(long = "auto-start-backend", default_value_t = false)]
    auto_start_backend: bool,
    #[arg(long = "no-auto-start-backend", default_value_t = false)]
    no_auto_start_backend: bool,
    #[arg(long = "backend-timeout-ms")]
    backend_timeout_ms: Option<u64>,
    #[arg(long = "openocd-elf")]
    openocd_elf: Option<String>,
    #[arg(long = "openocd-symbol")]
    openocd_symbol: Option<String>,
    #[arg(long = "openocd-interface")]
    openocd_interface: Option<String>,
    #[arg(long = "openocd-target")]
    openocd_target: Option<String>,
    #[arg(long = "openocd-transport")]
    openocd_transport: Option<String>,
    #[arg(long = "openocd-speed")]
    openocd_speed: Option<u32>,
    #[arg(long = "openocd-polling")]
    openocd_polling: Option<u32>,
    #[arg(long = "openocd-disable-debug-ports")]
    openocd_disable_debug_ports: Option<bool>,
    #[arg(long = "jlink-device")]
    jlink_device: Option<String>,
    #[arg(long = "jlink-if")]
    jlink_if: Option<String>,
    #[arg(long = "jlink-speed")]
    jlink_speed: Option<u32>,
    #[arg(long = "jlink-serial")]
    jlink_serial: Option<String>,
    #[arg(long = "jlink-ip")]
    jlink_ip: Option<String>,
    #[arg(long = "jlink-rtt-port")]
    jlink_rtt_port: Option<u16>,
}

#[derive(Args, Debug, Clone)]
struct SyncArgs {
    #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
    config: String,
    #[arg(long = "scan-root")]
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
    let raw_args: Vec<String> = env::args().collect();
    auto_sync_before_parse(&raw_args)?;

    let cli = Cli::parse();
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

fn auto_sync_before_parse(args: &[String]) -> Result<()> {
    let command = detect_command(args);
    let is_help = args
        .iter()
        .any(|arg| arg == "-h" || arg == "--help" || arg == "help");
    if is_help {
        return Ok(());
    }

    match command.as_deref() {
        Some("server") | Some("foxglove") | None => {
            let config_path = extract_config_path(args, DEFAULT_CONFIG_PATH)?;
            let result = sync_packets(&config_path, None)?;
            debug!(config = %config_path, changed = result.changed, packets = result.config.packets.len(), "auto packet sync completed");
            Ok(())
        }
        _ => Ok(()),
    }
}

fn detect_command(args: &[String]) -> Option<String> {
    if args.len() < 2 {
        return None;
    }
    let cmd = args[1].as_str();
    if cmd.starts_with('-') {
        None
    } else {
        Some(cmd.to_string())
    }
}

fn extract_config_path(args: &[String], fallback: &str) -> Result<String> {
    for (idx, arg) in args.iter().enumerate() {
        if arg == "--config" {
            let value = args
                .get(idx + 1)
                .ok_or_else(|| anyhow!("--config requires a value"))?;
            return Ok(value.clone());
        }
        if let Some(value) = arg.strip_prefix("--config=") {
            if value.is_empty() {
                return Err(anyhow!("--config requires a value"));
            }
            return Ok(value.to_string());
        }
    }
    Ok(fallback.to_string())
}

fn run_sync(args: SyncArgs) -> Result<()> {
    let scan_root_override = args.scan_root.as_deref().map(std::path::Path::new);
    let result = sync_packets(&args.config, scan_root_override)?;
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

fn load_generated_packets(cfg: &mut RatitudeConfig) -> Result<()> {
    let generated_path = cfg.generated_toml_path().to_path_buf();
    let (generated, exists) = load_generated_or_default(&generated_path)?;
    if !exists {
        warn!(path = %generated_path.display(), "generated packet config not found, packets are empty");
        cfg.packets.clear();
        return Ok(());
    }
    cfg.packets = generated.to_packet_defs();
    cfg.validate()?;
    Ok(())
}

async fn run_server(args: ServerArgs) -> Result<()> {
    let config_path = args
        .config
        .unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());
    let (mut cfg, _) = load_or_default(&config_path)?;
    load_generated_packets(&mut cfg)?;

    let addr = args.addr.unwrap_or_else(|| cfg.rttd.server.addr.clone());
    let text_id = parse_u8_id(args.text_id.as_deref(), cfg.rttd.text_id)?;
    let reconnect = parse_duration(args.reconnect.as_deref(), &cfg.rttd.server.reconnect)?;
    let buf = args.buf.unwrap_or(cfg.rttd.server.buf);
    let _reader_buf = args.reader_buf.unwrap_or(cfg.rttd.server.reader_buf);

    let backend_cfg = merge_backend_config(&cfg.rttd.server.backend, &args.backend, &addr)?;
    let mut backend_runtime = BackendRuntime::start(&backend_cfg, &addr).await?;

    info!(
        mode = "server",
        config = %config_path,
        addr = %addr,
        text_id = format!("0x{:02X}", text_id),
        backend = ?backend_cfg.backend_type,
        auto_start_backend = backend_cfg.auto_start,
        "starting server runtime"
    );

    let mut protocol = ProtocolContext::new();
    protocol.set_text_packet_id(text_id);
    register_dynamic_packets(&mut protocol, &cfg.packets)?;
    let protocol = Arc::new(protocol);

    let shutdown = CancellationToken::new();
    let hub = Hub::new(buf.max(1));
    let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>(buf.max(1));

    let listener = spawn_listener(
        shutdown.clone(),
        addr,
        frame_tx,
        ListenerOptions {
            reconnect,
            reconnect_max: Duration::from_secs(30),
            dial_timeout: Duration::from_secs(5),
            strip_jlink_banner: matches!(backend_cfg.backend_type, BackendType::Jlink),
        },
    );

    let consume_task =
        spawn_frame_consumer(frame_rx, hub.clone(), protocol.clone(), shutdown.clone());

    let writer: Box<dyn Write + Send> = if let Some(log_path) = args.log {
        Box::new(File::create(log_path).context("failed to open log file")?)
    } else {
        Box::new(io::stdout())
    };
    let writer = Arc::new(Mutex::new(writer));
    let log_task = spawn_jsonl_writer(hub.subscribe(), writer);

    tokio::signal::ctrl_c()
        .await
        .context("failed to wait ctrl-c")?;
    info!("received ctrl-c, shutting down server runtime");
    shutdown.cancel();

    listener.abort();
    let _ = listener.await;
    let _ = consume_task.await;
    drop(hub);
    let _ = log_task.await;
    backend_runtime.shutdown().await;
    Ok(())
}

async fn run_foxglove(args: FoxgloveArgs) -> Result<()> {
    let config_path = args
        .config
        .unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());
    let (mut cfg, _) = load_or_default(&config_path)?;
    load_generated_packets(&mut cfg)?;

    let addr = args.addr.unwrap_or_else(|| cfg.rttd.server.addr.clone());
    let text_id = parse_u8_id(args.text_id.as_deref(), cfg.rttd.text_id)?;
    let reconnect = parse_duration(args.reconnect.as_deref(), &cfg.rttd.server.reconnect)?;
    let buf = args.buf.unwrap_or(cfg.rttd.server.buf);

    let resolved_quat_default = choose_default_quat_id(&cfg);
    let quat_id = parse_u8_id(args.quat_id.as_deref(), resolved_quat_default as u16)?;
    let temp_id = parse_u8_id(args.temp_id.as_deref(), cfg.rttd.foxglove.temp_id)?;

    let backend_cfg = merge_backend_config(&cfg.rttd.server.backend, &args.backend, &addr)?;

    info!(
        mode = "foxglove",
        config = %config_path,
        addr = %addr,
        quat_id = format!("0x{:02X}", quat_id),
        temp_id = format!("0x{:02X}", temp_id),
        mock = args.mock,
        backend = ?backend_cfg.backend_type,
        auto_start_backend = backend_cfg.auto_start,
        "starting foxglove runtime"
    );

    let mut protocol = ProtocolContext::new();
    protocol.set_text_packet_id(text_id);
    protocol.register_static_quat(quat_id);
    protocol.register_static_temperature(temp_id);
    register_dynamic_packets(&mut protocol, &cfg.packets)?;
    let protocol = Arc::new(protocol);

    let shutdown = CancellationToken::new();
    let hub = Hub::new(buf.max(1));

    let bridge_cfg = BridgeConfig {
        ws_addr: args
            .ws_addr
            .unwrap_or_else(|| cfg.rttd.foxglove.ws_addr.clone()),
        topic: args
            .topic
            .unwrap_or_else(|| cfg.rttd.foxglove.topic.clone()),
        schema_name: args
            .schema_name
            .unwrap_or_else(|| cfg.rttd.foxglove.schema_name.clone()),
        marker_topic: args
            .marker_topic
            .unwrap_or_else(|| cfg.rttd.foxglove.marker_topic.clone()),
        parent_frame_id: args
            .parent_frame
            .unwrap_or_else(|| cfg.rttd.foxglove.parent_frame.clone()),
        frame_id: args
            .frame_id
            .unwrap_or_else(|| cfg.rttd.foxglove.frame_id.clone()),
        image_path: choose_image_path(args.image_path.clone(), &cfg),
        image_frame_id: args
            .image_frame
            .unwrap_or_else(|| cfg.rttd.foxglove.image_frame.clone()),
        image_format: args
            .image_format
            .unwrap_or_else(|| cfg.rttd.foxglove.image_format.clone()),
        log_topic: args
            .log_topic
            .unwrap_or_else(|| cfg.rttd.foxglove.log_topic.clone()),
        log_name: args
            .log_name
            .unwrap_or_else(|| cfg.rttd.foxglove.log_name.clone()),
        ..BridgeConfig::default()
    };

    let bridge_task = tokio::spawn(run_bridge(
        bridge_cfg,
        hub.clone(),
        text_id,
        quat_id,
        shutdown.clone(),
    ));

    let mut backend_runtime = BackendRuntime::disabled();
    let mut listener_task: Option<JoinHandle<()>> = None;
    let mut consume_task: Option<JoinHandle<()>> = None;
    let mut mock_task: Option<JoinHandle<()>> = None;

    if args.mock {
        let mock_hz = args.mock_hz.unwrap_or(50).max(1);
        let mock_id = parse_u8_id(args.mock_id.as_deref(), resolved_quat_default)?;
        mock_task = Some(spawn_mock_publisher(
            hub.clone(),
            text_id,
            temp_id,
            mock_id,
            mock_hz,
            shutdown.clone(),
        ));
    } else {
        backend_runtime = BackendRuntime::start(&backend_cfg, &addr).await?;
        let _reader_buf = args.reader_buf.unwrap_or(cfg.rttd.server.reader_buf);
        let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>(buf.max(1));
        listener_task = Some(spawn_listener(
            shutdown.clone(),
            addr,
            frame_tx,
            ListenerOptions {
                reconnect,
                reconnect_max: Duration::from_secs(30),
                dial_timeout: Duration::from_secs(5),
                strip_jlink_banner: matches!(backend_cfg.backend_type, BackendType::Jlink),
            },
        ));
        consume_task = Some(spawn_frame_consumer(
            frame_rx,
            hub.clone(),
            protocol.clone(),
            shutdown.clone(),
        ));
    }

    tokio::signal::ctrl_c()
        .await
        .context("failed to wait ctrl-c")?;
    info!("received ctrl-c, shutting down foxglove runtime");
    shutdown.cancel();

    if let Some(task) = mock_task {
        let _ = task.await;
    }
    if let Some(task) = listener_task {
        task.abort();
        let _ = task.await;
    }
    if let Some(task) = consume_task {
        let _ = task.await;
    }

    drop(hub);

    let bridge_result = match bridge_task.await {
        Ok(result) => result,
        Err(err) => Err(anyhow!("foxglove task failed: {err}")),
    };

    backend_runtime.shutdown().await;
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

fn choose_image_path(cli_image_path: Option<String>, cfg: &RatitudeConfig) -> String {
    cli_image_path.unwrap_or_else(|| resolve_toml_relative_path(cfg, &cfg.rttd.foxglove.image_path))
}

fn resolve_toml_relative_path(cfg: &RatitudeConfig, raw: &str) -> String {
    let resolved = cfg.resolve_relative_path(raw);
    if resolved.as_os_str().is_empty() {
        return String::new();
    }
    resolved.to_string_lossy().to_string()
}

fn choose_default_quat_id(cfg: &RatitudeConfig) -> u16 {
    let explicit = cfg.rttd.foxglove.quat_id;
    if explicit != 0 && cfg.packets.iter().any(|packet| packet.id == explicit) {
        return explicit;
    }

    cfg.packets
        .iter()
        .find(|packet| packet.packet_type.eq_ignore_ascii_case("pose_3d"))
        .map(|packet| packet.id)
        .unwrap_or(explicit.max(0x10))
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

fn spawn_frame_consumer(
    mut receiver: mpsc::Receiver<Vec<u8>>,
    hub: Hub,
    protocol: Arc<ProtocolContext>,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
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
                        info!(fingerprint = format!("0x{:016X}", fingerprint), "received librat init magic packet");
                        continue;
                    }
                    let data = match protocol.parse_packet(id, &payload) {
                        Ok(data) => data,
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
    })
}

fn spawn_mock_publisher(
    hub: Hub,
    text_id: u8,
    temp_id: u8,
    mock_id: u8,
    hz: u32,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let hz = hz.max(1);
        let mut quat_ticker = tokio::time::interval(Duration::from_secs_f64(1.0 / hz as f64));
        let mut log_ticker = tokio::time::interval(Duration::from_secs(1));
        let start = SystemTime::now();
        let mut seq: i64 = 0;

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = quat_ticker.tick() => {
                    let now = SystemTime::now();
                    let t = now.duration_since(start).unwrap_or_else(|_| Duration::from_secs(0)).as_secs_f64();
                    let quat = mock_quaternion(t);
                    let payload = {
                        let mut data = Vec::with_capacity(16);
                        data.extend_from_slice(&quat.w.to_le_bytes());
                        data.extend_from_slice(&quat.x.to_le_bytes());
                        data.extend_from_slice(&quat.y.to_le_bytes());
                        data.extend_from_slice(&quat.z.to_le_bytes());
                        data
                    };
                    hub.publish(RatPacket {
                        id: mock_id,
                        timestamp: now,
                        payload,
                        data: rat_protocol::PacketData::Quat(quat.clone()),
                    });

                    let celsius = mock_temperature(t);
                    hub.publish(RatPacket {
                        id: temp_id,
                        timestamp: now,
                        payload: celsius.to_le_bytes().to_vec(),
                        data: rat_protocol::PacketData::Temperature(rat_protocol::TemperaturePacket { celsius }),
                    });
                    seq += 1;
                }
                _ = log_ticker.tick() => {
                    let text = format!("rat_info mock seq={}", seq);
                    hub.publish(RatPacket {
                        id: text_id,
                        timestamp: SystemTime::now(),
                        payload: text.as_bytes().to_vec(),
                        data: rat_protocol::PacketData::Text(text),
                    });
                }
            }
        }
    })
}

fn mock_temperature(t: f64) -> f32 {
    (36.5 + 3.5 * (2.0 * std::f64::consts::PI * 0.08 * t + std::f64::consts::PI / 5.0).sin()) as f32
}

fn mock_quaternion(t: f64) -> rat_protocol::QuatPacket {
    let roll = 35_f64.to_radians() * (2.0 * std::f64::consts::PI * 0.23 * t).sin();
    let pitch = 25_f64.to_radians()
        * (2.0 * std::f64::consts::PI * 0.31 * t + std::f64::consts::PI / 3.0).sin();
    let yaw = 40_f64.to_radians()
        * (2.0 * std::f64::consts::PI * 0.17 * t + 2.0 * std::f64::consts::PI / 3.0).sin();

    let cr = (roll * 0.5).cos();
    let sr = (roll * 0.5).sin();
    let cp = (pitch * 0.5).cos();
    let sp = (pitch * 0.5).sin();
    let cy = (yaw * 0.5).cos();
    let sy = (yaw * 0.5).sin();

    let w = cr * cp * cy + sr * sp * sy;
    let x = sr * cp * cy - cr * sp * sy;
    let y = cr * sp * cy + sr * cp * sy;
    let z = cr * cp * sy - sr * sp * cy;

    let norm = (w * w + x * x + y * y + z * z).sqrt();
    let inv = if norm == 0.0 { 1.0 } else { 1.0 / norm };

    rat_protocol::QuatPacket {
        w: (w * inv) as f32,
        x: (x * inv) as f32,
        y: (y * inv) as f32,
        z: (z * inv) as f32,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn build_test_config(config_path: &str, image_path: &str) -> RatitudeConfig {
        let mut cfg = RatitudeConfig::default();
        cfg.rttd.foxglove.image_path = image_path.to_string();
        cfg.normalize(Path::new(config_path));
        cfg
    }

    #[test]
    fn resolve_toml_relative_path_uses_config_dir() {
        let cfg = build_test_config("configs/rat.toml", "demo.jpg");
        let resolved = choose_image_path(None, &cfg);
        assert!(PathBuf::from(resolved).ends_with(Path::new("configs").join("demo.jpg")));
    }

    #[test]
    fn resolve_toml_absolute_path_keeps_original() {
        let absolute = std::env::temp_dir().join("demo.jpg");
        let cfg = build_test_config("configs/rat.toml", absolute.to_string_lossy().as_ref());
        let resolved = choose_image_path(None, &cfg);
        assert_eq!(PathBuf::from(resolved), absolute);
    }

    #[test]
    fn cli_image_path_keeps_cwd_semantics() {
        let cfg = build_test_config("configs/rat.toml", "demo.jpg");
        let resolved = choose_image_path(Some("assets/demo.jpg".to_string()), &cfg);
        assert_eq!(resolved, "assets/demo.jpg");
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
