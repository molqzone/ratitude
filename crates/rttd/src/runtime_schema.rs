use rat_config::PacketDef;

#[derive(Debug, Clone, Default)]
pub struct RuntimeSchemaState {
    schema_hash: Option<u64>,
    packets: Vec<PacketDef>,
}

impl RuntimeSchemaState {
    pub fn clear(&mut self) {
        self.schema_hash = None;
        self.packets.clear();
    }

    pub fn replace(&mut self, schema_hash: u64, packets: Vec<PacketDef>) {
        self.schema_hash = Some(schema_hash);
        self.packets = packets;
    }

    pub fn schema_hash(&self) -> Option<u64> {
        self.schema_hash
    }

    pub fn packet_count(&self) -> usize {
        self.packets.len()
    }

    pub fn packets(&self) -> &[PacketDef] {
        &self.packets
    }
}
