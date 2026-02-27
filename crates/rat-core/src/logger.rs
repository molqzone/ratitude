use std::io::Write;
use std::sync::{Arc, Mutex};
use std::{fmt, str::FromStr};

use serde::Serialize;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::warn;

use crate::{PacketEnvelope, PacketPayload};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SinkFailure {
    pub sink_key: SinkKey,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SinkKey {
    Jsonl,
    Foxglove,
    Custom(&'static str),
}

impl SinkKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jsonl => "jsonl",
            Self::Foxglove => "foxglove",
            Self::Custom(key) => key,
        }
    }
}

impl fmt::Display for SinkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SinkKey {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "jsonl" => Ok(Self::Jsonl),
            "foxglove" => Ok(Self::Foxglove),
            _ => Err(()),
        }
    }
}

#[derive(Serialize)]
struct JsonRecord<'a> {
    ts: String,
    id: String,
    payload_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<JsonRecordData<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum JsonRecordData<'a> {
    Text(&'a str),
    Dynamic(&'a serde_json::Map<String, Value>),
}

pub fn spawn_jsonl_writer(
    mut receiver: broadcast::Receiver<PacketEnvelope>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    failure_tx: broadcast::Sender<SinkFailure>,
    sink_key: SinkKey,
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
                    let line = match serde_json::to_vec(&record) {
                        Ok(line) => line,
                        Err(err) => {
                            report_sink_failure(
                                &failure_tx,
                                sink_key,
                                format!("serialize jsonl record failed: {err}"),
                            );
                            break;
                        }
                    };
                    if !write_jsonl_record(Arc::clone(&writer), &failure_tx, sink_key, line).await {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(
                        sink = %sink_key,
                        skipped, "jsonl writer lagged; dropping packets from hub channel"
                    );
                    continue;
                }
            }
        }
    })
}

async fn write_jsonl_record(
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    failure_tx: &broadcast::Sender<SinkFailure>,
    sink_key: SinkKey,
    mut line: Vec<u8>,
) -> bool {
    line.push(b'\n');
    let write_result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut guard = writer
            .lock()
            .map_err(|err| format!("jsonl writer lock poisoned: {err}"))?;
        guard
            .write_all(&line)
            .map_err(|err| format!("write jsonl record failed: {err}"))?;
        Ok(())
    })
    .await;

    match write_result {
        Ok(Ok(())) => true,
        Ok(Err(reason)) => {
            report_sink_failure(failure_tx, sink_key, reason);
            false
        }
        Err(err) => {
            if !err.is_cancelled() {
                report_sink_failure(
                    failure_tx,
                    sink_key,
                    format!("jsonl writer worker join failed: {err}"),
                );
            }
            false
        }
    }
}

fn report_sink_failure(
    failure_tx: &broadcast::Sender<SinkFailure>,
    sink_key: SinkKey,
    reason: String,
) {
    let _ = failure_tx.send(SinkFailure { sink_key, reason });
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

fn packet_data_json(data: &PacketPayload) -> (Option<JsonRecordData<'_>>, Option<&str>) {
    match data {
        PacketPayload::Text(text) => (Some(JsonRecordData::Text(text)), Some(text)),
        PacketPayload::Dynamic(map) => (Some(JsonRecordData::Dynamic(map)), None),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Result as IoResult, Write};
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    use tokio::sync::broadcast;
    use tokio::time::{timeout, Duration};

    use super::{spawn_jsonl_writer, PacketEnvelope, PacketPayload, SinkFailure, SinkKey};

    struct SharedVecWriter {
        output: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for SharedVecWriter {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            self.output
                .lock()
                .expect("shared writer output lock")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    fn text_packet(id: u8, text: &str) -> PacketEnvelope {
        PacketEnvelope {
            id,
            timestamp: SystemTime::UNIX_EPOCH,
            payload: vec![id],
            data: PacketPayload::Text(text.to_string()),
        }
    }

    #[tokio::test]
    async fn writer_keeps_running_after_lagged_receive_error() {
        let (tx, _) = broadcast::channel::<PacketEnvelope>(1);
        let receiver = tx.subscribe();
        let (failure_tx, mut failure_rx) = broadcast::channel::<SinkFailure>(8);

        tx.send(text_packet(0x01, "first"))
            .expect("send first packet");
        tx.send(text_packet(0x02, "second"))
            .expect("send second packet");

        let output = Arc::new(Mutex::new(Vec::new()));
        let writer: Box<dyn Write + Send> = Box::new(SharedVecWriter {
            output: output.clone(),
        });
        let writer = Arc::new(Mutex::new(writer));

        let task = spawn_jsonl_writer(receiver, writer, failure_tx, SinkKey::Jsonl);

        tx.send(text_packet(0x03, "third"))
            .expect("send third packet");
        drop(tx);

        timeout(Duration::from_secs(1), task)
            .await
            .expect("jsonl writer task timed out")
            .expect("jsonl writer task join failed");

        assert!(
            failure_rx.try_recv().is_err(),
            "lagged receive must not report sink failure"
        );

        let written = String::from_utf8(output.lock().expect("shared writer output lock").clone())
            .expect("jsonl output must be utf8");
        assert!(
            written.contains("\"id\":\"0x02\"") || written.contains("\"id\":\"0x03\""),
            "writer must keep consuming packets after lagged event; written={written}"
        );
    }
}
