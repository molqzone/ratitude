use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rat_core::{PacketEnvelope, PacketPayload, SinkFailure, SinkKey};
use tokio::sync::broadcast::error::TryRecvError;

use super::*;

fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}_{unique}"));
    std::fs::create_dir_all(&dir).expect("mkdir temp dir");
    dir
}

struct FailOnceSink {
    sent: bool,
}

struct NoopSink;
struct UnhealthyProbeSink;
struct RecoverProbeSink {
    sync_calls: Arc<AtomicUsize>,
    shutdown_calls: Arc<AtomicUsize>,
}
struct FlakySink {
    failures_left: usize,
}

impl PacketSink for FailOnceSink {
    fn key(&self) -> SinkKey {
        SinkKey::Jsonl
    }

    fn sync(
        &mut self,
        _desired: &OutputState,
        _context: Option<&SinkContext>,
        failure_tx: &broadcast::Sender<SinkFailure>,
    ) -> Result<()> {
        if !self.sent {
            self.sent = true;
            let _ = failure_tx.send(SinkFailure {
                sink_key: self.key(),
                reason: "sink failed".to_string(),
            });
        }
        Ok(())
    }

    fn shutdown(&mut self) {}
}

impl PacketSink for NoopSink {
    fn key(&self) -> SinkKey {
        SinkKey::Jsonl
    }

    fn sync(
        &mut self,
        _desired: &OutputState,
        _context: Option<&SinkContext>,
        _failure_tx: &broadcast::Sender<SinkFailure>,
    ) -> Result<()> {
        Ok(())
    }

    fn shutdown(&mut self) {}
}

impl PacketSink for UnhealthyProbeSink {
    fn key(&self) -> SinkKey {
        SinkKey::Custom("probe")
    }

    fn sync(
        &mut self,
        _desired: &OutputState,
        _context: Option<&SinkContext>,
        _failure_tx: &broadcast::Sender<SinkFailure>,
    ) -> Result<()> {
        Ok(())
    }

    fn shutdown(&mut self) {}

    fn is_healthy(&self, _desired: &OutputState, _context: Option<&SinkContext>) -> bool {
        false
    }
}

impl PacketSink for RecoverProbeSink {
    fn key(&self) -> SinkKey {
        SinkKey::Custom("probe")
    }

    fn sync(
        &mut self,
        _desired: &OutputState,
        _context: Option<&SinkContext>,
        _failure_tx: &broadcast::Sender<SinkFailure>,
    ) -> Result<()> {
        self.sync_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn shutdown(&mut self) {
        self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
    }
}

impl PacketSink for FlakySink {
    fn key(&self) -> SinkKey {
        SinkKey::Custom("flaky")
    }

    fn sync(
        &mut self,
        _desired: &OutputState,
        _context: Option<&SinkContext>,
        _failure_tx: &broadcast::Sender<SinkFailure>,
    ) -> Result<()> {
        if self.failures_left > 0 {
            self.failures_left -= 1;
            return Err(anyhow::anyhow!("flaky sink failed"));
        }
        Ok(())
    }

    fn shutdown(&mut self) {}
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
    assert_eq!(first.sink_key, SinkKey::Jsonl);
    assert!(first.reason.contains("sink failed"));
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
        .recover_sink_after_failure(SinkKey::Custom("probe"))
        .expect("recover sinks after failure");

