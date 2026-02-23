use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub(crate) struct UnknownPacketWindowReport {
    pub(crate) count: u32,
    pub(crate) unique_ids: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct UnknownPacketObservation {
    pub(crate) total_count: u64,
    pub(crate) window_count: u32,
    pub(crate) threshold_crossed: bool,
    pub(crate) rolled_over: Option<UnknownPacketWindowReport>,
}

#[derive(Clone, Debug)]
pub(crate) struct UnknownPacketMonitor {
    pub(crate) window: Duration,
    pub(crate) threshold: u32,
    window_started_at: Instant,
    window_count: u32,
    total_count: u64,
    per_window_ids: BTreeMap<u8, u32>,
}

impl UnknownPacketMonitor {
    pub(crate) fn new(window: Duration, threshold: u32) -> Self {
        Self {
            window,
            threshold: threshold.max(1),
            window_started_at: Instant::now(),
            window_count: 0,
            total_count: 0,
            per_window_ids: BTreeMap::new(),
        }
    }

    pub(crate) fn record(&mut self, packet_id: u8) -> UnknownPacketObservation {
        self.record_at(packet_id, Instant::now())
    }

    pub(crate) fn record_at(&mut self, packet_id: u8, now: Instant) -> UnknownPacketObservation {
        let mut rolled_over = None;
        if now.duration_since(self.window_started_at) >= self.window {
            if self.window_count > 0 {
                rolled_over = Some(UnknownPacketWindowReport {
                    count: self.window_count,
                    unique_ids: self.per_window_ids.len(),
                });
            }
            self.window_started_at = now;
            self.window_count = 0;
            self.per_window_ids.clear();
        }

        self.window_count = self.window_count.saturating_add(1);
        self.total_count = self.total_count.saturating_add(1);
        *self.per_window_ids.entry(packet_id).or_insert(0) += 1;

        UnknownPacketObservation {
            total_count: self.total_count,
            window_count: self.window_count,
            threshold_crossed: self.window_count == self.threshold,
            rolled_over,
        }
    }
}
