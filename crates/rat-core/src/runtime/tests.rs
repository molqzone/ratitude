use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use rat_protocol::hash_schema_bytes;
use rat_protocol::PacketType;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::control::{
    CONTROL_HELLO, CONTROL_MAGIC, CONTROL_SCHEMA_CHUNK, CONTROL_SCHEMA_COMMIT, CONTROL_VERSION,
};
use super::*;
use crate::PacketPayload;

fn encode_frame(decoded: &[u8]) -> Vec<u8> {
    let mut encoded = cobs::encode_vec(decoded);
    encoded.push(0);
    encoded
}

fn listener_opts() -> ListenerOptions {
    ListenerOptions {
        reconnect: Duration::from_millis(50),
        reconnect_max: Duration::from_millis(200),
        dial_timeout: Duration::from_millis(200),
        reader_buf_bytes: 1024,
    }
}

fn schema_toml() -> String {
    [
        "[[packets]]",
        "id = 33",
        "struct_name = \"DemoPacket\"",
        "type = \"plot\"",
        "packed = true",
        "byte_size = 4",
        "",
        "[[packets.fields]]",
        "name = \"value\"",
        "c_type = \"uint32_t\"",
        "offset = 0",
        "size = 4",
    ]
    .join("\n")
}

fn schema_toml_with_duplicate_id() -> String {
    [
        "[[packets]]",
        "id = 33",
        "struct_name = \"DemoPacketA\"",
        "type = \"plot\"",
        "packed = true",
        "byte_size = 4",
        "",
        "[[packets.fields]]",
        "name = \"value\"",
        "c_type = \"uint32_t\"",
        "offset = 0",
        "size = 4",
        "",
        "[[packets]]",
        "id = 33",
        "struct_name = \"DemoPacketB\"",
        "type = \"plot\"",
        "packed = true",
        "byte_size = 4",
        "",
        "[[packets.fields]]",
        "name = \"value\"",
        "c_type = \"uint32_t\"",
        "offset = 0",
        "size = 4",
    ]
    .join("\n")
}

fn schema_toml_with_unknown_field() -> String {
    [
        "[[packets]]",
        "id = 33",
        "struct_name = \"DemoPacket\"",
        "type = \"plot\"",
        "packed = true",
        "byte_size = 4",
        "extra = \"x\"",
        "",
        "[[packets.fields]]",
        "name = \"value\"",
        "c_type = \"uint32_t\"",
        "offset = 0",
        "size = 4",
    ]
    .join("\n")
}

fn control_frames(schema: &[u8], chunk_size: usize) -> Vec<Vec<u8>> {
    let schema_hash = hash_schema_bytes(schema);
    let mut decoded_frames = Vec::new();

    let mut hello = vec![CONTROL_PACKET_ID, CONTROL_HELLO];
    hello.extend_from_slice(CONTROL_MAGIC);
    hello.push(CONTROL_VERSION);
    hello.extend_from_slice(&(schema.len() as u32).to_le_bytes());
    hello.extend_from_slice(&schema_hash.to_le_bytes());
    decoded_frames.push(hello);

    let mut offset = 0usize;
    let chunk_size = chunk_size.max(1);
    while offset < schema.len() {
        let end = (offset + chunk_size).min(schema.len());
        let chunk = &schema[offset..end];
        let mut frame = vec![CONTROL_PACKET_ID, CONTROL_SCHEMA_CHUNK];
        frame.extend_from_slice(&(offset as u32).to_le_bytes());
        frame.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        frame.extend_from_slice(chunk);
        decoded_frames.push(frame);
        offset = end;
    }

    let mut commit = vec![CONTROL_PACKET_ID, CONTROL_SCHEMA_COMMIT];
    commit.extend_from_slice(&schema_hash.to_le_bytes());
    decoded_frames.push(commit);

    decoded_frames
        .into_iter()
        .map(|decoded| encode_frame(&decoded))
        .collect()
}

async fn spawn_frames_once(listener: TcpListener, frames: Vec<Vec<u8>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        for frame in frames {
            socket.write_all(&frame).await.expect("write frame");
        }
    })
}

async fn spawn_frames_with_interval(
    listener: TcpListener,
    frames: Vec<Vec<u8>>,
    interval: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        for (idx, frame) in frames.into_iter().enumerate() {
            if idx > 0 {
                tokio::time::sleep(interval).await;
            }
            socket.write_all(&frame).await.expect("write frame");
        }
    })
}

