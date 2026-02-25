use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use rat_config::{FieldDef, PacketDef, PacketType};
use serde_json::json;
use tracing::info;

pub(crate) const DEFAULT_MARKER_SCHEMA: &str = r#"{
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

pub(crate) const DEFAULT_TRANSFORM_SCHEMA: &str = r#"{
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
pub(crate) struct PacketBinding {
    pub(crate) id: u8,
    pub(crate) struct_name: String,
    pub(crate) packet_type: PacketType,
    pub(crate) topic: String,
    pub(crate) schema_name: String,
    pub(crate) schema_json: String,
    pub(crate) tf_topic: Option<String>,
    pub(crate) marker_topic: Option<String>,
    pub(crate) image_topic: Option<String>,
}

pub(crate) fn packet_schema_json(fields: &[FieldDef]) -> Result<String> {
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

pub(crate) fn build_packet_bindings(packets: &[PacketDef]) -> Result<Vec<PacketBinding>> {
    let mut topic_count: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = Vec::with_capacity(packets.len());

    for packet in packets {
        if packet.id > 0xFF {
            return Err(anyhow!("packet id out of range: 0x{:X}", packet.id));
        }

        let base = binding_base(&packet.struct_name, packet.id);
        let mut topic = format!("/rat/{}", base);
        let counter = topic_count.entry(topic.clone()).or_insert(0);
        let schema_name = if *counter > 0 {
            format!("ratitude.{}_0x{:02X}", base, packet.id)
        } else {
            format!("ratitude.{}", base)
        };
        if *counter > 0 {
            topic = format!("{}_0x{:02X}", topic, packet.id);
        }
        *counter += 1;

        let is_quat = packet.packet_type == PacketType::Quat;
        let is_image = packet.packet_type == PacketType::Image;

        out.push(PacketBinding {
            id: packet.id as u8,
            struct_name: packet.struct_name.clone(),
            packet_type: packet.packet_type,
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

fn binding_base(struct_name: &str, packet_id: u16) -> String {
    let base = sanitize_topic_segment(struct_name);
    if base.chars().any(|ch| ch.is_ascii_alphanumeric()) {
        return base;
    }
    format!("packet_0x{:02X}", packet_id)
}

pub(crate) fn log_derived_image_channels(bindings: &[PacketBinding]) {
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
