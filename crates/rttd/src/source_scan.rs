use std::collections::BTreeSet;
use std::time::Duration;

use rat_config::RttdSourceConfig;
use tokio::net::TcpStream;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCandidate {
    pub addr: String,
    pub reachable: bool,
}

pub async fn discover_sources(config: &RttdSourceConfig) -> Vec<SourceCandidate> {
    let mut addresses: BTreeSet<String> = BTreeSet::new();
    if !config.last_selected_addr.trim().is_empty() {
        addresses.insert(config.last_selected_addr.trim().to_string());
    }

    addresses.insert("127.0.0.1:19021".to_string());
    addresses.insert("127.0.0.1:2331".to_string());
    addresses.insert("127.0.0.1:9090".to_string());

    if !config.auto_scan {
        return addresses
            .into_iter()
            .map(|addr| SourceCandidate {
                reachable: addr == config.last_selected_addr,
                addr,
            })
            .collect();
    }

    let timeout = Duration::from_millis(config.scan_timeout_ms.max(1));
    let mut candidates = Vec::new();
    for addr in addresses {
        let reachable = probe_addr(&addr, timeout).await;
        candidates.push(SourceCandidate { addr, reachable });
    }

    candidates.sort_by(|a, b| {
        b.reachable
            .cmp(&a.reachable)
            .then_with(|| a.addr.cmp(&b.addr))
    });
    candidates
}

#[cfg(test)]
pub fn select_default_source(candidates: &[SourceCandidate], fallback: &str) -> String {
    candidates
        .iter()
        .find(|candidate| candidate.reachable)
        .or_else(|| candidates.first())
        .map(|candidate| candidate.addr.clone())
        .unwrap_or_else(|| fallback.to_string())
}

pub fn render_candidates(candidates: &[SourceCandidate]) {
    if candidates.is_empty() {
        println!("no source candidates detected");
        return;
    }

    println!("source candidates:");
    for (idx, candidate) in candidates.iter().enumerate() {
        let status = if candidate.reachable {
            "reachable"
        } else {
            "unreachable"
        };
        println!("  [{idx}] {} ({status})", candidate.addr);
    }
}

async fn probe_addr(addr: &str, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(timeout, TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_source_prefers_reachable_candidate() {
        let candidates = vec![
            SourceCandidate {
                addr: "127.0.0.1:10000".to_string(),
                reachable: false,
            },
            SourceCandidate {
                addr: "127.0.0.1:19021".to_string(),
                reachable: true,
            },
        ];
        assert_eq!(
            select_default_source(&candidates, "127.0.0.1:19021"),
            "127.0.0.1:19021"
        );
    }

    #[test]
    fn default_source_uses_fallback_when_empty() {
        assert_eq!(
            select_default_source(&[], "127.0.0.1:19021"),
            "127.0.0.1:19021"
        );
    }
}
