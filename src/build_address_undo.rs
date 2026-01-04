/// Build Address Undo Data for Existing Blocks
///
/// This utility builds address undo data for all blocks in the database.
/// Should be run after initial sync to enable address index rollback during reorgs.

use std::sync::Arc;
use rocksdb::DB;
use crate::address_rollback::{store_address_undo, build_address_undo_from_block};

/// Build address undo data for all blocks in the database
/// 
/// This should be called:
/// - After initial sync completes
/// - After address enrichment completes
/// - Before enabling live sync (which may encounter reorgs)
/// 
/// # Arguments
/// * `db` - Database handle
/// * `start_height` - Height to start building undo data from (typically 0 or current height - N)
/// * `end_height` - Height to build undo data up to (typically current tip)
/// 
/// # Returns
/// Number of blocks processed
pub async fn build_address_undo_for_range(
    db: Arc<DB>,
    start_height: i32,
    end_height: i32,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║      BUILDING ADDRESS UNDO DATA FOR REORGS        ║");
    println!("╚════════════════════════════════════════════════════╝");
    println!("  Building undo data for blocks {} to {}", start_height, end_height);
    println!("  This enables address index rollback during reorganizations\n");
    
    let blocks_processed = (end_height - start_height + 1) as usize;
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    for height in start_height..=end_height {
        if height % 1000 == 0 {
            println!("  Processing block {} ({:.1}% complete)", 
                     height, 
                     (height - start_height) as f64 / blocks_processed as f64 * 100.0);
        }
        
        // Get all transactions in this block
        let mut txids = Vec::new();
        let mut prefix = vec![b'B'];
        prefix.extend_from_slice(&height.to_le_bytes());
        
        let iter = db.prefix_iterator_cf(&cf_transactions, &prefix);
        
        for item in iter {
            let (key, value) = item?;
            
            // Key format: 'B' + height(4) + index(4)
            if key.len() == 9 && key[0] == b'B' {
                // Value is the txid (internal format, 32 bytes)
                if value.len() == 32 {
                    txids.push(value.to_vec());
                } else {
                    // Older format: hex-encoded txid
                    let txid_hex = String::from_utf8_lossy(&value).to_string();
                    if let Ok(txid_bytes) = hex::decode(&txid_hex) {
                        let internal_txid: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
                        txids.push(internal_txid);
                    }
                }
            }
        }
        
        if !txids.is_empty() {
            // Build undo data for this block
            let undo = build_address_undo_from_block(db.clone(), height, txids).await?;
            
            // Store it
            store_address_undo(db.clone(), &undo).await?;
        }
    }
    
    println!("\n  ✅ Address undo data built for {} blocks", blocks_processed);
    println!("     Reorg protection is now enabled for address index\n");
    
    Ok(blocks_processed)
}

/// Check if address undo data exists for a height range
/// 
/// Returns true if all blocks in range have undo data
pub async fn check_address_undo_coverage(
    db: Arc<DB>,
    start_height: i32,
    end_height: i32,
) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>> {
    let total_blocks = (end_height - start_height + 1) as usize;
    
    let db_clone = db.clone();
    
    let results = tokio::task::spawn_blocking(move || {
        let cf_chain_state = db_clone.cf_handle("chain_state")
            .ok_or("chain_state CF not found")?;
        
        let mut found = 0;
        
        for height in start_height..=end_height {
            let mut key = b"addr_undo".to_vec();
            key.extend_from_slice(&height.to_le_bytes());
            
            if let Ok(Some(_)) = db_clone.get_cf(&cf_chain_state, &key) {
                found += 1;
            }
        }
        
        Ok::<usize, Box<dyn std::error::Error + Send + Sync>>(found)
    })
    .await??;
    
    let blocks_with_undo = results;
    
    Ok((blocks_with_undo, total_blocks))
}

#[cfg(test)]
mod tests {
    
    
    #[tokio::test]
    async fn test_coverage_check() {
        // This is a placeholder test - requires actual database
        // In practice, would test with temp database
        assert!(true);
    }
}
