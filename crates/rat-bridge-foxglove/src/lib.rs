use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context as AnyhowContext, Result};
use base64::Engine;
use foxglove::{Context, PartialMetadata, RawChannel, Schema, WebSocketServer};
use rat_engine::Hub;
use rat_protocol::{PacketData, QuatPacket, RatPacket};
use serde::Serialize;
use serde_json::{json, Value};
use time::OffsetDateTime;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_TEMP_TOPIC: &str = "/ratitude/temperature";
pub const DEFAULT_TEMP_UNIT: &str = "C";

pub const DEFAULT_PACKET_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "id": { "type": "string" },
    "ts": { "type": "string" },
    "payload_hex": { "type": "string" },
    "data": { "type": "object", "additionalProperties": true },
    "text": { "type": "string" }
  },
  "required": ["id", "payload_hex"]
}"#;

pub const DEFAULT_MARKER_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "header": {
      "type": "object",
      "properties": {
        "frame_id": { "type": "string" },
        "stamp": {
          "type": "object",
          "properties": {
            "sec": { "type": "integer" },
            "nsec": { "type": "integer" }
          },
          "required": ["sec", "nsec"]
        }
      },
      "required": ["frame_id", "stamp"]
    },
    "ns": { "type": "string" },
    "id": { "type": "integer" },
    "type": { "type": "integer" },
    "action": { "type": "integer" },
    "pose": {
      "type": "object",
      "properties": {
        "position": {
          "type": "object",
          "properties": {
            "x": { "type": "number" },
            "y": { "type": "number" },
            "z": { "type": "number" }
          },
          "required": ["x", "y", "z"]
        },
        "orientation": {
          "type": "object",
          "properties": {
            "x": { "type": "number" },
            "y": { "type": "number" },
            "z": { "type": "number" },
            "w": { "type": "number" }
          },
          "required": ["x", "y", "z", "w"]
        }
      },
      "required": ["position", "orientation"]
    },
    "scale": {
      "type": "object",
      "properties": {
        "x": { "type": "number" },
        "y": { "type": "number" },
        "z": { "type": "number" }
      },
      "required": ["x", "y", "z"]
    },
    "color": {
      "type": "object",
      "properties": {
        "r": { "type": "number" },
        "g": { "type": "number" },
        "b": { "type": "number" },
        "a": { "type": "number" }
      },
      "required": ["r", "g", "b", "a"]
    }
  },
  "required": ["header", "ns", "id", "type", "action", "pose", "scale", "color"]
}"#;

pub const DEFAULT_TRANSFORM_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "transforms": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "timestamp": {
            "type": "object",
            "properties": {
              "sec": { "type": "integer" },
              "nsec": { "type": "integer" }
            },
            "required": ["sec", "nsec"]
          },
          "parent_frame_id": { "type": "string" },
          "child_frame_id": { "type": "string" },
          "translation": {
            "type": "object",
            "properties": {
              "x": { "type": "number" },
              "y": { "type": "number" },
              "z": { "type": "number" }
            },
            "required": ["x", "y", "z"]
          },
          "rotation": {
            "type": "object",
            "properties": {
              "x": { "type": "number" },
              "y": { "type": "number" },
              "z": { "type": "number" },
              "w": { "type": "number" }
            },
            "required": ["x", "y", "z", "w"]
          }
        },
        "required": ["timestamp", "parent_frame_id", "child_frame_id", "translation", "rotation"]
      }
    }
  },
  "required": ["transforms"]
}"#;

pub const DEFAULT_IMAGE_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "timestamp": {
      "type": "object",
      "properties": {
        "sec": { "type": "integer" },
        "nsec": { "type": "integer" }
      },
      "required": ["sec", "nsec"]
    },
    "frame_id": { "type": "string" },
    "format": { "type": "string" },
    "data": { "type": "string", "contentEncoding": "base64" }
  },
  "required": ["timestamp", "frame_id", "format", "data"]
}"#;

