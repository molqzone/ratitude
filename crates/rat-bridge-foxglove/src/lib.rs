mod binding;
mod channels;
mod publisher;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context as AnyhowContext, Result};
use foxglove::{Context, WebSocketServer};
use rat_config::PacketDef;
use rat_core::Hub;
use tokio_util::sync::CancellationToken;

use crate::binding::{build_packet_bindings, log_derived_image_channels};
use crate::channels::build_packet_channels;
use crate::publisher::spawn_packet_publish_task;

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

pub async fn run_bridge(
    cfg: BridgeConfig,
    packets: Vec<PacketDef>,
    hub: Hub,
    shutdown: CancellationToken,
) -> Result<()> {
    if packets.is_empty() {
        return Err(anyhow!("no packets available in runtime schema"));
    }

    let context = Context::new();
    let (host, port) = split_host_port(&cfg.ws_addr)?;

    let bindings = build_packet_bindings(&packets)?;
    log_derived_image_channels(&bindings);
    let channels = build_packet_channels(&context, &bindings)?;

    let channel_map = Arc::new(
        channels
            .iter()
            .map(|item| (item.binding.id, item.clone()))
            .collect::<HashMap<u8, _>>(),
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
    let normalized = raw.trim();
    if let Some(rest) = normalized.strip_prefix('[') {
        let (host, suffix) = rest
            .split_once(']')
            .ok_or_else(|| anyhow!("invalid ws address: {}", raw))?;
        if host.is_empty() {
            return Err(anyhow!("invalid ws address: {}", raw));
        }
        let port = parse_ws_port(
            suffix
                .strip_prefix(':')
                .ok_or_else(|| anyhow!("invalid ws address: {}", raw))?,
            raw,
        )?;
        return Ok((host.to_string(), port));
    }

    let (host, port_raw) = normalized
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("invalid ws address: {}", raw))?;
    if host.is_empty() || host.contains(':') {
        return Err(anyhow!("invalid ws address: {}", raw));
    }
    let port = parse_ws_port(port_raw, raw)?;
    Ok((host.to_string(), port))
}

fn parse_ws_port(raw_port: &str, raw_addr: &str) -> Result<u16> {
    raw_port
        .parse::<u16>()
        .with_context(|| format!("invalid ws port in {}", raw_addr))
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use rat_config::{FieldDef, PacketType};
    use rat_protocol::{PacketData, RatPacket};
    use serde_json::{json, Value};

    use crate::binding::{build_packet_bindings, packet_schema_json};
    use crate::publisher::{build_image_message, extract_quaternion, packet_data_to_json};

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
    fn split_host_port_supports_hostname_and_ipv4() {
        let (host, port) = split_host_port("localhost:8765").expect("parse host:port");
        assert_eq!(host, "localhost");
        assert_eq!(port, 8765);
    }

    #[test]
    fn split_host_port_supports_bracketed_ipv6() {
        let (host, port) = split_host_port("[::1]:8765").expect("parse ipv6");
        assert_eq!(host, "::1");
        assert_eq!(port, 8765);
    }

    #[test]
    fn split_host_port_rejects_unbracketed_ipv6() {
        let err = split_host_port("::1:8765").expect_err("ipv6 must be bracketed");
        assert!(err.to_string().contains("invalid ws address"));
    }

    #[test]
    fn binding_uses_struct_name_topic_and_suffix_for_duplicates() {
        let packets = vec![
            PacketDef {
                id: 0x10,
                struct_name: "Attitude".to_string(),
                packet_type: PacketType::Quat,
                packed: true,
                byte_size: 16,
                source: String::new(),
                fields: sample_fields(),
            },
            PacketDef {
                id: 0x11,
                struct_name: "Attitude".to_string(),
                packet_type: PacketType::Plot,
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
            packet_type: PacketType::Image,
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
