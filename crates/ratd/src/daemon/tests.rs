use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rat_config::ConfigStore;
use rat_core::Hub;
use tokio::net::TcpListener;

use super::*;
use crate::command_loop::handle_console_command;
use crate::config_io::load_config;
use crate::console::ConsoleCommand;
use crate::source_scan::SourceCandidate;
use crate::source_state::{build_source_domain, select_active_source, SourceDomainState};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}_{unique}"));
    fs::create_dir_all(&dir).expect("mkdir temp dir");
    dir
}

#[test]
fn select_active_source_prefers_reachable_last_selected() {
    let candidates = vec![
        SourceCandidate {
            addr: "127.0.0.1:19021".to_string(),
            reachable: true,
        },
        SourceCandidate {
            addr: "127.0.0.1:2331".to_string(),
            reachable: true,
        },
    ];
    let selected = select_active_source(&candidates, "127.0.0.1:2331").expect("select");
    assert_eq!(selected, "127.0.0.1:2331");
}

#[test]
fn select_active_source_falls_back_when_no_reachable_candidate() {
    let candidates = vec![
        SourceCandidate {
            addr: "127.0.0.1:19021".to_string(),
            reachable: false,
        },
        SourceCandidate {
            addr: "127.0.0.1:2331".to_string(),
            reachable: false,
        },
    ];
    let selected = select_active_source(&candidates, "127.0.0.1:19021").expect("select source");
    assert_eq!(selected, "127.0.0.1:19021");
}

#[tokio::test]
async fn build_state_does_not_overwrite_last_selected_addr_on_startup() {
    let dir = unique_temp_dir("ratd_build_state");
    let config_path = dir.join("rat.toml");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr").to_string();

    let mut cfg = RatitudeConfig::default();
    cfg.project.scan_root = ".".to_string();
    cfg.ratd.source.auto_scan = false;
    cfg.ratd.source.last_selected_addr = addr.clone();
    ConfigStore::new(&config_path)
        .save(&cfg)
        .expect("save config");

    let source = build_source_domain(&cfg.ratd.source)
        .await
        .expect("build source");
    let state = DaemonState::new(config_path.to_string_lossy().to_string(), cfg, source);
    assert_eq!(state.source().active_addr(), addr);
    assert_eq!(state.config().ratd.source.last_selected_addr, addr);

    let raw = fs::read_to_string(&config_path).expect("read config");
    assert!(raw.contains("last_selected_addr ="));

    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn source_list_refreshes_candidates_before_render() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr").to_string();

    let mut cfg = RatitudeConfig::default();
    cfg.ratd.source.auto_scan = false;
    cfg.ratd.source.scan_timeout_ms = 100;
    cfg.ratd.source.last_selected_addr = addr.clone();

    let mut state = DaemonState::new(
        String::new(),
        cfg.clone(),
        SourceDomainState::new(
            vec![
                SourceCandidate {
                    addr: "127.0.0.1:19021".to_string(),
                    reachable: false,
                },
                SourceCandidate {
                    addr: "127.0.0.1:2331".to_string(),
                    reachable: false,
                },
            ],
            addr.clone(),
        ),
    );
    let mut output_manager = OutputManager::from_config(&cfg).expect("build output manager");
    let action =
        handle_console_command(ConsoleCommand::SourceList, &mut state, &mut output_manager)
            .await
            .expect("source list");
    assert!(!action.should_quit);
    assert!(!action.restart_runtime);
    assert_eq!(state.source().candidates().len(), 1);
    assert_eq!(state.source().candidates()[0].addr, addr);
    assert!(state.source().candidates()[0].reachable);
}

#[tokio::test]
async fn source_use_revalidates_index_against_refreshed_candidates() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr").to_string();

    let mut cfg = RatitudeConfig::default();
    cfg.ratd.source.auto_scan = false;
    cfg.ratd.source.scan_timeout_ms = 100;
    cfg.ratd.source.last_selected_addr = addr.clone();

    let original_active = addr.clone();
    let mut state = DaemonState::new(
        String::new(),
        cfg.clone(),
        SourceDomainState::new(
            vec![
                SourceCandidate {
                    addr: addr.clone(),
                    reachable: true,
                },
                SourceCandidate {
                    addr: "127.0.0.1:65535".to_string(),
                    reachable: true,
                },
            ],
            original_active.clone(),
        ),
    );
    let mut output_manager = OutputManager::from_config(&cfg).expect("build output manager");
    let action = handle_console_command(
        ConsoleCommand::SourceUse(1),
        &mut state,
        &mut output_manager,
    )
    .await
    .expect("source use");

    assert!(!action.should_quit);
    assert!(
        !action.restart_runtime,
        "index 1 should be invalid after refresh"
    );
    assert_eq!(state.source().active_addr(), original_active);
    assert_eq!(state.source().candidates().len(), 1);
    assert_eq!(state.source().candidates()[0].addr, addr);
}