pub const DEFAULT_LOG_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "timestamp": {
      "type": "object",
      "properties": {
        "sec": { "type": "integer" },
        "nsec": { "type": "integer" }
      },
      "required": ["sec", "nsec"]
    },
    "level": { "type": "integer" },
    "message": { "type": "string" },
    "name": { "type": "string" },
    "file": { "type": "string" },
    "line": { "type": "integer" }
  },
  "required": ["timestamp", "level", "message", "name", "file", "line"]
}"#;

pub const DEFAULT_TEMPERATURE_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "timestamp": {
      "type": "object",
      "properties": {
        "sec": { "type": "integer" },
        "nsec": { "type": "integer" }
      },
      "required": ["sec", "nsec"]
    },
    "value": { "type": "number" },
    "unit": { "type": "string" }
  },
  "required": ["timestamp", "value", "unit"]
}"#;

#[derive(Clone, Debug)]
pub struct BridgeConfig {
    pub ws_addr: String,
    pub topic: String,
    pub schema_name: String,
    pub marker_topic: String,
    pub parent_frame_id: String,
    pub frame_id: String,
    pub image_path: String,
    pub image_frame_id: String,
    pub image_format: String,
    pub log_topic: String,
    pub log_name: String,
    pub temp_topic: String,
    pub temp_unit: String,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            ws_addr: "127.0.0.1:8765".to_string(),
            topic: "ratitude/packet".to_string(),
            schema_name: "ratitude.Packet".to_string(),
            marker_topic: "/visualization_marker".to_string(),
            parent_frame_id: "world".to_string(),
            frame_id: "base_link".to_string(),
            image_path: "D:/Repos/ratitude/demo.jpg".to_string(),
            image_frame_id: "camera".to_string(),
            image_format: "jpeg".to_string(),
            log_topic: "/ratitude/log".to_string(),
            log_name: "ratitude".to_string(),
            temp_topic: DEFAULT_TEMP_TOPIC.to_string(),
            temp_unit: DEFAULT_TEMP_UNIT.to_string(),
        }
    }
}

#[derive(Clone)]
struct Channels {
    packet: Arc<RawChannel>,
    marker: Arc<RawChannel>,
    transform: Arc<RawChannel>,
    image: Option<Arc<RawChannel>>,
    log: Arc<RawChannel>,
    temp: Arc<RawChannel>,
}

#[derive(Serialize)]
struct FoxglovePacket {
    id: String,
    ts: String,
    payload_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

pub async fn run_bridge(
    cfg: BridgeConfig,
    hub: Hub,
    text_id: u8,
    quat_id: u8,
    shutdown: CancellationToken,
) -> Result<()> {
    let context = Context::new();
    let (host, port) = split_host_port(&cfg.ws_addr)?;

    let channels = Channels {
        packet: build_raw_channel(
            &context,
            &cfg.topic,
            &cfg.schema_name,
            DEFAULT_PACKET_SCHEMA,
        )?,
        marker: build_raw_channel(
            &context,
            &cfg.marker_topic,
            "visualization_msgs/Marker",
            DEFAULT_MARKER_SCHEMA,
        )?,
        transform: build_raw_channel(
            &context,
            "/tf",
            "foxglove.FrameTransforms",
            DEFAULT_TRANSFORM_SCHEMA,
        )?,
        image: build_image_channel(&context, &cfg)?,
        log: build_raw_channel(&context, &cfg.log_topic, "foxglove.Log", DEFAULT_LOG_SCHEMA)?,
        temp: build_raw_channel(
            &context,
            &cfg.temp_topic,
            "ratitude.Temperature",
            DEFAULT_TEMPERATURE_SCHEMA,
        )?,
    };

    let server = WebSocketServer::new()
        .name("ratitude")
        .bind(host, port)
        .context(&context)
        .start()
        .await
        .with_context(|| format!("failed to start foxglove websocket at {}", cfg.ws_addr))?;

    let packet_task = spawn_packet_publish_task(
        hub.subscribe(),
        channels.clone(),
        cfg.clone(),
        text_id,
        quat_id,
        shutdown.clone(),
    );

    let image_task =
        spawn_image_publish_task(channels.image.clone(), cfg.clone(), shutdown.clone());

    shutdown.cancelled().await;

    packet_task.abort();
    if let Some(task) = image_task {
        task.abort();
    }

    server.stop().wait().await;
    Ok(())
}

fn split_host_port(raw: &str) -> Result<(String, u16)> {
    let (host, port) = raw
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("invalid ws address: {}", raw))?;
    let port = port
        .parse::<u16>()
        .with_context(|| format!("invalid ws port in {}", raw))?;
    Ok((host.to_string(), port))
}