#[test]
fn unknown_packet_monitor_threshold_once_per_window() {
    let mut monitor = UnknownPacketMonitor::new(Duration::from_secs(10), 3);
    let start = Instant::now();

    let first = monitor.record_at(0x10, start);
    assert!(!first.threshold_crossed);

    let second = monitor.record_at(0x10, start + Duration::from_millis(1));
    assert!(!second.threshold_crossed);

    let third = monitor.record_at(0x10, start + Duration::from_millis(2));
    assert!(third.threshold_crossed);

    let fourth = monitor.record_at(0x10, start + Duration::from_millis(3));
    assert!(!fourth.threshold_crossed);
}

#[tokio::test]
async fn runtime_decodes_and_publishes_valid_packet_after_schema_ready() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let schema = schema_toml().into_bytes();

    let mut frames = control_frames(&schema, 16);
    frames.push(encode_frame(&[0x21, 1, 0, 0, 0]));
    let send_task = spawn_frames_once(listener, frames).await;

    let mut runtime = start_ingest_runtime(IngestRuntimeConfig {
        addr,
        listener: listener_opts(),
        hub_buffer: 8,
        text_packet_id: 0xFF,
        schema_timeout: Duration::from_secs(1),
        unknown_window: Duration::from_secs(5),
        unknown_threshold: 20,
    })
    .await
    .expect("start runtime");

    let mut sub = runtime.hub().subscribe();
    let ready = tokio::time::timeout(Duration::from_secs(1), runtime.recv_signal())
        .await
        .expect("schema ready timeout")
        .expect("schema ready signal");
    assert!(matches!(ready, RuntimeSignal::SchemaReady { .. }));

    let packet = tokio::time::timeout(Duration::from_secs(1), sub.recv())
        .await
        .expect("recv timeout")
        .expect("recv packet");

    assert_eq!(packet.id, 0x21);
    let PacketPayload::Dynamic(map) = packet.data else {
        panic!("expected dynamic packet");
    };
    let expected = BTreeMap::from([("value".to_string(), serde_json::Value::from(1_u64))]);
    let actual = map
        .into_iter()
        .collect::<BTreeMap<String, serde_json::Value>>();
    assert_eq!(actual, expected);

    runtime.shutdown().await;
    let _ = send_task.await;
}

#[tokio::test]
async fn runtime_emits_schema_ready_signal() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let schema = schema_toml().into_bytes();

    let send_task = spawn_frames_once(listener, control_frames(&schema, 7)).await;

    let mut runtime = start_ingest_runtime(IngestRuntimeConfig {
        addr,
        listener: listener_opts(),
        hub_buffer: 8,
        text_packet_id: 0xFF,
        schema_timeout: Duration::from_secs(1),
        unknown_window: Duration::from_secs(5),
        unknown_threshold: 20,
    })
    .await
    .expect("start runtime");

    let signal = tokio::time::timeout(Duration::from_secs(1), runtime.recv_signal())
        .await
        .expect("signal timeout")
        .expect("signal");
    match signal {
        RuntimeSignal::SchemaReady {
            schema_hash,
            packets,
        } => {
            assert_eq!(schema_hash, hash_schema_bytes(&schema));
            assert_eq!(packets.len(), 1);
            assert_eq!(packets[0].id, 0x21);
            assert_eq!(packets[0].packet_type, PacketType::Plot);
        }
        other => panic!("unexpected signal: {other:?}"),
    }

    runtime.shutdown().await;
    let _ = send_task.await;
}

#[tokio::test]
async fn runtime_schema_timeout_renews_while_chunks_keep_arriving() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let schema = schema_toml().into_bytes();
    let frames = control_frames(&schema, 8);
    let send_task = spawn_frames_with_interval(listener, frames, Duration::from_millis(40)).await;

    let mut runtime = start_ingest_runtime(IngestRuntimeConfig {
        addr,
        listener: listener_opts(),
        hub_buffer: 8,
        text_packet_id: 0xFF,
        schema_timeout: Duration::from_millis(90),
        unknown_window: Duration::from_secs(5),
        unknown_threshold: 20,
    })
    .await
    .expect("start runtime");

    let signal = tokio::time::timeout(Duration::from_secs(3), runtime.recv_signal())
        .await
        .expect("signal timeout")
        .expect("signal");
    assert!(matches!(signal, RuntimeSignal::SchemaReady { .. }));

    runtime.shutdown().await;
    let _ = send_task.await;
}

