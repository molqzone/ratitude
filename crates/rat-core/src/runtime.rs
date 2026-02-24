mod control;
mod control_message;
mod schema_assembly;
mod unknown_monitor;

use std::time::{Duration, SystemTime};

use rat_config::{FieldDef, PacketDef};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use crate::protocol_engine::{
    decode_frame, PacketEnvelope, ProtocolEngineError, RatProtocolEngine,
};
use crate::{spawn_listener, Hub, ListenerOptions, PacketPayload};

use self::control::{handle_control_payload, ControlOutcome, SchemaState, CONTROL_PACKET_ID};
use self::unknown_monitor::{UnknownPacketMonitor, UnknownPacketObservation};

const SIGNAL_BUFFER: usize = 8;

pub type RuntimeFieldDef = FieldDef;
pub type RuntimePacketDef = PacketDef;

#[derive(Clone, Debug)]
pub struct IngestRuntimeConfig {
    pub addr: String,
    pub listener: ListenerOptions,
    pub hub_buffer: usize,
    pub text_packet_id: u8,
    pub schema_timeout: Duration,
    pub unknown_window: Duration,
    pub unknown_threshold: u32,
}

#[derive(Clone, Debug)]
pub enum RuntimeSignal {
    SchemaReady {
        schema_hash: u64,
        packets: Vec<RuntimePacketDef>,
    },
    Fatal(RuntimeError),
}

#[derive(Debug, Error, Clone)]
pub enum RuntimeError {
    #[error("duplicate packet id in runtime schema: 0x{id:X}")]
    DuplicatePacketId { id: u16 },

    #[error("packet id out of range: 0x{id:X}")]
    PacketIdOutOfRange { id: u16 },

    #[error("register packet 0x{id:02X} ({struct_name}) failed: {reason}")]
    PacketRegisterFailed {
        id: u16,
        struct_name: String,
        reason: String,
    },

    #[error("runtime schema timeout after {timeout_ms}ms (no complete schema received)")]
    SchemaTimeout { timeout_ms: u64 },

    #[error("schema control protocol error: {reason}")]
    ControlProtocol { reason: String },

    #[error("schema size exceeds limit: {actual} > {max}")]
    SchemaTooLarge { actual: usize, max: usize },

    #[error("schema chunk out of order: expected offset {expected}, got {actual}")]
    SchemaChunkOutOfOrder { expected: usize, actual: usize },

    #[error("schema chunk overflow: offset={offset}, chunk={chunk_len}, total={total}")]
    SchemaChunkOverflow {
        offset: usize,
        chunk_len: usize,
        total: usize,
    },

    #[error("schema commit before completion: received {received}, expected {expected}")]
    SchemaCommitBeforeComplete { received: usize, expected: usize },

    #[error(
        "schema hash mismatch: expected 0x{expected:016X}, actual 0x{actual:016X}; run ratsync, rebuild firmware, and reflash before starting ratd"
    )]
    SchemaHashMismatch { expected: u64, actual: u64 },

    #[error("schema parse failed: {reason}")]
    SchemaParseFailed { reason: String },

    #[error("frame consumer stopped before shutdown")]
    FrameConsumerStopped,
}

pub struct IngestRuntime {
    shutdown: CancellationToken,
    hub: Hub,
    listener_task: JoinHandle<()>,
    consume_task: JoinHandle<()>,
    signals_rx: mpsc::Receiver<RuntimeSignal>,
}

impl IngestRuntime {
    pub fn hub(&self) -> Hub {
        self.hub.clone()
    }

    pub async fn recv_signal(&mut self) -> Option<RuntimeSignal> {
        self.signals_rx.recv().await
    }

    pub async fn shutdown(self) {
        let IngestRuntime {
            shutdown,
            hub: _hub,
            listener_task,
            consume_task,
            signals_rx: _signals_rx,
        } = self;

        shutdown.cancel();
        listener_task.abort();
        if let Err(err) = listener_task.await {
            if !err.is_cancelled() {
                warn!(error = %err, "listener task join failed during runtime shutdown");
            }
        }
        if let Err(err) = consume_task.await {
            if !err.is_cancelled() {
                warn!(error = %err, "consumer task join failed during runtime shutdown");
            }
        }
    }
}

