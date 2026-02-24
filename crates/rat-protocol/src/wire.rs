use crate::ProtocolError;

pub fn cobs_decode(frame: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if frame.is_empty() {
        return Ok(Vec::new());
    }

    cobs::decode_vec(frame).map_err(|_| {
        if frame.contains(&0) {
            ProtocolError::InvalidCobsCode
        } else {
            ProtocolError::TruncatedFrame
        }
    })
}

pub fn hash_schema_bytes(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
