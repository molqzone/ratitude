use anyhow::{anyhow, Result};
use rat_config::RatitudeConfig;

use crate::daemon::DaemonState;
use crate::runtime_schema::RuntimeSchemaState;
use crate::source_scan::{discover_sources, render_candidates, SourceCandidate};

pub(crate) async fn build_state(config_path: String, cfg: RatitudeConfig) -> Result<DaemonState> {
    let source_candidates = discover_sources(&cfg.ratd.source).await;
    render_candidates(&source_candidates);

    let active_source =
        select_active_source(&source_candidates, &cfg.ratd.source.last_selected_addr)?;

    Ok(DaemonState {
        config_path,
        config: cfg,
        source_candidates,
        active_source,
        runtime_schema: RuntimeSchemaState::default(),
    })
}

pub(crate) async fn refresh_source_candidates(state: &mut DaemonState, render: bool) {
    state.source_candidates = discover_sources(&state.config.ratd.source).await;
    if render {
        render_candidates(&state.source_candidates);
    }
}

pub(crate) fn select_active_source(
    candidates: &[SourceCandidate],
    last_selected_addr: &str,
) -> Result<String> {
    if let Some(candidate) = candidates
        .iter()
        .find(|candidate| candidate.reachable && candidate.addr == last_selected_addr)
    {
        return Ok(candidate.addr.clone());
    }
    if let Some(candidate) = candidates.iter().find(|candidate| candidate.reachable) {
        return Ok(candidate.addr.clone());
    }
    Err(anyhow!(
        "no reachable RTT source detected; start RTT endpoint first"
    ))
}