    assert_eq!(shutdown_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sync_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn recover_sink_failure_tracks_unhealthy_state_until_success() {
    let mut manager = OutputManager::with_sinks_for_test(
        OutputState {
            jsonl_enabled: false,
            jsonl_path: None,
            foxglove_enabled: false,
            foxglove_ws_addr: "127.0.0.1:8765".to_string(),
        },
        vec![Box::new(FlakySink { failures_left: 1 })],
    )
    .expect("build output manager");

    manager
        .recover_sink_after_failure(SinkKey::Custom("flaky"))
        .expect_err("first recovery should fail");
    assert_eq!(
        manager.unhealthy_sink_keys(),
        vec![SinkKey::Custom("flaky")]
    );

    manager
        .recover_sink_after_failure(SinkKey::Custom("flaky"))
        .expect("second recovery should succeed");
    assert!(
        manager.unhealthy_sink_keys().is_empty(),
        "successful recovery should clear unhealthy marker"
    );
}

#[test]
fn refresh_unhealthy_sinks_marks_runtime_unhealthy_probe() {
    let mut manager = OutputManager::with_sinks_for_test(
        OutputState {
            jsonl_enabled: false,
            jsonl_path: None,
            foxglove_enabled: false,
            foxglove_ws_addr: "127.0.0.1:8765".to_string(),
        },
        vec![Box::new(UnhealthyProbeSink)],
    )
    .expect("build output manager");

    assert!(
        manager.unhealthy_sink_keys().is_empty(),
        "precondition: unhealthy should start empty"
    );
    manager.refresh_unhealthy_sinks();
    assert_eq!(
        manager.unhealthy_sink_keys(),
        vec![SinkKey::Custom("probe")]
    );
}

#[tokio::test]
async fn apply_reports_sink_failure_without_stopping_runtime_path() {
    let dir = unique_temp_dir("ratd_jsonl_apply_degrade");
    let invalid_jsonl_path = dir
        .join("blocked")
        .join("packets.jsonl")
        .to_string_lossy()
        .to_string();
    let mut cfg = RatitudeConfig::default();
    cfg.ratd.outputs.foxglove.enabled = false;
    cfg.ratd.outputs.jsonl.enabled = true;
    cfg.ratd.outputs.jsonl.path = invalid_jsonl_path;
    let mut manager = OutputManager::from_config(&cfg).expect("build output manager");
    let mut failures = manager.subscribe_failures();

    manager
        .apply(Hub::new(8), 1, 0xABCD_u64, Vec::new())
        .await
        .expect("runtime apply should degrade instead of failing");

    let failure = failures
        .try_recv()
        .expect("sink failure should be reported");
    assert_eq!(failure.sink_key, SinkKey::Jsonl);
    assert!(failure.reason.contains("output sink apply failed"));
    assert!(failure.reason.contains("failed to open jsonl file"));

    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn apply_marks_sync_failure_sink_as_unhealthy_for_periodic_retry() {
    let mut manager = OutputManager::with_sinks_for_test(
        OutputState {
            jsonl_enabled: false,
            jsonl_path: None,
            foxglove_enabled: false,
            foxglove_ws_addr: "127.0.0.1:8765".to_string(),
        },
        vec![Box::new(FlakySink { failures_left: 2 })],
    )
    .expect("build output manager");
    let mut failures = manager.subscribe_failures();

    manager
        .apply(Hub::new(8), 1, 0xA_u64, Vec::new())
        .await
        .expect("apply should degrade");
    let failure = failures.try_recv().expect("failure should be emitted");
    assert_eq!(failure.sink_key, SinkKey::Custom("flaky"));
    assert_eq!(
        manager.unhealthy_sink_keys(),
        vec![SinkKey::Custom("flaky")]
    );

    manager
        .recover_sink_after_failure(SinkKey::Custom("flaky"))
        .expect_err("first retry should still fail");
    assert_eq!(
        manager.unhealthy_sink_keys(),
        vec![SinkKey::Custom("flaky")]
    );

    manager
        .recover_sink_after_failure(SinkKey::Custom("flaky"))
        .expect("second retry should recover");
    assert!(manager.unhealthy_sink_keys().is_empty());
}

#[tokio::test]
async fn jsonl_sink_ignores_schema_hash_for_same_runtime_generation() {
    let dir = unique_temp_dir("ratd_jsonl_hash_no_restart");
    let jsonl_path = dir.join("packets.jsonl");
    let desired = OutputState {
        jsonl_enabled: true,
        jsonl_path: Some(jsonl_path.to_string_lossy().to_string()),
        foxglove_enabled: false,
        foxglove_ws_addr: "127.0.0.1:8765".to_string(),
    };
    let (failure_tx, _failure_rx) = tokio::sync::broadcast::channel::<SinkFailure>(8);
    let mut sink = JsonlSink::new();
    let hub = Hub::new(8);
    let first_context = SinkContext {
        hub: hub.clone(),
        packets: Vec::new(),
        key: SinkContextKey {
            runtime_generation: 7,
            schema_hash: 0xAAAA,
        },
    };
    let second_context = SinkContext {
        hub,
        packets: Vec::new(),
        key: SinkContextKey {
            runtime_generation: 7,
            schema_hash: 0xBBBB,
        },
    };

    sink.sync(&desired, Some(&first_context), &failure_tx)
        .expect("first sync");
    sink.sync(&desired, Some(&second_context), &failure_tx)
        .expect("second sync");
    assert_eq!(
        sink.restart_count, 0,
        "jsonl should not restart when only schema hash changes"
    );

    sink.shutdown();
    let _ = std::fs::remove_file(&jsonl_path);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn jsonl_sink_restarts_when_task_finished_even_if_state_unchanged() {
    let dir = unique_temp_dir("ratd_jsonl_restart_on_finished");
    let jsonl_path = dir.join("packets.jsonl");
    let desired = OutputState {
        jsonl_enabled: true,
        jsonl_path: Some(jsonl_path.to_string_lossy().to_string()),
        foxglove_enabled: false,
        foxglove_ws_addr: "127.0.0.1:8765".to_string(),
    };
    let (failure_tx, _failure_rx) = tokio::sync::broadcast::channel::<SinkFailure>(8);
    let mut sink = JsonlSink::new();
    let hub = Hub::new(8);
    let context = SinkContext {
        hub,
        packets: Vec::new(),
        key: SinkContextKey {
            runtime_generation: 3,
            schema_hash: 0xAA,
        },
    };

    sink.sync(&desired, Some(&context), &failure_tx)
        .expect("first sync");
    sink.task.as_ref().expect("jsonl task should exist").abort();
    tokio::task::yield_now().await;
    assert!(
        sink.task
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished),
        "precondition: task should be finished after abort"
    );

    sink.sync(&desired, Some(&context), &failure_tx)
        .expect("resync should restart finished task");
    assert_eq!(sink.restart_count, 1, "finished task must trigger restart");
    assert!(
        sink.task.as_ref().is_some_and(|task| !task.is_finished()),
        "restarted task should be running"
    );

    sink.shutdown();
    let _ = std::fs::remove_file(&jsonl_path);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn jsonl_apply_across_runtime_generations_keeps_existing_file_content() {
    let dir = unique_temp_dir("ratd_jsonl_append");
    let jsonl_path = dir.join("packets.jsonl");
    let jsonl_path_str = jsonl_path.to_string_lossy().to_string();

    let mut cfg = RatitudeConfig::default();
    cfg.ratd.outputs.foxglove.enabled = false;
    cfg.ratd.outputs.jsonl.enabled = true;
    cfg.ratd.outputs.jsonl.path = jsonl_path_str;
    let mut manager = OutputManager::from_config(&cfg).expect("build output manager");

    let first_hub = Hub::new(8);
    manager
        .apply(first_hub.clone(), 1, 0x1111_u64, Vec::new())
        .await
        .expect("first apply");
    tokio::time::sleep(Duration::from_millis(40)).await;

    for _ in 0..3 {
        let _ = first_hub.publish(PacketEnvelope {
            id: 0x10,
            timestamp: SystemTime::UNIX_EPOCH,
            payload: vec![0x01],
            data: PacketPayload::Text("first".to_string()),
        });
    }
    tokio::time::sleep(Duration::from_millis(40)).await;

    let second_hub = Hub::new(8);
    manager
        .apply(second_hub.clone(), 2, 0x1111_u64, Vec::new())
        .await
        .expect("second apply");
    tokio::time::sleep(Duration::from_millis(40)).await;

    for _ in 0..3 {
        let _ = second_hub.publish(PacketEnvelope {
            id: 0x11,
            timestamp: SystemTime::UNIX_EPOCH,
            payload: vec![0x02],
            data: PacketPayload::Text("second".to_string()),
        });
    }
    tokio::time::sleep(Duration::from_millis(40)).await;
    manager.shutdown().await;

    let content = std::fs::read_to_string(&jsonl_path).expect("read jsonl");
    let lines = content.lines().collect::<Vec<_>>();
    assert!(
        lines.len() >= 2,
        "jsonl should keep lines across runtime restart"
    );
    assert!(lines.iter().any(|line| line.contains("\"text\":\"first\"")));
    assert!(lines
        .iter()
        .any(|line| line.contains("\"text\":\"second\"")));

    let _ = std::fs::remove_file(&jsonl_path);
    let _ = std::fs::remove_dir_all(dir);
}
