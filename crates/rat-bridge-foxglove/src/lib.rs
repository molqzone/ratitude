use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context as AnyhowContext, Result};
use foxglove::schemas::{RawImage, Timestamp};
use foxglove::{Channel, Context, PartialMetadata, RawChannel, Schema, WebSocketServer};
use rat_config::{FieldDef, PacketDef};
use rat_core::Hub;
use rat_protocol::{PacketData, RatPacket};
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

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

#[derive(Clone, Debug)]
pub struct BridgeConfig {
    pub ws_addr: String,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            ws_addr: "127.0.0.1:8765".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
struct PacketBinding {
    id: u8,
    struct_name: String,
    packet_type: String,
    topic: String,
    schema_name: String,
    schema_json: String,
    tf_topic: Option<String>,
    marker_topic: Option<String>,
    image_topic: Option<String>,
}

#[derive(Clone)]
struct PacketChannels {
    data: Arc<RawChannel>,
    marker: Option<Arc<RawChannel>>,
    tf: Option<Arc<RawChannel>>,
    image: Option<Arc<Channel<RawImage>>>,
    binding: PacketBinding,
}

#[derive(Clone, Copy, Debug)]
struct Quaternion {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

pub async fn run_bridge(
    cfg: BridgeConfig,
    packets: Vec<PacketDef>,
    hub: Hub,
    shutdown: CancellationToken,
) -> Result<()> {
    if packets.is_empty() {
        return Err(anyhow!("no packets found in rat_gen.toml"));
    }

    let context = Context::new();
    let (host, port) = split_host_port(&cfg.ws_addr)?;

    let bindings = build_packet_bindings(&packets)?;
    log_derived_image_channels(&bindings);
    let channels = build_packet_channels(&context, &bindings)?;

    let channel_map: Arc<HashMap<u8, PacketChannels>> = Arc::new(
        channels
            .iter()
            .map(|item| (item.binding.id, item.clone()))
            .collect(),
    );

    let server = WebSocketServer::new()
        .name("ratitude")
        .bind(host, port)
        .context(&context)
        .start()
        .await
        .with_context(|| format!("failed to start foxglove websocket at {}", cfg.ws_addr))?;

    let packet_task = spawn_packet_publish_task(hub.subscribe(), channel_map, shutdown.clone());

    shutdown.cancelled().await;

    packet_task.abort();
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

fn sanitize_topic_segment(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn c_type_to_json_type(c_type: &str) -> Option<&'static str> {
    match c_type.trim().to_ascii_lowercase().as_str() {
        "float" | "double" => Some("number"),
        "bool" | "_bool" => Some("boolean"),
        "int8_t" | "uint8_t" | "int16_t" | "uint16_t" | "int32_t" | "uint32_t" | "int64_t"
        | "uint64_t" => Some("integer"),
        _ => None,
    }
}

fn packet_schema_json(fields: &[FieldDef]) -> Result<String> {
    if fields.is_empty() {
        return Err(anyhow!("packet schema requires at least one field"));
    }

    let mut properties = serde_json::Map::new();
    let mut required = Vec::with_capacity(fields.len());
    for field in fields {
        let json_type = c_type_to_json_type(&field.c_type)
            .ok_or_else(|| anyhow!("unsupported c type for foxglove schema: {}", field.c_type))?;
        properties.insert(field.name.clone(), json!({ "type": json_type }));
        required.push(field.name.clone());
    }

    serde_json::to_string(&json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    }))
    .map_err(|err| anyhow!(err.to_string()))
}

fn build_packet_bindings(packets: &[PacketDef]) -> Result<Vec<PacketBinding>> {
    let mut topic_count: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = Vec::with_capacity(packets.len());

    for packet in packets {
        if packet.id > 0xFF {
            return Err(anyhow!("packet id out of range: 0x{:X}", packet.id));
        }

        let base = sanitize_topic_segment(&packet.struct_name);
        let mut topic = format!("/rat/{}", base);
        let counter = topic_count.entry(topic.clone()).or_insert(0);
        if *counter > 0 {
            topic = format!("{}_0x{:02X}", topic, packet.id);
        }
        *counter += 1;

        let schema_name = format!("ratitude.{}", base);
        let is_quat = packet.packet_type.eq_ignore_ascii_case("quat");
        let is_image = packet.packet_type.eq_ignore_ascii_case("image");

        out.push(PacketBinding {
            id: packet.id as u8,
            struct_name: packet.struct_name.clone(),
            packet_type: packet.packet_type.clone(),
            topic: topic.clone(),
            schema_name,
            schema_json: packet_schema_json(&packet.fields)?,
            marker_topic: if is_quat {
                Some(format!("{}/marker", topic))
            } else {
                None
            },
            tf_topic: if is_quat {
                Some(format!("{}/tf", topic))
            } else {
                None
            },
            image_topic: if is_image {
                Some(format!("{}/image", topic))
            } else {
                None
            },
        });
    }

    Ok(out)
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

fn build_packet_channels(
    context: &Arc<Context>,
    bindings: &[PacketBinding],
) -> Result<Vec<PacketChannels>> {
    let mut out = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let data = build_raw_channel(
            context,
            &binding.topic,
            &binding.schema_name,
            &binding.schema_json,
        )?;

        let marker = match &binding.marker_topic {
            Some(topic) => Some(build_raw_channel(
                context,
                topic,
                "visualization_msgs/Marker",
                DEFAULT_MARKER_SCHEMA,
            )?),
            None => None,
        };

        let tf = match &binding.tf_topic {
            Some(topic) => Some(build_raw_channel(
                context,
                topic,
                "foxglove.FrameTransforms",
                DEFAULT_TRANSFORM_SCHEMA,
            )?),
            None => None,
        };

        let image = binding
            .image_topic
            .as_ref()
            .map(|topic| Arc::new(context.channel_builder(topic).build::<RawImage>()));

        out.push(PacketChannels {
            data,
            marker,
            tf,
            image,
            binding: binding.clone(),
        });
    }
    Ok(out)
}

fn log_derived_image_channels(bindings: &[PacketBinding]) {
    let derived_topics = bindings
        .iter()
        .filter_map(|binding| binding.image_topic.clone())
        .collect::<Vec<String>>();
    if derived_topics.is_empty() {
        return;
    }

    info!(
        derived_image_channels = derived_topics.len(),
        topics = ?derived_topics,
        "image channels publish derived mono8 frames from dynamic fields (width/height/frame_idx/luma), not raw payload image bytes"
    );
}

fn spawn_packet_publish_task(
    mut receiver: tokio::sync::broadcast::Receiver<RatPacket>,
    channels: Arc<HashMap<u8, PacketChannels>>,
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
                    publish_packet(&channels, &packet);
                }
            }
        }
    })
}

