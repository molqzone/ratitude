use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use foxglove::schemas::{RawImage, Timestamp};
use foxglove::{PartialMetadata, RawChannel};
use rat_config::PacketType;
use rat_protocol::{PacketData, RatPacket};
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::channels::PacketChannels;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Quaternion {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
    pub(crate) w: f32,
}

pub(crate) fn spawn_packet_publish_task(
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

    if channels.binding.packet_type == PacketType::Quat {
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

    if channels.binding.packet_type == PacketType::Image {
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

pub(crate) fn packet_data_to_json(data: &PacketData) -> Option<Value> {
    match data {
        PacketData::Dynamic(map) => Some(Value::Object(map.clone())),
        PacketData::Text(_) => None,
    }
}

pub(crate) fn build_image_message(packet: &RatPacket, frame_id: &str) -> Option<RawImage> {
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

pub(crate) fn extract_quaternion(packet: &RatPacket) -> Option<Quaternion> {
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
