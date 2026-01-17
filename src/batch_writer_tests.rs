//! Regression tests for BatchWriter read-your-writes overlay
//! 
//! These tests verify the fix for the critical race condition where multiple
//! transactions updating the same address key within one batch would lose updates.

#[cfg(test)]
mod tests {
    use super::super::batch_writer::BatchWriter;
    use std::sync::Arc;
    use rocksdb::{DB, Options};
    use tempfile::TempDir;

    /// Helper: Create a temporary RocksDB instance for testing
    fn create_test_db() -> (Arc<DB>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        
        let cf_names = vec!["default", "addr_index", "pubkey", "transactions"];
        let db = DB::open_cf(&opts, temp_dir.path(), &cf_names).unwrap();
        
        (Arc::new(db), temp_dir)
    }

    /// Helper: Serialize a simple UTXO list for testing
    fn serialize_utxo_list(utxos: &[(Vec<u8>, u64)]) -> Vec<u8> {
        let mut result = Vec::new();
        for (txid, vout) in utxos {
            result.extend_from_slice(txid);
            result.extend_from_slice(&vout.to_le_bytes());
        }
        result
    }

    /// Helper: Deserialize UTXO list
    fn deserialize_utxo_list(data: &[u8]) -> Vec<(Vec<u8>, u64)> {
        let mut utxos = Vec::new();
        let mut i = 0;
        while i + 40 <= data.len() {
            let mut txid = vec![0u8; 32];
            txid.copy_from_slice(&data[i..i+32]);
            let vout = u64::from_le_bytes([
                data[i+32], data[i+33], data[i+34], data[i+35],
                data[i+36], data[i+37], data[i+38], data[i+39],
            ]);
            utxos.push((txid, vout));
            i += 40;
        }
        utxos
    }