fn publish_packet(channels: &HashMap<u8, PacketChannels>, packet: &RatPacket) {
    let Some(channels) = channels.get(&packet.id) else {
        return;
    };

    if let Some(value) = packet_data_to_json(&packet.data) {
        publish_json(&channels.data, packet.timestamp, &value);
    }

    if channels.binding.packet_type.eq_ignore_ascii_case("quat") {
        if let Some(quat) = extract_quaternion(packet) {
            if let Some(marker_channel) = &channels.marker {
                let marker = json!({
                    "header": {
                        "frame_id": "world",
                        "stamp": marker_stamp(packet.timestamp),
                    },
                    "ns": channels.binding.struct_name,
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
                    "scale": {"x": 0.1, "y": 0.1, "z": 0.1},
                    "color": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0}
                });
                publish_json(marker_channel, packet.timestamp, &marker);
            }

            if let Some(tf_channel) = &channels.tf {
                let transform = json!({
                    "transforms": [{
                        "timestamp": frame_time(packet.timestamp),
                        "parent_frame_id": "world",
                        "child_frame_id": channels.binding.struct_name,
                        "translation": {"x": 0.0, "y": 0.0, "z": 0.0},
                        "rotation": {
                            "x": quat.x,
                            "y": quat.y,
                            "z": quat.z,
                            "w": quat.w,
                        }
                    }]
                });
                publish_json(tf_channel, packet.timestamp, &transform);
            }
        }
    }

    if channels.binding.packet_type.eq_ignore_ascii_case("image") {
        if let Some(image_channel) = &channels.image {
            if let Some(image_msg) = build_image_message(packet, &channels.binding.struct_name) {
                image_channel.log(&image_msg);
            }
        }
    }
}