#[tokio::test]
async fn output_commands_apply_without_runtime_restart() {
    let dir = unique_temp_dir("ratd_output_command_apply");
    let config_path = dir.join("rat.toml");
    let config_path_str = config_path.to_string_lossy().to_string();

    let cfg = RatitudeConfig::default();
    ConfigStore::new(&config_path)
        .save(&cfg)
        .expect("save config");

    let mut state = DaemonState::new(
        config_path_str.clone(),
        cfg.clone(),
        SourceDomainState::new(Vec::new(), "127.0.0.1:19021".to_string()),
    );
    let mut output_manager = OutputManager::from_config(&cfg).expect("build output manager");
    let foxglove_action = handle_console_command(
        ConsoleCommand::Foxglove(true),
        &mut state,
        &mut output_manager,
    )
    .await
    .expect("foxglove command");
    assert!(!foxglove_action.restart_runtime);

    let jsonl_action = handle_console_command(
        ConsoleCommand::Jsonl {
            enabled: true,
            path: Some(String::new()),
        },
        &mut state,
        &mut output_manager,
    )
    .await
    .expect("jsonl command");
    assert!(!jsonl_action.restart_runtime);

    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn jsonl_command_failure_does_not_persist_invalid_config() {
    let dir = unique_temp_dir("ratd_jsonl_command_rollback");
    let config_path = dir.join("rat.toml");
    let config_path_str = config_path.to_string_lossy().to_string();

    let cfg = RatitudeConfig::default();
    ConfigStore::new(&config_path)
        .save(&cfg)
        .expect("save config");

    let mut state = DaemonState::new(
        config_path_str.clone(),
        cfg.clone(),
        SourceDomainState::new(Vec::new(), "127.0.0.1:19021".to_string()),
    );
    let mut output_manager = OutputManager::from_config(&cfg).expect("build output manager");
    output_manager
        .apply(Hub::new(8), 1, 0x1, Vec::new())
        .await
        .expect("attach runtime context");

    let invalid_jsonl_path = config_path
        .join("blocked.jsonl")
        .to_string_lossy()
        .to_string();
    let result = handle_console_command(
        ConsoleCommand::Jsonl {
            enabled: true,
            path: Some(invalid_jsonl_path.clone()),
        },
        &mut state,
        &mut output_manager,
    )
    .await;
    let err = match result {
        Ok(_) => panic!("jsonl command should fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("failed to apply jsonl output"));
    assert_eq!(
        state.config().ratd.outputs.jsonl.path,
        cfg.ratd.outputs.jsonl.path
    );

    let raw = fs::read_to_string(&config_path).expect("read config");
    assert!(
        !raw.contains(&invalid_jsonl_path),
        "invalid jsonl path must not be persisted on command failure"
    );

    output_manager.shutdown().await;
    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn load_config_does_not_rewrite_existing_file() {
    let dir = unique_temp_dir("ratd_load_config_read_only");
    let config_path = dir.join("rat.toml");

    let raw = r#"# keep comment and formatting
[project]
name = "demo"
scan_root = "."
recursive = true
extensions = [".c", ".h"]

[generation]
out_dir = "."
header_name = "rat_gen.h"

[ratd]
text_id = 255
"#;
    fs::write(&config_path, raw).expect("write config");

    let _cfg = load_config(&config_path.to_string_lossy())
        .await
        .expect("load");
    let after = fs::read_to_string(&config_path).expect("read config");
    assert_eq!(after, raw);

    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn load_config_fails_when_missing() {
    let dir = unique_temp_dir("ratd_load_config_missing");
    let config_path = dir.join("rat.toml");
    assert!(!config_path.exists());

    let err = load_config(&config_path.to_string_lossy())
        .await
        .expect_err("load should fail without rat.toml");
    assert!(err.to_string().contains("config file does not exist"));
    assert!(!config_path.exists());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ratd_manifest_keeps_protocol_dependency_indirect() {
    let manifest = include_str!("../../Cargo.toml");
    assert!(
        !manifest.contains("rat-protocol"),
        "ratd must consume protocol via rat-core runtime only"
    );
    assert!(
        manifest.contains("rat-core"),
        "ratd must depend on rat-core runtime"
    );
}

#[test]
fn output_failure_lagged_is_non_fatal() {
    let mut output_manager =
        OutputManager::from_config(&RatitudeConfig::default()).expect("build output manager");
    let mut backoff = SinkRecoveryBackoff::new(Duration::from_secs(1));
    let result = process_output_failure(
        Err(tokio::sync::broadcast::error::RecvError::Lagged(3)),
        &mut output_manager,
        &mut backoff,
    );
    assert!(result.expect("lagged should keep listener attached"));
}

#[test]
fn output_failure_marks_sink_unhealthy_even_when_retry_is_throttled() {
    let mut output_manager =
        OutputManager::from_config(&RatitudeConfig::default()).expect("build output manager");
    let mut backoff = SinkRecoveryBackoff::new(Duration::from_secs(60));
    let now = Instant::now();
    assert!(
        backoff.should_attempt("jsonl", now),
        "precondition: first retry should be allowed"
    );

    let result = process_output_failure(
        Ok(rat_core::SinkFailure {
            sink_key: "jsonl",
            reason: "sink failed".to_string(),
        }),
        &mut output_manager,
        &mut backoff,
    );
    assert!(result.expect("sink failure should keep listener attached"));
    assert_eq!(
        output_manager.unhealthy_sink_keys(),
        vec!["jsonl"],
        "failure must enter unhealthy set even when retry is throttled"
    );
}

#[test]
fn output_failure_lagged_attempts_recovery_for_all_sink_keys() {
    let mut output_manager =
        OutputManager::from_config(&RatitudeConfig::default()).expect("build output manager");
    let mut backoff = SinkRecoveryBackoff::new(Duration::from_secs(30));
    assert!(
        output_manager.unhealthy_sink_keys().is_empty(),
        "precondition: no unhealthy sinks"
    );

    let result = process_output_failure(
        Err(tokio::sync::broadcast::error::RecvError::Lagged(3)),
        &mut output_manager,
        &mut backoff,
    );
    assert!(result.expect("lagged should keep listener attached"));
    assert!(
        backoff.next_retry_at.contains_key("jsonl"),
        "lagged compensation should cover jsonl even without unhealthy marker"
    );
    assert!(
        backoff.next_retry_at.contains_key("foxglove"),
        "lagged compensation should cover foxglove even without unhealthy marker"
    );
}

#[tokio::test]
async fn output_failure_lagged_attempts_unhealthy_sink_recovery() {
    let dir = unique_temp_dir("ratd_output_lagged_recovery");
    let invalid_jsonl_path = dir
        .join("blocked")
        .join("packets.jsonl")
        .to_string_lossy()
        .to_string();
    let mut cfg = RatitudeConfig::default();
    cfg.ratd.outputs.foxglove.enabled = false;
    cfg.ratd.outputs.jsonl.enabled = true;
    cfg.ratd.outputs.jsonl.path = invalid_jsonl_path;

    let mut output_manager = OutputManager::from_config(&cfg).expect("build output manager");
    output_manager
        .apply(Hub::new(8), 1, 0x1, Vec::new())
        .await
        .expect("apply should degrade");
    assert_eq!(output_manager.unhealthy_sink_keys(), vec!["jsonl"]);

    let mut backoff = SinkRecoveryBackoff::new(Duration::from_secs(30));
    assert!(
        !backoff.next_retry_at.contains_key("jsonl"),
        "lagged compensation should set retry schedule"
    );
    let result = process_output_failure(
        Err(tokio::sync::broadcast::error::RecvError::Lagged(5)),
        &mut output_manager,
        &mut backoff,
    );
    assert!(result.expect("lagged should keep listener attached"));
    assert!(
        backoff.next_retry_at.contains_key("jsonl"),
        "lagged compensation should attempt unhealthy sink recovery"
    );
    assert_eq!(
        output_manager.unhealthy_sink_keys(),
        vec!["jsonl"],
        "invalid path keeps sink unhealthy after retry attempt"
    );

    output_manager.shutdown().await;
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn output_failure_reason_is_non_fatal() {
    let mut output_manager =
        OutputManager::from_config(&RatitudeConfig::default()).expect("build output manager");
    let mut backoff = SinkRecoveryBackoff::new(Duration::from_secs(1));
    let result = process_output_failure(
        Ok(rat_core::SinkFailure {
            sink_key: "jsonl",
            reason: "sink failed".to_string(),
        }),
        &mut output_manager,
        &mut backoff,
    );
    assert!(result.expect("sink failure should keep listener attached"));
}

#[test]
fn output_failure_channel_closed_is_non_fatal() {
    let mut output_manager =
        OutputManager::from_config(&RatitudeConfig::default()).expect("build output manager");
    let mut backoff = SinkRecoveryBackoff::new(Duration::from_secs(1));
    let result = process_output_failure(
        Err(tokio::sync::broadcast::error::RecvError::Closed),
        &mut output_manager,
        &mut backoff,
    );
    assert!(!result.expect("closed should detach listener"));
}

#[test]
fn sink_recovery_backoff_throttles_immediate_retry() {
    let mut backoff = SinkRecoveryBackoff::new(Duration::from_secs(1));
    let now = Instant::now();

    assert!(backoff.should_attempt("jsonl", now));
    assert!(!backoff.should_attempt("jsonl", now + Duration::from_millis(500)));
    assert!(backoff.should_attempt("jsonl", now + Duration::from_secs(1)));
    assert!(backoff.should_attempt("foxglove", now + Duration::from_millis(500)));
}

#[test]
fn console_channel_closed_keeps_daemon_running_without_console() {
    let state = process_console_channel_closed();
    assert!(!state.keep_attached);
    assert!(!state.should_quit);
}