fn build_raw_channel(
    context: &Arc<Context>,
    topic: &str,
    schema_name: &str,
    schema_json: &str,
) -> Result<Arc<RawChannel>> {
    context
        .channel_builder(topic)
        .message_encoding("json")
        .schema(Some(Schema::new(
            schema_name,
            "jsonschema",
            schema_json.as_bytes().to_vec(),
        )))
        .build_raw()
        .map_err(|err| anyhow!(err.to_string()))
}

fn build_image_channel(
    context: &Arc<Context>,
    cfg: &BridgeConfig,
) -> Result<Option<Arc<RawChannel>>> {
    if cfg.image_path.trim().is_empty() || !Path::new(&cfg.image_path).exists() {
        return Ok(None);
    }
    let channel = build_raw_channel(
        context,
        "/camera/image/compressed",
        "foxglove.CompressedImage",
        DEFAULT_IMAGE_SCHEMA,
    )?;
    Ok(Some(channel))
}

fn spawn_packet_publish_task(
    mut receiver: tokio::sync::broadcast::Receiver<RatPacket>,
    channels: Channels,
    cfg: BridgeConfig,
    text_id: u8,
    quat_id: u8,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                packet = receiver.recv() => {
                    let packet = match packet {
                        Ok(value) => value,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    };
                    publish_packet(&channels, &cfg, &packet, text_id, quat_id);
                }
            }
        }
    })
}

fn spawn_image_publish_task(
    image_channel: Option<Arc<RawChannel>>,
    cfg: BridgeConfig,
    shutdown: CancellationToken,
) -> Option<JoinHandle<()>> {
    let channel = image_channel?;
    let bytes = std::fs::read(&cfg.image_path).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);

    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {
                    let now = SystemTime::now();
                    let frame = json!({
                        "timestamp": frame_time(now),
                        "frame_id": cfg.image_frame_id,
                        "format": cfg.image_format,
                        "data": encoded,
                    });
                    publish_json(&channel, now, &frame);
                }
            }
        }
    }))
}

fn publish_packet(
    channels: &Channels,
    cfg: &BridgeConfig,
    packet: &RatPacket,
    text_id: u8,
    quat_id: u8,
) {
    let ts = packet.timestamp;
    let packet_data = packet_data_value(&packet.data);
    let text = match &packet.data {
        PacketData::Text(value) => Some(value.clone()),
        _ => None,
    };

    let record = FoxglovePacket {
        id: format!("0x{:02x}", packet.id),
        ts: format_timestamp(ts),
        payload_hex: hex::encode(&packet.payload),
        data: if packet.id == text_id {
            None
        } else {
            packet_data
        },
        text,
    };
    publish_json(&channels.packet, ts, &record);

    if packet.id == text_id {
        let text = match &packet.data {
            PacketData::Text(value) => value.clone(),
            _ => String::new(),
        };
        let log = json!({
            "timestamp": frame_time(ts),
            "level": 2,
            "message": text,
            "name": cfg.log_name,
            "file": "",
            "line": 0,
        });
        publish_json(&channels.log, ts, &log);
    }

    if let PacketData::Temperature(temp) = &packet.data {
        let value = json!({
            "timestamp": frame_time(ts),
            "value": temp.celsius,
            "unit": cfg.temp_unit,
        });
        publish_json(&channels.temp, ts, &value);
    }

    if packet.id == quat_id {
        if let Some(quat) = extract_quaternion(packet) {
            let marker = json!({
                "header": {
                    "frame_id": cfg.frame_id,
                    "stamp": marker_stamp(ts),
                },
                "ns": "ratitude",
                "id": 0,
                "type": 1,
                "action": 0,
                "pose": {
                    "position": {"x": 0.0, "y": 0.0, "z": 0.0},
                    "orientation": {
                        "x": quat.x,
                        "y": quat.y,
                        "z": quat.z,
                        "w": quat.w,
                    }
                },
                "scale": {"x": 0.3, "y": 0.1, "z": 0.05},
                "color": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0}
            });
            publish_json(&channels.marker, ts, &marker);

            let transform = json!({
                "transforms": [{
                    "timestamp": frame_time(ts),
                    "parent_frame_id": cfg.parent_frame_id,
                    "child_frame_id": cfg.frame_id,
                    "translation": {"x": 0.0, "y": 0.0, "z": 0.0},
                    "rotation": {
                        "x": quat.x,
                        "y": quat.y,
                        "z": quat.z,
                        "w": quat.w,
                    }
                }]
            });
            publish_json(&channels.transform, ts, &transform);
        }
    }
}

