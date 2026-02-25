use super::RuntimeError;

pub(crate) const CONTROL_HELLO: u8 = 0x01;
pub(crate) const CONTROL_SCHEMA_CHUNK: u8 = 0x02;
pub(crate) const CONTROL_SCHEMA_COMMIT: u8 = 0x03;
pub(crate) const CONTROL_MAGIC: &[u8; 4] = b"RATS";
pub(crate) const CONTROL_VERSION: u8 = 1;
const HELLO_PAYLOAD_LEN: usize = 18;
const COMMIT_PAYLOAD_LEN: usize = 9;

pub(crate) enum ControlMessage {
    Hello { total_len: usize, schema_hash: u64 },
    SchemaChunk { offset: usize, chunk: Vec<u8> },
    SchemaCommit { schema_hash: u64 },
}

pub(crate) fn parse_control_message(payload: &[u8]) -> Result<ControlMessage, RuntimeError> {
    let Some(op) = payload.first().copied() else {
        return Err(RuntimeError::ControlProtocol {
            reason: "empty control payload".to_string(),
        });
    };

    match op {
        CONTROL_HELLO => parse_hello_message(payload),
        CONTROL_SCHEMA_CHUNK => parse_chunk_message(payload),
        CONTROL_SCHEMA_COMMIT => parse_commit_message(payload),
        other => Err(RuntimeError::ControlProtocol {
            reason: format!("unknown control opcode: 0x{other:02X}"),
        }),
    }
}

fn parse_hello_message(payload: &[u8]) -> Result<ControlMessage, RuntimeError> {
    if payload.len() != HELLO_PAYLOAD_LEN {
        return Err(RuntimeError::ControlProtocol {
            reason: format!(
                "invalid hello payload length: expected {HELLO_PAYLOAD_LEN}, got {}",
                payload.len()
            ),
        });
    }
    if payload.get(1..5) != Some(CONTROL_MAGIC.as_slice()) {
        return Err(RuntimeError::ControlProtocol {
            reason: "invalid hello magic".to_string(),
        });
    }
    if payload[5] != CONTROL_VERSION {
        return Err(RuntimeError::ControlProtocol {
            reason: format!(
                "unsupported control version: expected {CONTROL_VERSION}, got {}",
                payload[5]
            ),
        });
    }

    let total_len = read_u32_le(&payload[6..10])? as usize;
    let schema_hash = read_u64_le(&payload[10..18])?;
    if total_len == 0 {
        return Err(RuntimeError::ControlProtocol {
            reason: "schema total length must be > 0".to_string(),
        });
    }

    Ok(ControlMessage::Hello {
        total_len,
        schema_hash,
    })
}

fn parse_chunk_message(payload: &[u8]) -> Result<ControlMessage, RuntimeError> {
    if payload.len() < 7 {
        return Err(RuntimeError::ControlProtocol {
            reason: "schema chunk payload too short".to_string(),
        });
    }

    let offset = read_u32_le(&payload[1..5])? as usize;
    let chunk_len = read_u16_le(&payload[5..7])? as usize;
    if chunk_len == 0 {
        return Err(RuntimeError::ControlProtocol {
            reason: "schema chunk length must be > 0".to_string(),
        });
    }
    let expected_len = 7 + chunk_len;
    if payload.len() != expected_len {
        return Err(RuntimeError::ControlProtocol {
            reason: format!(
                "schema chunk length mismatch: declared {chunk_len}, payload {}",
                payload.len() - 7
            ),
        });
    }

    Ok(ControlMessage::SchemaChunk {
        offset,
        chunk: payload[7..].to_vec(),
    })
}

fn parse_commit_message(payload: &[u8]) -> Result<ControlMessage, RuntimeError> {
    if payload.len() != COMMIT_PAYLOAD_LEN {
        return Err(RuntimeError::ControlProtocol {
            reason: format!(
                "invalid schema commit payload length: expected {COMMIT_PAYLOAD_LEN}, got {}",
                payload.len()
            ),
        });
    }

    let schema_hash = read_u64_le(&payload[1..9])?;
    Ok(ControlMessage::SchemaCommit { schema_hash })
}

fn read_u16_le(raw: &[u8]) -> Result<u16, RuntimeError> {
    if raw.len() != 2 {
        return Err(RuntimeError::ControlProtocol {
            reason: format!("invalid u16 width: {}", raw.len()),
        });
    }
    let mut bytes = [0_u8; 2];
    bytes.copy_from_slice(raw);
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32_le(raw: &[u8]) -> Result<u32, RuntimeError> {
    if raw.len() != 4 {
        return Err(RuntimeError::ControlProtocol {
            reason: format!("invalid u32 width: {}", raw.len()),
        });
    }
    let mut bytes = [0_u8; 4];
    bytes.copy_from_slice(raw);
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64_le(raw: &[u8]) -> Result<u64, RuntimeError> {
    if raw.len() != 8 {
        return Err(RuntimeError::ControlProtocol {
            reason: format!("invalid u64 width: {}", raw.len()),
        });
    }
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(raw);
    Ok(u64::from_le_bytes(bytes))
}