fn publish_json(channel: &Arc<RawChannel>, ts: SystemTime, value: &Value) {
    let Ok(payload) = serde_json::to_vec(value) else {
        return;
    };
    channel.log_with_meta(
        &payload,
        PartialMetadata::with_log_time(system_time_to_nanos(ts)),
    );
}

fn packet_data_to_json(data: &PacketData) -> Option<Value> {
    match data {
        PacketData::Dynamic(map) => Some(Value::Object(map.clone())),
        PacketData::Text(_) => None,
    }
}

fn build_image_message(packet: &RatPacket, frame_id: &str) -> Option<RawImage> {
    let PacketData::Dynamic(map) = &packet.data else {
        return None;
    };

    let width = number_to_u32(map.get("width"))
        .unwrap_or(320)
        .clamp(1, 1024);
    let height = number_to_u32(map.get("height"))
        .unwrap_or(240)
        .clamp(1, 1024);
    let frame_idx = number_to_u32(map.get("frame_idx"))
        .or_else(|| number_to_u32(map.get("frame")))
        .unwrap_or(0);
    let luma = number_to_u8(map.get("luma"))
        .or_else(|| number_to_u8(map.get("gray")))
        .unwrap_or(128);

    let step = width;
    let size = width.checked_mul(height)? as usize;
    let mut data = vec![0_u8; size];
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let gradient = ((x + frame_idx) & 0xFF) as u8;
            let wave = ((y.wrapping_add(frame_idx / 2)) & 0xFF) as u8;
            data[index] = gradient.wrapping_add(wave).wrapping_add(luma / 2);
        }
    }

    Some(RawImage {
        timestamp: to_fox_timestamp(packet.timestamp),
        frame_id: frame_id.to_string(),
        width,
        height,
        encoding: "mono8".to_string(),
        step,
        data: data.into(),
    })
}

fn to_fox_timestamp(ts: SystemTime) -> Option<Timestamp> {
    Timestamp::try_from(ts).ok()
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    match value {
        Some(Value::Number(number)) => number.as_u64().and_then(|v| u32::try_from(v).ok()),
        Some(Value::Bool(flag)) => Some(if *flag { 1 } else { 0 }),
        _ => None,
    }
}

fn number_to_u8(value: Option<&Value>) -> Option<u8> {
    number_to_u32(value).and_then(|v| u8::try_from(v).ok())
}

