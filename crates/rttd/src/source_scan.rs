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

    if config.auto_scan {
        addresses.insert("127.0.0.1:19021".to_string());
        addresses.insert("127.0.0.1:2331".to_string());
        addresses.insert("127.0.0.1:9090".to_string());
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
    use rat_config::RttdSourceConfig;
    use tokio::net::TcpListener;

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

    #[tokio::test]
    async fn auto_scan_false_only_probes_last_selected_addr_when_reachable() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr").to_string();

        let config = RttdSourceConfig {
            auto_scan: false,
            scan_timeout_ms: 100,
            last_selected_addr: addr.clone(),
        };
        let candidates = discover_sources(&config).await;

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].addr, addr);
        assert!(candidates[0].reachable);
    }

    #[tokio::test]
    async fn auto_scan_false_only_probes_last_selected_addr_when_unreachable() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr").to_string();
        drop(listener);

        let config = RttdSourceConfig {
            auto_scan: false,
            scan_timeout_ms: 100,
            last_selected_addr: addr.clone(),
        };
        let candidates = discover_sources(&config).await;

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].addr, addr);
        assert!(!candidates[0].reachable);
    }

    #[tokio::test]
    async fn auto_scan_true_keeps_default_candidate_set() {
        let config = RttdSourceConfig {
            auto_scan: true,
            scan_timeout_ms: 1,
            last_selected_addr: String::new(),
        };
        let candidates = discover_sources(&config).await;
        let addresses = candidates
            .into_iter()
            .map(|item| item.addr)
            .collect::<Vec<_>>();

        assert!(addresses.contains(&"127.0.0.1:19021".to_string()));
        assert!(addresses.contains(&"127.0.0.1:2331".to_string()));
        assert!(addresses.contains(&"127.0.0.1:9090".to_string()));
    }
}
