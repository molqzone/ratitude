use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast::error::TryRecvError;

use super::*;

struct FailOnceSink {
    sent: bool,
}

struct NoopSink;
struct RecoverProbeSink {
    sync_calls: Arc<AtomicUsize>,
    shutdown_calls: Arc<AtomicUsize>,
}

impl PacketSink for FailOnceSink {
    fn key(&self) -> &'static str {
        "jsonl"
    }

    fn sync(
        &mut self,
        _desired: &OutputState,
        _context: Option<&SinkContext>,
        failure_tx: &broadcast::Sender<String>,
    ) -> Result<()> {
        if !self.sent {
            self.sent = true;
            let _ = failure_tx.send("sink failed".to_string());
        }
        Ok(())
    }

    fn shutdown(&mut self) {}
}

impl PacketSink for NoopSink {
    fn key(&self) -> &'static str {
        "jsonl"
    }

    fn sync(
        &mut self,
        _desired: &OutputState,
        _context: Option<&SinkContext>,
        _failure_tx: &broadcast::Sender<String>,
    ) -> Result<()> {
        Ok(())
    }

    fn shutdown(&mut self) {}
}

impl PacketSink for RecoverProbeSink {
    fn key(&self) -> &'static str {
        "probe"
    }

    fn sync(
        &mut self,
        _desired: &OutputState,
        _context: Option<&SinkContext>,
        _failure_tx: &broadcast::Sender<String>,
    ) -> Result<()> {
        self.sync_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn shutdown(&mut self) {
        self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn failure_subscription_receives_sink_error_once() {
    let mut manager = OutputManager::with_sinks_for_test(
        OutputState {
            jsonl_enabled: false,
            jsonl_path: None,
            foxglove_enabled: false,
            foxglove_ws_addr: "127.0.0.1:8765".to_string(),
        },
        vec![Box::new(FailOnceSink { sent: false })],
    )
    .expect("build output manager");
    let mut failures = manager.subscribe_failures();

    let cfg = RatitudeConfig::default();
    manager.reload_from_config(&cfg).expect("reload");

    let first = failures.try_recv().expect("first failure");
    assert!(first.contains("sink failed"));
    assert!(matches!(failures.try_recv(), Err(TryRecvError::Empty)));
}

#[test]
fn reload_from_config_replaces_desired_state() {
    let mut manager = OutputManager::with_sinks_for_test(
        OutputState {
            jsonl_enabled: false,
            jsonl_path: None,
            foxglove_enabled: false,
            foxglove_ws_addr: "127.0.0.1:8765".to_string(),
        },
        vec![Box::new(FailOnceSink { sent: true })],
    )
    .expect("build output manager");
    let mut cfg = RatitudeConfig::default();
    cfg.ratd.outputs.jsonl.enabled = true;
    cfg.ratd.outputs.jsonl.path = "out.jsonl".to_string();
    cfg.ratd.outputs.foxglove.enabled = true;
    cfg.ratd.outputs.foxglove.ws_addr = "127.0.0.1:9000".to_string();

    manager.reload_from_config(&cfg).expect("reload");
    let snapshot = manager.snapshot();
    assert!(snapshot.jsonl_enabled);
    assert_eq!(snapshot.jsonl_path.as_deref(), Some("out.jsonl"));
    assert!(snapshot.foxglove_enabled);
    assert_eq!(snapshot.foxglove_ws_addr, "127.0.0.1:9000");
}

#[test]
fn with_sinks_for_test_rejects_duplicate_sink_keys() {
    let result = OutputManager::with_sinks_for_test(
        OutputState {
            jsonl_enabled: false,
            jsonl_path: None,
            foxglove_enabled: false,
            foxglove_ws_addr: "127.0.0.1:8765".to_string(),
        },
        vec![
            Box::new(FailOnceSink { sent: true }),
            Box::new(FailOnceSink { sent: true }),
        ],
    );
    match result {
        Ok(_) => panic!("duplicate sink keys should fail"),
        Err(err) => assert!(err.to_string().contains("duplicate sink key")),
    }
}

#[tokio::test]
async fn apply_keeps_context_key_for_same_runtime_generation_and_hash() {
    let mut manager = OutputManager::with_sinks_for_test(
        OutputState {
            jsonl_enabled: false,
            jsonl_path: None,
            foxglove_enabled: false,
            foxglove_ws_addr: "127.0.0.1:8765".to_string(),
        },
        vec![Box::new(NoopSink)],
    )
    .expect("build output manager");
    let hub = Hub::new(8);

    manager
        .apply(hub.clone(), 1, 0xABCD_EF01_u64, Vec::new())
        .await
        .expect("first apply");
    let first_key = manager.context.as_ref().map(|ctx| ctx.key);

    manager
        .apply(hub, 1, 0xABCD_EF01_u64, Vec::new())
        .await
        .expect("second apply");
    assert_eq!(manager.context.as_ref().map(|ctx| ctx.key), first_key);
}

#[tokio::test]
async fn apply_updates_context_key_when_runtime_generation_changes() {
    let mut manager = OutputManager::with_sinks_for_test(
        OutputState {
            jsonl_enabled: false,
            jsonl_path: None,
            foxglove_enabled: false,
            foxglove_ws_addr: "127.0.0.1:8765".to_string(),
        },
        vec![Box::new(NoopSink)],
    )
    .expect("build output manager");
    let hub = Hub::new(8);

    manager
        .apply(hub.clone(), 1, 0x1111_u64, Vec::new())
        .await
        .expect("first apply");
    let first_key = manager.context.as_ref().map(|ctx| ctx.key);

    manager
        .apply(hub, 2, 0x1111_u64, Vec::new())
        .await
        .expect("second apply");
    assert_ne!(manager.context.as_ref().map(|ctx| ctx.key), first_key);
}

#[test]
fn recover_after_sink_failure_forces_shutdown_then_reconcile() {
    let sync_calls = Arc::new(AtomicUsize::new(0));
    let shutdown_calls = Arc::new(AtomicUsize::new(0));
    let sink = RecoverProbeSink {
        sync_calls: Arc::clone(&sync_calls),
        shutdown_calls: Arc::clone(&shutdown_calls),
    };
    let mut manager = OutputManager::with_sinks_for_test(
        OutputState {
            jsonl_enabled: false,
            jsonl_path: None,
            foxglove_enabled: false,
            foxglove_ws_addr: "127.0.0.1:8765".to_string(),
        },
        vec![Box::new(sink)],
    )
    .expect("build output manager");

    manager
        .recover_after_sink_failure()
        .expect("recover sinks after failure");

    assert_eq!(shutdown_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sync_calls.load(Ordering::SeqCst), 1);
}
