use std::env;
use std::fs::File;
use std::io::{self, Write};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand};
use rat_bridge_foxglove::{run_bridge, BridgeConfig};
use rat_config::{load_or_default, FieldDef, PacketDef, RatitudeConfig, DEFAULT_CONFIG_PATH};
use rat_engine::Hub;
use rat_logger::spawn_jsonl_writer;
use rat_protocol::{
    clear_dynamic_registry, clear_static_registry, cobs_decode, parse_packet, register_dynamic,
    register_static_quat, register_static_temperature, set_text_packet_id, DynamicFieldDef,
    DynamicPacketDef, RatPacket,
};
use rat_sync::sync_packets;
use rat_transport::{spawn_listener, ListenerOptions};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

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
    let exit = run().await;
    if let Err(err) = exit {
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
            })
            .await
        }
    }
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
            let _ = sync_packets(&config_path, None)?;
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

async fn run_server(args: ServerArgs) -> Result<()> {
    let config_path = args
        .config
        .unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());
    let (cfg, _) = load_or_default(&config_path)?;

    let addr = args.addr.unwrap_or_else(|| cfg.rttd.server.addr.clone());
    let text_id = parse_u8_id(args.text_id.as_deref(), cfg.rttd.text_id)?;
    let reconnect = parse_duration(args.reconnect.as_deref(), &cfg.rttd.server.reconnect)?;
    let buf = args.buf.unwrap_or(cfg.rttd.server.buf);
    let _reader_buf = args.reader_buf.unwrap_or(cfg.rttd.server.reader_buf);

    set_text_packet_id(text_id);
    clear_static_registry();
    register_dynamic_packets(&cfg.packets)?;

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
        },
    );

    let consume_task = spawn_frame_consumer(frame_rx, hub.clone(), shutdown.clone());

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
    shutdown.cancel();

    listener.abort();
    consume_task.abort();
    log_task.abort();
    Ok(())
}

async fn run_foxglove(args: FoxgloveArgs) -> Result<()> {
    let config_path = args
        .config
        .unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());
    let (cfg, _) = load_or_default(&config_path)?;

    let addr = args.addr.unwrap_or_else(|| cfg.rttd.server.addr.clone());
    let text_id = parse_u8_id(args.text_id.as_deref(), cfg.rttd.text_id)?;
    let reconnect = parse_duration(args.reconnect.as_deref(), &cfg.rttd.server.reconnect)?;
    let buf = args.buf.unwrap_or(cfg.rttd.server.buf);

    let resolved_quat_default = choose_default_quat_id(&cfg);
    let quat_id = parse_u8_id(args.quat_id.as_deref(), resolved_quat_default as u16)?;
    let temp_id = parse_u8_id(args.temp_id.as_deref(), cfg.rttd.foxglove.temp_id)?;

    set_text_packet_id(text_id);
    clear_static_registry();
    register_static_quat(quat_id);
    register_static_temperature(temp_id);
    register_dynamic_packets(&cfg.packets)?;

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
        image_path: args
            .image_path
            .unwrap_or_else(|| cfg.rttd.foxglove.image_path.clone()),
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
            },
        ));
        consume_task = Some(spawn_frame_consumer(
            frame_rx,
            hub.clone(),
            shutdown.clone(),
        ));
    }

    tokio::signal::ctrl_c()
        .await
        .context("failed to wait ctrl-c")?;
    shutdown.cancel();

    if let Some(task) = listener_task {
        task.abort();
    }
    if let Some(task) = consume_task {
        task.abort();
    }
    if let Some(task) = mock_task {
        task.abort();
    }

    match bridge_task.await {
        Ok(result) => result,
        Err(err) => Err(anyhow!("foxglove task failed: {err}")),
    }
}

fn register_dynamic_packets(packets: &[PacketDef]) -> Result<()> {
    clear_dynamic_registry();
    for packet in packets {
        if packet.id > 0xFF {
            return Err(anyhow!("packet id out of range: 0x{:X}", packet.id));
        }

        let fields = packet
            .fields
            .iter()
            .map(map_field)
            .collect::<Vec<DynamicFieldDef>>();

        register_dynamic(
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

fn spawn_frame_consumer(
    mut receiver: mpsc::Receiver<Vec<u8>>,
    hub: Hub,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                maybe_frame = receiver.recv() => {
                    let Some(frame) = maybe_frame else { break; };
                    let Ok(decoded) = cobs_decode(&frame) else { continue; };
                    if decoded.is_empty() {
                        continue;
                    }
                    let id = decoded[0];
                    let payload = decoded[1..].to_vec();
                    let Ok(data) = parse_packet(id, &payload) else { continue; };
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
