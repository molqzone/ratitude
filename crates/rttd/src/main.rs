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
    #[arg(long)]
    reconnect: Option<String>,
    #[arg(long)]
    buf: Option<usize>,
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
    if cfg.packets.is_empty() {
        return Err(anyhow!(
            "rat_gen.toml has no packets; foxglove mode requires generated declarations"
        ));
    }

    let addr = args.addr.unwrap_or_else(|| cfg.rttd.server.addr.clone());
    let reconnect = parse_duration(args.reconnect.as_deref(), &cfg.rttd.server.reconnect)?;
    let buf = args.buf.unwrap_or(cfg.rttd.server.buf);

    let backend_cfg = merge_backend_config(&cfg.rttd.server.backend, &args.backend, &addr)?;

    info!(
        mode = "foxglove",
        config = %config_path,
        addr = %addr,
        packets = cfg.packets.len(),
        backend = ?backend_cfg.backend_type,
        auto_start_backend = backend_cfg.auto_start,
        "starting foxglove runtime"
    );

    let mut protocol = ProtocolContext::new();
    register_dynamic_packets(&mut protocol, &cfg.packets)?;
    let protocol = Arc::new(protocol);

    let shutdown = CancellationToken::new();
    let hub = Hub::new(buf.max(1));

    let bridge_cfg = BridgeConfig {
        ws_addr: args
            .ws_addr
            .unwrap_or_else(|| cfg.rttd.foxglove.ws_addr.clone()),
    };

    let bridge_task = tokio::spawn(run_bridge(
        bridge_cfg,
        cfg.packets.clone(),
        hub.clone(),
        shutdown.clone(),
    ));

    let mut backend_runtime = BackendRuntime::start(&backend_cfg, &addr).await?;
    let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>(buf.max(1));
    let listener_task = spawn_listener(
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
    let consume_task = spawn_frame_consumer(
        frame_rx,
        hub.clone(),
        protocol.clone(),
        shutdown.clone(),
    );

    tokio::signal::ctrl_c()
        .await
        .context("failed to wait ctrl-c")?;
    info!("received ctrl-c, shutting down foxglove runtime");
    shutdown.cancel();

    listener_task.abort();
    let _ = listener_task.await;
    let _ = consume_task.await;

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

    #[tokio::test]
    async fn run_foxglove_rejects_empty_generated_packets() {
        let dir = unique_temp_dir("rttd_foxglove_empty_gen");
        let config_path = dir.join("rat.toml");
        let generated_path = dir.join("rat_gen.toml");

        let config = r#"
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
"#;
        fs::write(&config_path, config).expect("write config");
        fs::write(&generated_path, "packets = []\n\n[meta]\nproject = \"demo\"\nfingerprint = \"0x1\"\n")
            .expect("write generated");

        let args = FoxgloveArgs {
            config: Some(config_path.to_string_lossy().to_string()),
            addr: None,
            ws_addr: None,
            reconnect: None,
            buf: None,
            backend: BackendArgs::default(),
        };

        let err = run_foxglove(args).await.expect_err("empty packets should fail");
        assert!(err
            .to_string()
            .contains("rat_gen.toml has no packets; foxglove mode requires generated declarations"));

        let _ = fs::remove_file(config_path);
        let _ = fs::remove_file(generated_path);
        let _ = fs::remove_dir_all(dir);
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
