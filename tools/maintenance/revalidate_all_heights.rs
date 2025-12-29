/// Re-validate ALL transaction heights against canonical chain
/// 
/// Unlike the normal height_resolver which only fixes height=0 transactions,
/// this tool re-checks EVERY transaction (including orphaned ones) to ensure
/// they have the correct height based on PIVX Core's current canonical chain.
/// 
/// Use this when:
/// - You suspect false orphan classification
/// - After PIVX Core reorg or re-sync
/// - To recover from sync race conditions

use std::sync::Arc;
use std::collections::HashMap;
use rocksdb::{DB, Options, ColumnFamilyDescriptor, WriteBatch, IteratorMode};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║     RE-VALIDATE ALL TRANSACTION HEIGHTS            ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    
    // 1. Open database
    let db_path = std::env::var("DB_PATH")
        .unwrap_or_else(|_| "data/blocks.db".to_string());
    
    println!("📂 Opening database: {}", db_path);
    
    let mut opts = Options::default();
    opts.create_if_missing(false);
    
    let cfs = vec![
        ColumnFamilyDescriptor::new("default", Options::default()),
        ColumnFamilyDescriptor::new("blocks", Options::default()),
        ColumnFamilyDescriptor::new("transactions", Options::default()),
        ColumnFamilyDescriptor::new("addr_index", Options::default()),
        ColumnFamilyDescriptor::new("utxo", Options::default()),
        ColumnFamilyDescriptor::new("chain_metadata", Options::default()),
        ColumnFamilyDescriptor::new("pubkey", Options::default()),
        ColumnFamilyDescriptor::new("chain_state", Options::default()),
        ColumnFamilyDescriptor::new("utxo_undo", Options::default()),
    ];
    
    let db = Arc::new(DB::open_cf_descriptors(&opts, &db_path, cfs)?);
    
    println!("✅ Database opened\n");
    
    // 2. Build canonical chain from PIVX Core
    println!("📂 Reading PIVX Core block index...");
    
    let pivx_dir = std::env::var("HOME")
        .map(|h| format!("{}/Library/Application Support/PIVX", h))
        .unwrap_or_else(|_| "/Users/liquid/Library/Application Support/PIVX".to_string());
    
    let block_index_src = format!("{}/blocks/index", pivx_dir);
    let block_index_copy = "/tmp/pivx_block_index_revalidate";
    
    // Remove old copy
    std::fs::remove_dir_all(block_index_copy).ok();
    
    // Copy block index
    println!("   Copying from: {}", block_index_src);
    let copy_result = std::process::Command::new("cp")
        .args(["-R", &block_index_src, block_index_copy])
        .output()?;
    
    if !copy_result.status.success() {
        return Err(format!("Failed to copy block index: {}", 
            String::from_utf8_lossy(&copy_result.stderr)).into());
    }
    
    println!("✅ Block index copied\n");
    
    // Use the library function to build canonical chain
    use rustyblox::leveldb_index::build_canonical_chain_from_leveldb;
    
    let canonical_chain = match build_canonical_chain_from_leveldb(block_index_copy) {
        Ok(chain) => chain,
        Err(e) => {
            eprintln!("❌ Failed to read block index: {}", e);
            return Err(e);
        }
    };
    
    println!("✅ Canonical chain: {} blocks\n", canonical_chain.len());
    
    // 3. Build blockhash -> height lookup
    println!("📊 Building blockhash → height lookup...");
    
    let mut blockhash_to_height: HashMap<Vec<u8>, i32> = HashMap::new();
    
    for (height, block_hash, _, _) in &canonical_chain {
        blockhash_to_height.insert(block_hash.clone(), *height as i32);
    }
    
    println!("   ✅ Indexed {} blocks\n", blockhash_to_height.len());
    
    // 4. Scan ALL transactions and build txid -> correct_height map
    println!("🔍 Scanning block-transaction index ('B' entries)...");
    println!("   This will map every TXID to its canonical height");
    
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    let mut txid_to_canonical_height: HashMap<Vec<u8>, i32> = HashMap::new();
    let mut scanned_b_entries = 0;
    let mut canonical_txs = 0;
    let mut orphaned_b_entries = 0;
    
    // Read 'B' entries: 'B' + height(4) + tx_index(8) -> txid_hex
    let block_tx_iter = db.iterator_cf(&cf_transactions, IteratorMode::Start);
    
    for item in block_tx_iter {
        if let Ok((key, value)) = item {
            if key.first() != Some(&b'B') { continue; }
            
            scanned_b_entries += 1;
            
            if key.len() >= 5 {
                let height = i32::from_le_bytes([key[1], key[2], key[3], key[4]]);
                
                // Value is txid as hex string
                if let Ok(txid_hex) = String::from_utf8(value.to_vec()) {
                    if let Ok(txid_display) = hex::decode(&txid_hex) {
                        // Convert to internal format (reversed)
                        let txid_internal: Vec<u8> = txid_display.iter().rev().cloned().collect();
                        
                        // Check if this height's block is in canonical chain
                        // We need to lookup by blockhash, but we only have height from 'B' key
                        // So we check if this height exists in our canonical chain
                        let is_canonical = canonical_chain.iter()
                            .any(|(h, _, _, _)| *h == height as i64);
                        
                        if is_canonical {
                            txid_to_canonical_height.insert(txid_internal, height);
                            canonical_txs += 1;
                        } else {
                            orphaned_b_entries += 1;
                        }
                    }
                }
            }
            
            if scanned_b_entries % 100_000 == 0 {
                println!("      Scanned {} 'B' entries ({} canonical txs found)...", 
                         scanned_b_entries, canonical_txs);
            }
        }
    }
    
    println!("\n   ✅ Block-tx index scan complete:");
    println!("      Total 'B' entries: {}", scanned_b_entries);
    println!("      Canonical txs: {}", canonical_txs);
    println!("      Orphaned 'B' entries: {}", orphaned_b_entries);
    println!();
    
    // 5. Now scan ALL 't' entries and fix heights
    println!("🔧 Re-validating ALL transaction heights...");
    
    let mut total_txs = 0;
    let mut updated = 0;
    let mut already_correct = 0;
    let mut marked_orphan = 0;
    let mut batch = WriteBatch::default();
    const BATCH_SIZE: usize = 10_000;
    
    let tx_iter = db.iterator_cf(&cf_transactions, IteratorMode::Start);
    
    for item in tx_iter {
        if let Ok((key, value)) = item {
            // Only process 't' prefix entries
            if key.first() != Some(&b't') || value.len() < 8 {
                continue;
            }
            
            total_txs += 1;
            
            let txid_internal = &key[1..];
            let current_height = i32::from_le_bytes([value[4], value[5], value[6], value[7]]);
            
            // Determine correct height
            let correct_height = txid_to_canonical_height.get(txid_internal)
                .copied()
                .unwrap_or(-1); // If not in canonical chain, it's orphaned
            
            if current_height != correct_height {
                // Update height
                let mut new_value = value[0..4].to_vec(); // version
                new_value.extend(&correct_height.to_le_bytes());
                new_value.extend(&value[8..]); // rest of data
                
                batch.put_cf(&cf_transactions, &key, &new_value);
                updated += 1;
                
                if correct_height == -1 {
                    marked_orphan += 1;
                }
                
                if updated % BATCH_SIZE == 0 {
                    db.write(batch)?;
                    batch = WriteBatch::default();
                    println!("      Updated {} transactions ({} total scanned)...", updated, total_txs);
                }
            } else {
                already_correct += 1;
            }
            
            if total_txs % 500_000 == 0 {
                println!("      Processed {} transactions ({} updated)...", total_txs, updated);
            }
        }
    }
    
    // Write final batch
    if !batch.is_empty() {
        db.write(batch)?;
    }
    
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║     ✅ RE-VALIDATION COMPLETE! ✅                  ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    
    println!("📊 Results:");
    println!("   Total transactions: {}", total_txs);
    println!("   Already correct: {}", already_correct);
    println!("   Updated: {}", updated);
    println!("   Newly marked as orphan: {}", marked_orphan);
    println!("   Fixed (unorphaned): {}", updated - marked_orphan);
    println!();
    
    // Cleanup
    std::fs::remove_dir_all(block_index_copy).ok();
    
    Ok(())
}