pub async fn start_ingest_runtime(cfg: IngestRuntimeConfig) -> Result<IngestRuntime, RuntimeError> {
    let mut protocol = RatProtocolEngine::new();
    protocol.set_text_packet_id(cfg.text_packet_id);
    protocol.clear_dynamic_registry();

    let shutdown = CancellationToken::new();
    let hub = Hub::new(cfg.hub_buffer.max(1));
    let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>(cfg.hub_buffer.max(1));
    let (signals_tx, signals_rx) = mpsc::channel::<RuntimeSignal>(SIGNAL_BUFFER);

    let listener_task = spawn_listener(shutdown.clone(), cfg.addr, frame_tx, cfg.listener);

    let consume_task = spawn_frame_consumer_monitor(
        frame_rx,
        hub.clone(),
        protocol,
        shutdown.clone(),
        cfg.schema_timeout,
        cfg.unknown_window,
        cfg.unknown_threshold,
        signals_tx,
    );

    Ok(IngestRuntime {
        shutdown,
        hub,
        listener_task,
        consume_task,
        signals_rx,
    })
}

fn spawn_frame_consumer_monitor(
    receiver: mpsc::Receiver<Vec<u8>>,
    hub: Hub,
    mut protocol: RatProtocolEngine,
    shutdown: CancellationToken,
    schema_timeout: Duration,
    unknown_window: Duration,
    unknown_threshold: u32,
    signals: mpsc::Sender<RuntimeSignal>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let consume = run_frame_consumer(
            receiver,
            hub,
            &mut protocol,
            shutdown.clone(),
            schema_timeout,
            unknown_window,
            unknown_threshold,
            signals.clone(),
        )
        .await;

        if let Err(err) = consume {
            if signals
                .send(RuntimeSignal::Fatal(err.clone()))
                .await
                .is_err()
            {
                warn!(error = %err, "failed to deliver runtime fatal signal");
            }
            shutdown.cancel();
        }
    })
}

async fn run_frame_consumer(
    mut receiver: mpsc::Receiver<Vec<u8>>,
    hub: Hub,
    protocol: &mut RatProtocolEngine,
    shutdown: CancellationToken,
    schema_timeout: Duration,
    unknown_window: Duration,
    unknown_threshold: u32,
    signals: mpsc::Sender<RuntimeSignal>,
) -> Result<(), RuntimeError> {
    let timeout = schema_timeout.max(Duration::from_millis(1));
    let mut schema_state = SchemaState::new(timeout);
    let mut unknown_monitor = UnknownPacketMonitor::new(unknown_window, unknown_threshold);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep_until(schema_state.wait_deadline()), if !schema_state.is_ready() => {
                shutdown.cancel();
                return Err(RuntimeError::SchemaTimeout { timeout_ms: timeout.as_millis() as u64 });
            }
            maybe_frame = receiver.recv() => {
                let Some(frame) = maybe_frame else {
                    return Err(RuntimeError::FrameConsumerStopped);
                };
                let Some((id, payload)) = decode_transport_frame(&frame) else {
                    continue;
                };

                if id == CONTROL_PACKET_ID {
                    handle_control_frame(
                        &payload,
                        &mut schema_state,
                        protocol,
                        &mut unknown_monitor,
                        unknown_window,
                        unknown_threshold,
                        &signals,
                    )
                    .await?;
                    continue;
                }

                if !schema_state.is_ready() {
                    debug!(packet_id = format!("0x{:02X}", id), "dropping packet before runtime schema becomes ready");
                    continue;
                }

                let Some(data) = parse_data_packet(
                    protocol,
                    id,
                    &payload,
                    &mut unknown_monitor,
                ) else {
                    continue;
                };

                publish_runtime_packet(&hub, id, payload, data);
            }
        }
    }

    Ok(())
}

