use rat_protocol::RatPacket;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct Hub {
    tx: broadcast::Sender<RatPacket>,
}

impl Hub {
    pub fn new(buffer: usize) -> Self {
        let capacity = if buffer == 0 { 256 } else { buffer };
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(&self, packet: RatPacket) {
        let _ = self.tx.send(packet);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RatPacket> {
        self.tx.subscribe()
    }
}
