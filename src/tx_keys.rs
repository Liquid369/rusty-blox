/// Transaction Key Helpers
/// 
/// Centralized helpers for consistent transaction key format handling.
/// 
/// KEY FORMAT IN TRANSACTIONS CF:
/// - Prefix: b't' (1 byte)
/// - TXID: 32 bytes in natural/display order (NOT reversed)
/// - Total: 33 bytes
/// 
/// IMPORTANT: prevout.hash from deserialized transactions is hex-encoded in display order.
/// When decoded, the bytes are already in the correct order to use as-is (no reversal needed).

use std::sync::Arc;
use rocksdb::DB;

/// Build a transaction CF key from txid bytes.
/// 
/// # Arguments
/// * `txid_bytes` - 32-byte transaction ID in natural/display order
/// 
/// # Returns
/// 33-byte key: b't' + txid_bytes
pub fn tx_cf_key(txid_bytes: &[u8]) -> Vec<u8> {
    let mut key = vec![b't'];
    key.extend_from_slice(txid_bytes);
    key
}

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

/// Lookup a transaction by txid bytes.
/// 
/// # Arguments
/// * `db` - Database handle
/// * `cf_transactions` - Transaction column family handle
/// * `txid_bytes` - 32-byte txid in natural/display order
/// 
/// # Returns
/// Transaction data if found
pub async fn get_transaction(
    db: Arc<DB>,
    cf_transactions: &rocksdb::ColumnFamily,
    txid_bytes: &[u8],
) -> Option<Vec<u8>> {
    let key = tx_cf_key(txid_bytes);
    db.get_cf(cf_transactions, &key).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_tx_cf_key() {
        let txid = vec![0x12u8; 32];
        let key = tx_cf_key(&txid);
        assert_eq!(key.len(), 33);
        assert_eq!(key[0], b't');
        assert_eq!(&key[1..], &txid[..]);
    }
    
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