fn decode_transport_frame(frame: &[u8]) -> Option<(u8, Vec<u8>)> {
    let decoded = match decode_frame(frame) {
        Ok(decoded) => decoded,
        Err(err) => {
            debug!(
                error = %err,
                frame_len = frame.len(),
                "dropping invalid COBS frame"
            );
            return None;
        }
    };
    if decoded.is_empty() {
        return None;
    }

    let id = decoded[0];
    let payload = decoded[1..].to_vec();
    Some((id, payload))
}

async fn handle_control_frame(
    payload: &[u8],
    schema_state: &mut SchemaState,
    protocol: &mut RatProtocolEngine,
    unknown_monitor: &mut UnknownPacketMonitor,
    unknown_window: Duration,
    unknown_threshold: u32,
    signals: &mpsc::Sender<RuntimeSignal>,
) -> Result<(), RuntimeError> {
    let control_outcome = handle_control_payload(payload, schema_state, protocol)?;
    match control_outcome {
        ControlOutcome::SchemaReset => {
            reset_unknown_monitor(unknown_monitor, unknown_window, unknown_threshold);
        }
        ControlOutcome::SchemaReady {
            schema_hash,
            packets,
        } => {
            reset_unknown_monitor(unknown_monitor, unknown_window, unknown_threshold);
            if signals
                .send(RuntimeSignal::SchemaReady {
                    schema_hash,
                    packets,
                })
                .await
                .is_err()
            {
                return Err(RuntimeError::FrameConsumerStopped);
            }
        }
        ControlOutcome::Noop => {}
    }
    Ok(())
}

fn parse_data_packet(
    protocol: &RatProtocolEngine,
    id: u8,
    payload: &[u8],
    unknown_monitor: &mut UnknownPacketMonitor,
) -> Option<PacketPayload> {
    match protocol.parse_packet(id, payload) {
        Ok(data) => Some(data),
        Err(ProtocolEngineError::UnknownPacketId(unknown_id)) => {
            let observation = unknown_monitor.record(unknown_id);
            report_unknown_packet(unknown_id, unknown_monitor, observation);
            None
        }
        Err(err) => {
            warn!(
                packet_id = format!("0x{:02X}", id),
                error = %err,
                payload_len = payload.len(),
                "dropping undecodable packet"
            );
            None
        }
    }
}

fn report_unknown_packet(
    unknown_id: u8,
    unknown_monitor: &UnknownPacketMonitor,
    observation: UnknownPacketObservation,
) {
    if let Some(report) = observation.rolled_over {
        warn!(
            window_secs = unknown_monitor.window.as_secs(),
            dropped = report.count,
            unique_ids = report.unique_ids,
            "unknown packets dropped in previous window"
        );
    }
    if observation.threshold_crossed {
        error!(
            packet_id = format!("0x{:02X}", unknown_id),
            window_secs = unknown_monitor.window.as_secs(),
            threshold = unknown_monitor.threshold,
            window_count = observation.window_count,
            total_unknown = observation.total_count,
            "unknown packet flood detected (not declared in runtime schema)"
        );
    } else {
        warn!(
            packet_id = format!("0x{:02X}", unknown_id),
            window_count = observation.window_count,
            total_unknown = observation.total_count,
            "dropping unknown packet id (not declared in runtime schema)"
        );
    }
}

fn publish_runtime_packet(hub: &Hub, id: u8, payload: Vec<u8>, data: PacketPayload) {
    if hub
        .publish(PacketEnvelope {
            id,
            timestamp: SystemTime::now(),
            payload,
            data,
        })
        .is_err()
    {
        debug!(
            packet_id = format!("0x{:02X}", id),
            "dropping packet because no hub subscribers are active"
        );
    }
}

fn reset_unknown_monitor(
    unknown_monitor: &mut UnknownPacketMonitor,
    unknown_window: Duration,
    unknown_threshold: u32,
) {
    *unknown_monitor = UnknownPacketMonitor::new(unknown_window, unknown_threshold);
}

#[cfg(test)]
mod tests;
