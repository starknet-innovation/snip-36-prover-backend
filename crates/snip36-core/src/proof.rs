//! Proof encoding/decoding utilities.
//!
//! The prover outputs proof as a base64-encoded string. Legacy format was
//! cairo-serde JSON (array of hex felt strings) packed as big-endian u32 values.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

/// Decode a base64 proof string into raw bytes.
pub fn decode_proof_base64(b64: &str) -> Result<Vec<u8>, ProofError> {
    BASE64
        .decode(b64.trim())
        .map_err(|e| ProofError::Base64Decode(e.to_string()))
}

/// Encode raw proof bytes as a base64 string.
pub fn encode_proof_base64(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

/// Convert legacy cairo-serde JSON proof (array of hex felt strings) to
/// base64-encoded packed big-endian u32 values.
///
/// Each felt is truncated to the lower 32 bits and packed as big-endian.
pub fn cairo_serde_to_base64(felts: &[String]) -> Result<String, ProofError> {
    let mut packed = Vec::with_capacity(felts.len() * 4);
    for felt_hex in felts {
        let value = u64::from_str_radix(felt_hex.trim_start_matches("0x"), 16)
            .map_err(|e| ProofError::InvalidFelt(format!("{felt_hex}: {e}")))?;
        let u32_val = (value & 0xFFFFFFFF) as u32;
        packed.extend_from_slice(&u32_val.to_be_bytes());
    }
    Ok(encode_proof_base64(&packed))
}

/// Parse proof_facts from a JSON array of hex strings into a Vec of hex strings.
pub fn parse_proof_facts_json(json_str: &str) -> Result<Vec<String>, ProofError> {
    let facts: Vec<String> =
        serde_json::from_str(json_str).map_err(|e| ProofError::InvalidJson(e.to_string()))?;
    Ok(facts)
}

#[derive(Debug, thiserror::Error)]
pub enum ProofError {
    #[error("base64 decode error: {0}")]
    Base64Decode(String),
    #[error("invalid felt value: {0}")]
    InvalidFelt(String),
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_base64() {
        let original = b"hello proof bytes";
        let encoded = encode_proof_base64(original);
        let decoded = decode_proof_base64(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_cairo_serde_to_base64() {
        let felts = vec!["0x1".to_string(), "0xff".to_string(), "0x100000000".to_string()];
        let b64 = cairo_serde_to_base64(&felts).unwrap();
        let bytes = decode_proof_base64(&b64).unwrap();
        // 0x1 -> [0,0,0,1], 0xff -> [0,0,0,255], 0x100000000 -> truncated to [0,0,0,0]
        assert_eq!(bytes, vec![0, 0, 0, 1, 0, 0, 0, 255, 0, 0, 0, 0]);
    }
}
