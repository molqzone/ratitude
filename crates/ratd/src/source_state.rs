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
    let last_selected = last_selected_addr.trim();
    if let Some(candidate) = candidates
        .iter()
        .find(|candidate| candidate.reachable && candidate.addr == last_selected)
    {
        return Ok(candidate.addr.clone());
    }
    if let Some(candidate) = candidates.iter().find(|candidate| candidate.reachable) {
        return Ok(candidate.addr.clone());
    }
    if let Some(candidate) = candidates
        .iter()
        .find(|candidate| candidate.addr == last_selected)
    {
        return Ok(candidate.addr.clone());
    }
    if let Some(candidate) = candidates.first() {
        return Ok(candidate.addr.clone());
    }
    Err(anyhow!(
        "no RTT source candidate configured; set ratd.source.last_selected_addr or ratd.source.seed_addrs"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(addr: &str, reachable: bool) -> SourceCandidate {
        SourceCandidate {
            addr: addr.to_string(),
            reachable,
        }
    }

    #[test]
    fn select_active_source_prefers_reachable_last_selected() {
        let candidates = vec![
            candidate("127.0.0.1:2331", true),
            candidate("127.0.0.1:19021", true),
        ];
        let selected =
            select_active_source(&candidates, "127.0.0.1:19021").expect("selected source");
        assert_eq!(selected, "127.0.0.1:19021");
    }

    #[test]
    fn select_active_source_falls_back_to_other_reachable_when_last_unreachable() {
        let candidates = vec![
            candidate("127.0.0.1:19021", false),
            candidate("127.0.0.1:2331", true),
        ];
        let selected =
            select_active_source(&candidates, "127.0.0.1:19021").expect("selected source");
        assert_eq!(selected, "127.0.0.1:2331");
    }

    #[test]
    fn select_active_source_falls_back_to_last_selected_when_all_unreachable() {
        let candidates = vec![
            candidate("127.0.0.1:19021", false),
            candidate("127.0.0.1:2331", false),
        ];
        let selected =
            select_active_source(&candidates, "127.0.0.1:19021").expect("selected source");
        assert_eq!(selected, "127.0.0.1:19021");
    }

    #[test]
    fn select_active_source_uses_first_candidate_when_last_missing_and_all_unreachable() {
        let candidates = vec![
            candidate("127.0.0.1:2331", false),
            candidate("127.0.0.1:19021", false),
        ];
        let selected =
            select_active_source(&candidates, "127.0.0.1:9090").expect("selected source");
        assert_eq!(selected, "127.0.0.1:2331");
    }

    #[test]
    fn select_active_source_errors_when_no_candidates_and_no_last_selected() {
        let err = select_active_source(&[], "").expect_err("missing source should fail");
        assert!(err
            .to_string()
            .contains("no RTT source candidate configured"));
    }
}
