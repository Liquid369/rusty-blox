/// Transaction Key Helpers
///
/// Byte-level txid helpers for the transactions CF ('t' + 32-byte txid = 33-byte key).
///
/// IMPORTANT — the CF holds records under TWO coexisting key orders: INTERNAL
/// (reversed; written by initial sync) and DISPLAY (written by the live monitor).
/// There is deliberately NO single-key lookup helper here: any serving-path read
/// must go through `crate::api::transactions::read_valid_tx_record`, which probes
/// both orders and refuses body-less stub records. (A display-only `get_transaction`
/// used to live here with zero callers — it was exactly the raw one-key read that
/// caused the stub-shadowing /tx 404 bug, so it was removed.)
///
/// prevout.hash from deserialized transactions is hex-encoded in display order;
/// hex-decoding it yields display-order bytes.

/// Extract txid bytes from a transaction CF key.
///
/// # Arguments
/// * `key` - Full CF key (should start with b't' and be 33 bytes)
///
/// # Returns
/// 32-byte txid in natural/display order, or empty vec if invalid
pub fn txid_from_key(key: &[u8]) -> Vec<u8> {
    if key.len() == 33 && key.first() == Some(&b't') {
        key[1..33].to_vec()
    } else if key.len() == 32 {
        // Key might be just the txid without prefix (legacy)
        key.to_vec()
    } else {
        Vec::new()
    }
}

/// Convert hex-encoded txid string to internal bytes.
///
/// prevout.hash is hex string in display order (big-endian representation).
/// When hex-decoded, we get bytes in natural order ready to use as-is.
///
/// # Arguments
/// * `txid_hex` - Hex-encoded txid string (64 chars)
///
/// # Returns
/// 32-byte txid in natural/display order
pub fn txid_from_hex(txid_hex: &str) -> Result<Vec<u8>, hex::FromHexError> {
    hex::decode(txid_hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_txid_from_key() {
        let mut key = vec![b't'];
        key.extend_from_slice(&[0xabu8; 32]);
        let txid = txid_from_key(&key);
        assert_eq!(txid.len(), 32);
        assert_eq!(txid[0], 0xab);
    }

    #[test]
    fn test_txid_from_hex() {
        let hex = "000000a08ed90e64aeeb720844d0b75e0aac1cb0a13361161edb2edebb5bba5c";
        let bytes = txid_from_hex(hex).unwrap();
        assert_eq!(bytes.len(), 32);
        assert_eq!(hex::encode(&bytes), hex);
    }
}
