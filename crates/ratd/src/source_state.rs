use anyhow::{anyhow, Result};
use rat_config::RatdSourceConfig;

use crate::source_scan::{discover_sources, SourceCandidate};

#[derive(Debug, Clone)]
pub(crate) struct SourceDomainState {
    candidates: Vec<SourceCandidate>,
    active_addr: String,
}

impl SourceDomainState {
    pub(crate) fn new(candidates: Vec<SourceCandidate>, active_addr: String) -> Self {
        Self {
            candidates,
            active_addr,
        }
    }

    pub(crate) fn candidates(&self) -> &[SourceCandidate] {
        &self.candidates
    }

    pub(crate) fn candidate(&self, index: usize) -> Option<&SourceCandidate> {
        self.candidates.get(index)
    }

    pub(crate) fn set_candidates(&mut self, candidates: Vec<SourceCandidate>) {
        self.candidates = candidates;
    }

    pub(crate) fn active_addr(&self) -> &str {
        &self.active_addr
    }

    pub(crate) fn set_active_addr(&mut self, addr: String) {
        self.active_addr = addr;
    }
}

pub(crate) async fn build_source_domain(
    source_cfg: &RatdSourceConfig,
) -> Result<SourceDomainState> {
    let source_candidates = discover_sources(source_cfg).await;

    let active_source = select_active_source(&source_candidates, &source_cfg.last_selected_addr)?;

    Ok(SourceDomainState::new(source_candidates, active_source))
}

pub(crate) async fn refresh_source_candidates(
    source: &mut SourceDomainState,
    source_cfg: &RatdSourceConfig,
) {
    let candidates = discover_sources(source_cfg).await;
    source.set_candidates(candidates);
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