#[tokio::test]
async fn runtime_emits_fatal_on_schema_hash_mismatch() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let schema = schema_toml().into_bytes();
    let schema_hash = hash_schema_bytes(&schema);

    let mut frames = Vec::new();
    let mut hello = vec![CONTROL_PACKET_ID, CONTROL_HELLO];
    hello.extend_from_slice(CONTROL_MAGIC);
    hello.push(CONTROL_VERSION);
    hello.extend_from_slice(&(schema.len() as u32).to_le_bytes());
    hello.extend_from_slice(&schema_hash.to_le_bytes());
    frames.push(encode_frame(&hello));

    let mut chunk = vec![CONTROL_PACKET_ID, CONTROL_SCHEMA_CHUNK];
    chunk.extend_from_slice(&0_u32.to_le_bytes());
    chunk.extend_from_slice(&(schema.len() as u16).to_le_bytes());
    chunk.extend_from_slice(&schema);
    frames.push(encode_frame(&chunk));

    let mut commit = vec![CONTROL_PACKET_ID, CONTROL_SCHEMA_COMMIT];
    commit.extend_from_slice(&(schema_hash ^ 0xFF).to_le_bytes());
    frames.push(encode_frame(&commit));

    let send_task = spawn_frames_once(listener, frames).await;

    let mut runtime = start_ingest_runtime(IngestRuntimeConfig {
        addr,
        listener: listener_opts(),
        hub_buffer: 8,
        text_packet_id: 0xFF,
        schema_timeout: Duration::from_secs(1),
        unknown_window: Duration::from_secs(5),
        unknown_threshold: 20,
    })
    .await
    .expect("start runtime");

    let signal = tokio::time::timeout(Duration::from_secs(1), runtime.recv_signal())
        .await
        .expect("signal timeout")
        .expect("signal");
    match signal {
        RuntimeSignal::Fatal(RuntimeError::SchemaHashMismatch { .. }) => {}
        other => panic!("unexpected signal: {other:?}"),
    }

    runtime.shutdown().await;
    let _ = send_task.await;
}

#[tokio::test]
async fn runtime_emits_fatal_on_duplicate_packet_id() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let schema = schema_toml_with_duplicate_id().into_bytes();
    let send_task = spawn_frames_once(listener, control_frames(&schema, 32)).await;

    let mut runtime = start_ingest_runtime(IngestRuntimeConfig {
        addr,
        listener: listener_opts(),
        hub_buffer: 8,
        text_packet_id: 0xFF,
        schema_timeout: Duration::from_secs(1),
        unknown_window: Duration::from_secs(5),
        unknown_threshold: 20,
    })
    .await
    .expect("start runtime");

    let signal = tokio::time::timeout(Duration::from_secs(1), runtime.recv_signal())
        .await
        .expect("signal timeout")
        .expect("signal");
    match signal {
        RuntimeSignal::Fatal(RuntimeError::DuplicatePacketId { id }) => {
            assert_eq!(id, 33);
        }
        other => panic!("unexpected signal: {other:?}"),
    }

    runtime.shutdown().await;
    let _ = send_task.await;
}

#[tokio::test]
async fn runtime_emits_fatal_on_unknown_schema_field() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let schema = schema_toml_with_unknown_field().into_bytes();
    let send_task = spawn_frames_once(listener, control_frames(&schema, 32)).await;

    let mut runtime = start_ingest_runtime(IngestRuntimeConfig {
        addr,
        listener: listener_opts(),
        hub_buffer: 8,
        text_packet_id: 0xFF,
        schema_timeout: Duration::from_secs(1),
        unknown_window: Duration::from_secs(5),
        unknown_threshold: 20,
    })
    .await
    .expect("start runtime");

    let signal = tokio::time::timeout(Duration::from_secs(1), runtime.recv_signal())
        .await
        .expect("signal timeout")
        .expect("signal");
    match signal {
        RuntimeSignal::Fatal(RuntimeError::SchemaParseFailed { reason }) => {
            assert!(
                reason.contains("unknown field"),
                "unexpected reason: {reason}"
            );
        }
        other => panic!("unexpected signal: {other:?}"),
    }

    runtime.shutdown().await;
    let _ = send_task.await;
}