    #[tokio::test]
    async fn test_batch_writer_single_update() {
        let (db, _temp_dir) = create_test_db();
        let mut batch = BatchWriter::new(db.clone(), 10000);
        
        let key = b"test_address_1".to_vec();
        let txid1 = vec![1u8; 32];
        
        // First update - should work
        let utxos = vec![(txid1.clone(), 0u64)];
        batch.put("addr_index", key.clone(), serialize_utxo_list(&utxos));
        
        // Flush and verify
        batch.flush().await.unwrap();
        
        let cf = db.cf_handle("addr_index").unwrap();
        let stored = db.get_cf(&cf, &key).unwrap().unwrap();
        let decoded = deserialize_utxo_list(&stored);
        
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].0, txid1);
        assert_eq!(decoded[0].1, 0);
    }

    #[tokio::test]
    async fn test_batch_writer_two_updates_same_key() {
        // This is the CRITICAL test for the race condition fix
        let (db, _temp_dir) = create_test_db();
        let mut batch = BatchWriter::new(db.clone(), 10000);
        
        let key = b"test_address_2".to_vec();
        let txid1 = vec![1u8; 32];
        let txid2 = vec![2u8; 32];
        
        // Simulate two transactions in the same block updating the same address
        
        // Transaction 1: Add UTXO1
        let existing = batch.get("addr_index", &key).await.unwrap();
        let mut utxos = match existing {
            Some(data) => deserialize_utxo_list(&data),
            None => Vec::new(),
        };
        utxos.push((txid1.clone(), 0u64));
        batch.put("addr_index", key.clone(), serialize_utxo_list(&utxos));
        
        // Transaction 2: Add UTXO2 (WITHOUT flush between - same batch!)
        let existing = batch.get("addr_index", &key).await.unwrap();
        let mut utxos = match existing {
            Some(data) => deserialize_utxo_list(&data),
            None => Vec::new(),
        };
        utxos.push((txid2.clone(), 1u64));
        batch.put("addr_index", key.clone(), serialize_utxo_list(&utxos));
        
        // Flush once
        batch.flush().await.unwrap();
        
        // Verify BOTH UTXOs are present (not just the last one!)
        let cf = db.cf_handle("addr_index").unwrap();
        let stored = db.get_cf(&cf, &key).unwrap().unwrap();
        let decoded = deserialize_utxo_list(&stored);
        
        assert_eq!(decoded.len(), 2, "Both UTXOs should be stored");
        assert_eq!(decoded[0].0, txid1, "First UTXO should be present");
        assert_eq!(decoded[0].1, 0);
        assert_eq!(decoded[1].0, txid2, "Second UTXO should be present");
        assert_eq!(decoded[1].1, 1);
    }

    #[tokio::test]
    async fn test_batch_writer_three_updates_same_key() {
        // Test with 3 transactions to the same address in one batch
        let (db, _temp_dir) = create_test_db();
        let mut batch = BatchWriter::new(db.clone(), 10000);
        
        let key = b"test_address_3".to_vec();
        let txid1 = vec![1u8; 32];
        let txid2 = vec![2u8; 32];
        let txid3 = vec![3u8; 32];
        
        // Transaction 1
        let existing = batch.get("addr_index", &key).await.unwrap();
        let mut utxos = match existing {
            Some(data) => deserialize_utxo_list(&data),
            None => Vec::new(),
        };
        utxos.push((txid1.clone(), 0u64));
        batch.put("addr_index", key.clone(), serialize_utxo_list(&utxos));
        
        // Transaction 2
        let existing = batch.get("addr_index", &key).await.unwrap();
        let mut utxos = match existing {
            Some(data) => deserialize_utxo_list(&data),
            None => Vec::new(),
        };
        utxos.push((txid2.clone(), 1u64));
        batch.put("addr_index", key.clone(), serialize_utxo_list(&utxos));
        
        // Transaction 3
        let existing = batch.get("addr_index", &key).await.unwrap();
        let mut utxos = match existing {
            Some(data) => deserialize_utxo_list(&data),
            None => Vec::new(),
        };
        utxos.push((txid3.clone(), 2u64));
        batch.put("addr_index", key.clone(), serialize_utxo_list(&utxos));
        
        // Flush
        batch.flush().await.unwrap();
        
        // Verify all 3 UTXOs
        let cf = db.cf_handle("addr_index").unwrap();
        let stored = db.get_cf(&cf, &key).unwrap().unwrap();
        let decoded = deserialize_utxo_list(&stored);
        
        assert_eq!(decoded.len(), 3, "All three UTXOs should be stored");
        assert_eq!(decoded[0].0, txid1);
        assert_eq!(decoded[1].0, txid2);
        assert_eq!(decoded[2].0, txid3);
    }

    #[tokio::test]
    async fn test_batch_writer_different_keys_no_conflict() {
        // Verify that updates to different keys don't interfere
        let (db, _temp_dir) = create_test_db();
        let mut batch = BatchWriter::new(db.clone(), 10000);
        
        let key1 = b"address_A".to_vec();
        let key2 = b"address_B".to_vec();
        let txid1 = vec![1u8; 32];
        let txid2 = vec![2u8; 32];
        
        // Update key1
        batch.put("addr_index", key1.clone(), serialize_utxo_list(&vec![(txid1.clone(), 0)]));
        
        // Update key2
        batch.put("addr_index", key2.clone(), serialize_utxo_list(&vec![(txid2.clone(), 0)]));
        
        batch.flush().await.unwrap();
        
        // Verify both keys stored correctly
        let cf = db.cf_handle("addr_index").unwrap();
        
        let stored1 = db.get_cf(&cf, &key1).unwrap().unwrap();
        let decoded1 = deserialize_utxo_list(&stored1);
        assert_eq!(decoded1.len(), 1);
        assert_eq!(decoded1[0].0, txid1);
        
        let stored2 = db.get_cf(&cf, &key2).unwrap().unwrap();
        let decoded2 = deserialize_utxo_list(&stored2);
        assert_eq!(decoded2.len(), 1);
        assert_eq!(decoded2[0].0, txid2);
    }

    #[tokio::test]
    async fn test_batch_writer_overlay_clears_after_flush() {
        // Verify overlay is cleared after flush
        let (db, _temp_dir) = create_test_db();
        let mut batch = BatchWriter::new(db.clone(), 10000);
        
        let key = b"test_address_4".to_vec();
        let txid1 = vec![1u8; 32];
        
        // First batch
        batch.put("addr_index", key.clone(), serialize_utxo_list(&vec![(txid1.clone(), 0)]));
        batch.flush().await.unwrap();
        
        // Second batch should read from DB, not stale overlay
        let txid2 = vec![2u8; 32];
        let existing = batch.get("addr_index", &key).await.unwrap();
        let mut utxos = match existing {
            Some(data) => deserialize_utxo_list(&data),
            None => Vec::new(),
        };
        utxos.push((txid2.clone(), 1));
        batch.put("addr_index", key.clone(), serialize_utxo_list(&utxos));
        batch.flush().await.unwrap();
        
        // Verify both UTXOs from separate batches
        let cf = db.cf_handle("addr_index").unwrap();
        let stored = db.get_cf(&cf, &key).unwrap().unwrap();
        let decoded = deserialize_utxo_list(&stored);
        
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].0, txid1);
        assert_eq!(decoded[1].0, txid2);
    }

    #[tokio::test]
    async fn test_batch_writer_multiple_column_families() {
        // Verify overlay works correctly across different CFs
        let (db, _temp_dir) = create_test_db();
        let mut batch = BatchWriter::new(db.clone(), 10000);
        
        let key = b"shared_key".to_vec();
        let value1 = b"addr_value".to_vec();
        let value2 = b"pubkey_value".to_vec();
        
        // Write same key to different CFs
        batch.put("addr_index", key.clone(), value1.clone());
        batch.put("pubkey", key.clone(), value2.clone());
        
        // Read from overlay
        let read1 = batch.get("addr_index", &key).await.unwrap().unwrap();
        let read2 = batch.get("pubkey", &key).await.unwrap().unwrap();
        
        assert_eq!(read1, value1, "Should read correct value from addr_index overlay");
        assert_eq!(read2, value2, "Should read correct value from pubkey overlay");
        
        batch.flush().await.unwrap();
        
        // Verify in DB
        let cf1 = db.cf_handle("addr_index").unwrap();
        let cf2 = db.cf_handle("pubkey").unwrap();
        
        let stored1 = db.get_cf(&cf1, &key).unwrap().unwrap();
        let stored2 = db.get_cf(&cf2, &key).unwrap().unwrap();
        
        assert_eq!(stored1, value1);
        assert_eq!(stored2, value2);
    }
}