fn publish_json(channel: &Arc<RawChannel>, ts: SystemTime, value: &impl Serialize) {
    let Ok(payload) = serde_json::to_vec(value) else {
        return;
    };
    channel.log_with_meta(
        &payload,
        PartialMetadata::with_log_time(system_time_to_nanos(ts)),
    );
}

fn packet_data_value(data: &PacketData) -> Option<Value> {
    match data {
        PacketData::Text(text) => Some(Value::String(text.clone())),
        PacketData::Dynamic(map) => Some(Value::Object(map.clone())),
        PacketData::Quat(value) => serde_json::to_value(value).ok(),
        PacketData::Temperature(value) => serde_json::to_value(value).ok(),
        PacketData::Raw(value) => serde_json::to_value(value).ok(),
    }
}

fn extract_quaternion(packet: &RatPacket) -> Option<QuatPacket> {
    match &packet.data {
        PacketData::Quat(quat) => Some(quat.clone()),
        PacketData::Dynamic(map) => {
            let x = number_to_f32(map.get("x"));
            let y = number_to_f32(map.get("y"));
            let z = number_to_f32(map.get("z"));
            let w = number_to_f32(map.get("w"));
            if let (Some(x), Some(y), Some(z), Some(w)) = (x, y, z, w) {
                return Some(QuatPacket { x, y, z, w });
            }

            let qx = number_to_f32(map.get("q_x"));
            let qy = number_to_f32(map.get("q_y"));
            let qz = number_to_f32(map.get("q_z"));
            let qw = number_to_f32(map.get("q_w"));
            if let (Some(x), Some(y), Some(z), Some(w)) = (qx, qy, qz, qw) {
                return Some(QuatPacket { x, y, z, w });
            }
            None
        }
        _ => {
            if packet.payload.len() < 16 {
                return None;
            }
            Some(QuatPacket {
                w: f32::from_bits(u32::from_le_bytes(packet.payload[0..4].try_into().ok()?)),
                x: f32::from_bits(u32::from_le_bytes(packet.payload[4..8].try_into().ok()?)),
                y: f32::from_bits(u32::from_le_bytes(packet.payload[8..12].try_into().ok()?)),
                z: f32::from_bits(u32::from_le_bytes(packet.payload[12..16].try_into().ok()?)),
            })
        }
    }
}

fn number_to_f32(value: Option<&Value>) -> Option<f32> {
    match value {
        Some(Value::Number(number)) => number.as_f64().map(|v| v as f32),
        Some(Value::Bool(flag)) => Some(if *flag { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn system_time_to_nanos(ts: SystemTime) -> u64 {
    ts.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_nanos() as u64
}

fn frame_time(ts: SystemTime) -> Value {
    let duration = ts
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    json!({
        "sec": duration.as_secs(),
        "nsec": duration.subsec_nanos(),
    })
}

fn marker_stamp(ts: SystemTime) -> Value {
    let duration = ts
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    json!({
        "sec": duration.as_secs() as i64,
        "nsec": duration.subsec_nanos() as i64,
    })
}

fn format_timestamp(ts: SystemTime) -> String {
    let duration = ts
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    OffsetDateTime::from_unix_timestamp_nanos(duration.as_nanos() as i128)
        .ok()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}