fn extract_quaternion(packet: &RatPacket) -> Option<Quaternion> {
    let PacketData::Dynamic(map) = &packet.data else {
        return None;
    };

    let x = number_to_f32(map.get("x"));
    let y = number_to_f32(map.get("y"));
    let z = number_to_f32(map.get("z"));
    let w = number_to_f32(map.get("w"));
    if let (Some(x), Some(y), Some(z), Some(w)) = (x, y, z, w) {
        return Some(Quaternion { x, y, z, w });
    }

    let qx = number_to_f32(map.get("q_x"));
    let qy = number_to_f32(map.get("q_y"));
    let qz = number_to_f32(map.get("q_z"));
    let qw = number_to_f32(map.get("q_w"));
    if let (Some(x), Some(y), Some(z), Some(w)) = (qx, qy, qz, qw) {
        return Some(Quaternion { x, y, z, w });
    }

    None
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fields() -> Vec<FieldDef> {
        vec![
            FieldDef {
                name: "w".to_string(),
                c_type: "float".to_string(),
                offset: 0,
                size: 4,
            },
            FieldDef {
                name: "x".to_string(),
                c_type: "float".to_string(),
                offset: 4,
                size: 4,
            },
        ]
    }

    #[test]
    fn schema_generation_maps_c_types() {
        let schema = packet_schema_json(&sample_fields()).expect("schema");
        assert!(schema.contains("\"w\":{\"type\":\"number\"}"));
        assert!(schema.contains("\"x\":{\"type\":\"number\"}"));
    }

    #[test]
    fn binding_uses_struct_name_topic_and_suffix_for_duplicates() {
        let packets = vec![
            PacketDef {
                id: 0x10,
                struct_name: "Attitude".to_string(),
                packet_type: "quat".to_string(),
                packed: true,
                byte_size: 16,
                source: String::new(),
                fields: sample_fields(),
            },
            PacketDef {
                id: 0x11,
                struct_name: "Attitude".to_string(),
                packet_type: "plot".to_string(),
                packed: true,
                byte_size: 8,
                source: String::new(),
                fields: sample_fields(),
            },
        ];

        let bindings = build_packet_bindings(&packets).expect("bindings");
        assert_eq!(bindings[0].topic, "/rat/Attitude");
        assert_eq!(bindings[1].topic, "/rat/Attitude_0x11");
        assert_eq!(
            bindings[0].marker_topic.as_deref(),
            Some("/rat/Attitude/marker")
        );
        assert!(bindings[0].image_topic.is_none());
        assert!(bindings[1].marker_topic.is_none());
        assert!(bindings[1].image_topic.is_none());
    }

    #[test]
    fn unknown_c_type_is_rejected() {
        let fields = vec![FieldDef {
            name: "name".to_string(),
            c_type: "char*".to_string(),
            offset: 0,
            size: 8,
        }];
        assert!(packet_schema_json(&fields).is_err());
    }

    #[test]
    fn image_binding_has_derived_image_topic() {
        let packets = vec![PacketDef {
            id: 0x20,
            struct_name: "CameraStats".to_string(),
            packet_type: "image".to_string(),
            packed: true,
            byte_size: 8,
            source: String::new(),
            fields: vec![FieldDef {
                name: "width".to_string(),
                c_type: "uint16_t".to_string(),
                offset: 0,
                size: 2,
            }],
        }];

        let bindings = build_packet_bindings(&packets).expect("bindings");
        assert_eq!(
            bindings[0].image_topic.as_deref(),
            Some("/rat/CameraStats/image")
        );
    }

    #[test]
    fn packet_data_to_json_supports_dynamic() {
        let value = packet_data_to_json(&PacketData::Dynamic(serde_json::Map::from_iter([
            ("x".to_string(), json!(0.1)),
            ("y".to_string(), json!(0.2)),
            ("z".to_string(), json!(0.3)),
            ("w".to_string(), json!(0.9)),
        ])))
        .expect("dynamic value");

        let x = value.get("x").and_then(Value::as_f64).expect("x");
        let w = value.get("w").and_then(Value::as_f64).expect("w");
        assert!((x - 0.1).abs() < 1e-6);
        assert!((w - 0.9).abs() < 1e-6);
    }

    #[test]
    fn build_image_message_generates_mono8_frame() {
        let packet = RatPacket {
            id: 0x30,
            timestamp: SystemTime::UNIX_EPOCH,
            payload: vec![],
            data: PacketData::Dynamic(serde_json::Map::from_iter([
                ("width".to_string(), json!(4)),
                ("height".to_string(), json!(3)),
                ("frame_idx".to_string(), json!(7)),
                ("luma".to_string(), json!(128)),
            ])),
        };

        let image = build_image_message(&packet, "Camera").expect("image message");
        assert_eq!(image.frame_id, "Camera");
        assert_eq!(image.width, 4);
        assert_eq!(image.height, 3);
        assert_eq!(image.step, 4);
        assert_eq!(image.encoding, "mono8");
        assert_eq!(image.data.len(), 12);
    }

    #[test]
    fn extract_quaternion_reads_dynamic_fields() {
        let packet = RatPacket {
            id: 0x10,
            timestamp: SystemTime::UNIX_EPOCH,
            payload: vec![],
            data: PacketData::Dynamic(serde_json::Map::from_iter([
                ("x".to_string(), json!(1.0)),
                ("y".to_string(), json!(2.0)),
                ("z".to_string(), json!(3.0)),
                ("w".to_string(), json!(4.0)),
            ])),
        };

        let quat = extract_quaternion(&packet).expect("quat");
        assert_eq!(quat.x, 1.0);
        assert_eq!(quat.y, 2.0);
        assert_eq!(quat.z, 3.0);
        assert_eq!(quat.w, 4.0);
    }
}
