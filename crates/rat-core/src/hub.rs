use tokio::sync::broadcast;

use crate::PacketEnvelope;

#[derive(Clone)]
pub struct Hub {
    tx: broadcast::Sender<PacketEnvelope>,
}

impl Hub {
    pub fn new(buffer: usize) -> Self {
        let capacity = if buffer == 0 { 256 } else { buffer };
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(
        &self,
        packet: PacketEnvelope,
    ) -> Result<usize, broadcast::error::SendError<PacketEnvelope>> {
        self.tx.send(packet)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PacketEnvelope> {
        self.tx.subscribe()
    }
}
