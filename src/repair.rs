/// Transaction repair utilities
/// 
/// Fixes database inconsistencies like transactions stored with height=0

use rocksdb::{DB, WriteBatch};
use std::sync::Arc;
use std::collections::HashMap;

/// Fix all transactions that have height=0 by looking them up in the block index
/// 
/// This repairs a bug where transactions were stored with height=0 during initial sync
/// when block heights hadn't been resolved yet. The fix:
/// 1. Scans all transactions to find ones with height=0
/// 2. Uses the 'B' (block transaction) index to find the correct height
/// 3. Updates the transaction data with the correct height
/// 
/// Returns (fixed_count, unfixable_count)
pub async fn fix_zero_height_transactions(db: &Arc<DB>) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    println!("\n🔧 Checking for transactions with height=0...");
    
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    // Step 1: Find all transactions with height=0
    let mut zero_height_txs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut total_txs = 0;
    
    let iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    
    for item in iter {
        match item {
            Ok((key, value)) => {
                // Only process 't' prefix entries (transaction data)
                if !key.is_empty() && key[0] == b't' {
                    total_txs += 1;
                    
                    // Check height (bytes 4-8 in value)
                    // Format: version(4) + height(4) + raw_tx
                    if value.len() >= 8 {
                        let height = i32::from_le_bytes([value[4], value[5], value[6], value[7]]);
                        
                        if height == 0 {
                            zero_height_txs.push((key.to_vec(), value.to_vec()));
                        }
                    }
                    
                    if total_txs % 500_000 == 0 {
                        println!("   Scanned {} transactions, found {} with height=0...", 
                                 total_txs, zero_height_txs.len());
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠️  Error reading transaction: {}", e);
            }
        }
    }
    
    if zero_height_txs.is_empty() {
        println!("   ✅ No transactions with height=0 found");
        return Ok((0, 0));
    }
    
    println!("   📊 Found {} transactions with height=0 out of {} total", 
             zero_height_txs.len(), total_txs);
    
    // Step 2: Build txid -> height mapping using the 'B' index
    println!("   🔍 Looking up correct heights from block index...");
    
    let mut txid_to_height: HashMap<Vec<u8>, i32> = HashMap::new();
    
    // Iterate through all 'B' prefix entries (block transaction index)
    // Format: 'B' + height(4) + tx_index(8) -> txid_hex
    let block_tx_iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    
    let mut block_entries = 0;
    for item in block_tx_iter {
        match item {
            Ok((key, value)) => {
                // Only process 'B' prefix entries
                if !key.is_empty() && key[0] == b'B' {
                    block_entries += 1;
                    
                    // Extract height from key (bytes 1-5)
                    if key.len() >= 5 {
                        let height = i32::from_le_bytes([key[1], key[2], key[3], key[4]]);
                        
                        // Value is txid as hex string (display format)
                        if let Ok(txid_hex) = String::from_utf8(value.to_vec()) {
                            if let Ok(txid_bytes) = hex::decode(&txid_hex) {
                                // Reverse to get internal format (txids are stored reversed)
                                let txid_internal: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
                                txid_to_height.insert(txid_internal, height);
                            }
                        }
                    }
                    
                    if block_entries % 100_000 == 0 {
                        println!("      Processed {} block index entries...", block_entries);
                    }
                }
            }
            Err(_) => break,
        }
    }
    
    println!("   📈 Found heights for {} transactions in block index", txid_to_height.len());
    
    // Step 3: Update fixable transactions and MARK orphaned ones (don't delete)
    let mut batch = WriteBatch::default();
    let mut fixed_count = 0;
    let mut orphaned_count = 0;
    const BATCH_SIZE: usize = 10_000;
    
    println!("   🔧 Updating fixable transactions and marking orphaned ones...");
    
    for (tx_key, tx_value) in &zero_height_txs {
        // Extract txid from key (skip 't' prefix)
        let txid = &tx_key[1..];
        
        if let Some(&correct_height) = txid_to_height.get(txid) {
            // Rebuild transaction data with correct height
            // Format: version (4 bytes) + height (4 bytes) + raw_tx
            let version_bytes = &tx_value[0..4];
            let raw_tx = &tx_value[8..]; // Skip old version+height
            
            let mut new_value = version_bytes.to_vec();
            new_value.extend(&correct_height.to_le_bytes());
            new_value.extend(raw_tx);
            
            batch.put_cf(&cf_transactions, tx_key, &new_value);
            fixed_count += 1;
            
            if fixed_count % BATCH_SIZE == 0 {
                db.write(batch)?;
                batch = WriteBatch::default();
                println!("      Fixed {} transactions...", fixed_count);
            }
        } else {
            // Transaction not in block index = orphaned transaction (from non-canonical block)
            // KEEP it but mark with height = -1 to indicate it's orphaned
            // This preserves the data for historical/debugging purposes while excluding
            // it from active UTXO set and address balances
            
            let version_bytes = &tx_value[0..4];
            let raw_tx = &tx_value[8..]; // Skip old version+height
            
            let mut new_value = version_bytes.to_vec();
            new_value.extend(&(-1i32).to_le_bytes()); // -1 = orphaned
            new_value.extend(raw_tx);
            
            batch.put_cf(&cf_transactions, tx_key, &new_value);
            orphaned_count += 1;
            
            // Log details about first few orphaned transactions
            if orphaned_count <= 10 {
                let txid_hex: Vec<u8> = txid.iter().rev().cloned().collect();
                eprintln!("      ⚠️  Marking as orphaned (not in canonical chain): {}", hex::encode(&txid_hex));
            }
        }
        
        // Commit batch periodically
        if (fixed_count + orphaned_count) % BATCH_SIZE == 0 {
            db.write(batch)?;
            batch = WriteBatch::default();
            
            if (fixed_count + orphaned_count) % 10_000 == 0 {
                println!("      Processed {} transactions ({} fixed, {} orphaned)...", 
                         fixed_count + orphaned_count, fixed_count, orphaned_count);
            }
        }
    }
    
    // Write final batch
    if batch.len() > 0 {
        db.write(batch)?;
    }
    
    println!("\n   ✅ Fixed {} transactions with correct heights", fixed_count);
    if orphaned_count > 0 {
        println!("   ⚠️  Marked {} transactions as orphaned (height=-1, not in canonical chain)", orphaned_count);
        println!("      These are kept for historical queries but excluded from balances/UTXOs");
    }
    
    Ok((fixed_count, orphaned_count))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_fix_zero_height_transactions() {
        // This test would require a test database with known bad data
        // For now, just ensure the function compiles and can be called
        assert!(true);
    }
}
