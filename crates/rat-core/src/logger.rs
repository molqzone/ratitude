use std::io::Write;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::{PacketEnvelope, PacketPayload};

#[derive(Serialize)]
struct JsonRecord {
    ts: String,
    id: String,
    payload_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

pub fn spawn_jsonl_writer(
    mut receiver: broadcast::Receiver<PacketEnvelope>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(packet) => {
                    let (data, text) = packet_data_json(&packet.data);
                    let record = JsonRecord {
                        ts: format_timestamp(packet.timestamp),
                        id: format!("0x{:02x}", packet.id),
                        payload_hex: hex::encode(&packet.payload),
                        data,
                        text,
                    };

                    if let Ok(line) = serde_json::to_string(&record) {
                        if let Ok(mut guard) = writer.lock() {
                            let _ = guard.write_all(line.as_bytes());
                            let _ = guard.write_all(b"\n");
                            let _ = guard.flush();
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    })
}

fn format_timestamp(ts: std::time::SystemTime) -> String {
    let duration = ts
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0));
    let nanos = duration.as_nanos() as i128;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .ok()
        .and_then(|odt| odt.format(&Rfc3339).ok())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

fn packet_data_json(data: &PacketPayload) -> (Option<Value>, Option<String>) {
    match data {
        PacketPayload::Text(text) => (Some(Value::String(text.clone())), Some(text.clone())),
        PacketPayload::Dynamic(map) => (Some(Value::Object(map.clone())), None),
    }
}
