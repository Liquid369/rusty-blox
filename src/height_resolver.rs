/// Height Resolution from PIVX Core Block Index
/// 
/// This module reads PIVX Core's leveldb block index to build the canonical chain,
/// then updates all transactions in our database with correct heights BEFORE enrichment.
/// 
/// This eliminates the need for the "repair" phase and ensures orphaned transactions
/// are identified using Core's canonical chain data.

use rocksdb::{DB, WriteBatch, IteratorMode};
use std::sync::Arc;
use std::collections::{HashSet, HashMap};
use crate::leveldb_index::build_canonical_chain_from_leveldb;
use crate::constants::{HEIGHT_GENESIS, HEIGHT_ORPHAN, HEIGHT_UNRESOLVED};

/// Resolve all transaction heights using PIVX Core's block index
/// 
/// This reads Core's canonical chain and updates ALL transactions in one pass:
/// - Transactions in canonical blocks get their correct height
/// - Transactions in non-canonical blocks get marked as orphaned (HEIGHT_ORPHAN)
/// 
/// Returns (fixed_count, orphaned_count)
pub async fn resolve_heights_from_block_index(
    db: Arc<DB>,
    pivx_data_dir: Option<String>,
) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║   RESOLVING HEIGHTS FROM PIVX CORE BLOCK INDEX    ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    
    // 1. Determine PIVX data directory
    let pivx_dir = pivx_data_dir.unwrap_or_else(|| {
        std::env::var("HOME")
            .map(|h| format!("{}/Library/Application Support/PIVX", h))
            .unwrap_or_else(|_| "/Users/liquid/Library/Application Support/PIVX".to_string())
    });
    
    let block_index_src = format!("{}/blocks/index", pivx_dir);
    let block_index_copy = "/tmp/pivx_block_index_current";
    
    println!("� Copying block index from PIVX Core...");
    println!("   Source: {}", block_index_src);
    println!("   Dest: {}", block_index_copy);
    
    // Remove old copy if exists
    std::fs::remove_dir_all(block_index_copy).ok();
    
    // Copy using cp command
    let copy_result = std::process::Command::new("cp")
        .args(["-R", &block_index_src, block_index_copy])
        .output()?;
    
    if !copy_result.status.success() {
        return Err(format!("Failed to copy block index: {}", 
            String::from_utf8_lossy(&copy_result.stderr)).into());
    }
    
    println!("✅ Block index copied!\n");
    
    // 2. Build canonical chain from copied block index
    println!("📂 Reading PIVX Core block index...");
    println!("   Path: {}", block_index_copy);
    
    let canonical_chain = match build_canonical_chain_from_leveldb(block_index_copy) {
        Ok(chain) => chain,
        Err(e) => {
            eprintln!("❌ Failed to read block index: {}", e);
            eprintln!("   Make sure PIVX Core is installed and has synced");
            return Err(e);
        }
    };
    
    println!("✅ Canonical chain built: {} blocks\n", canonical_chain.len());
    
    // 3. Build lookup structures
    println!("📊 Building canonical block lookup...");
    
    let mut canonical_blocks: HashSet<Vec<u8>> = HashSet::new();
    let mut blockhash_to_height: HashMap<Vec<u8>, i64> = HashMap::new();
    
    for (height, block_hash, _, _) in &canonical_chain {
        canonical_blocks.insert(block_hash.clone());
        blockhash_to_height.insert(block_hash.clone(), *height);
    }
    
    println!("   ✅ Indexed {} canonical blocks", canonical_blocks.len());
    println!("   ⏱️  Height range: 0 → {}\n", canonical_chain.last().map(|(h, _, _, _)| h).unwrap_or(&0));
    
    // 4. Scan all transactions to collect which ones need fixing
    println!("🔧 Scanning transactions to identify which need height updates...");
    
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    let mut total_txs = 0;
    let mut orphaned_count = 0;
    let mut already_correct = 0;
    
    // Collect txids that need fixing:
    // - height=0 or HEIGHT_UNRESOLVED need resolution
    // - Positive heights need validation against canonical chain
    let mut txids_needing_fix: HashSet<Vec<u8>> = HashSet::new();
    let mut txids_to_validate: HashSet<(Vec<u8>, i32)> = HashSet::new();
    
    const BATCH_SIZE: usize = 10_000;
    
    let iter = db.iterator_cf(&cf_transactions, IteratorMode::Start);
    
    for item in iter {
        match item {
            Ok((key, value)) => {
                // Only process 't' prefix entries (transaction data)
                if key.is_empty() || key[0] != b't' { continue; }
                
                total_txs += 1;
                
                // Parse current transaction data
                if value.len() < 8 {
                    continue;
                }
                
                let current_height = i32::from_le_bytes([value[4], value[5], value[6], value[7]]);
                
                if current_height == HEIGHT_GENESIS || current_height == HEIGHT_UNRESOLVED {
                    // Store txid (skip 't' prefix) for later fixing
                    txids_needing_fix.insert(key[1..].to_vec());
                } else if current_height == HEIGHT_ORPHAN {
                    orphaned_count += 1;
                } else if current_height > 0 {
                    // Positive height - need to validate against canonical chain
                    txids_to_validate.insert((key[1..].to_vec(), current_height));
                } else {
                    already_correct += 1;
                }
                
                if total_txs % 100_000 == 0 {
                    println!("   Scanned {} transactions ({} need fixing)...", 
                             total_txs, txids_needing_fix.len());
                }
            }
            Err(e) => {
                eprintln!("⚠️  Error reading transaction: {}", e);
                break;
            }
        }
    }
    
    let fixed_count = txids_needing_fix.len();
    let validate_count = txids_to_validate.len();
    
    println!("\n📈 Transaction scan complete:");
    println!("   Total: {}", total_txs);
    println!("   Need fixing (height=0 or {}): {}", HEIGHT_UNRESOLVED, fixed_count);
    println!("   Need validation (positive heights): {}", validate_count);
    println!("   Already orphaned (height={}): {}", HEIGHT_ORPHAN, orphaned_count);
    println!("   Already have heights: {}", already_correct);
    println!();
    
    // Early exit if nothing to fix or validate
    if fixed_count == 0 && validate_count == 0 {
        println!("✅ All transaction heights already correct!");
        return Ok((0, 0));
    }
    
    // 5. Build a reverse lookup: height -> block_hash
    println!("� Building height lookup table...");
    let mut height_to_blockhash: HashMap<i64, Vec<u8>> = HashMap::new();
    for (height, block_hash, _, _) in &canonical_chain {
        height_to_blockhash.insert(*height, block_hash.clone());
    }
    println!("   ✅ Indexed {} heights\n", height_to_blockhash.len());
    
    // Initialize counters
    let mut updated = 0;
    let mut newly_orphaned = 0;
    
    // 5b. Validate positive heights against canonical chain
    if !txids_to_validate.is_empty() {
        println!("🔍 Validating {} transactions with positive heights...", validate_count);
        let mut validated_batch = WriteBatch::default();
        let mut validated_orphaned = 0;
        
        for (txid_internal, current_height) in txids_to_validate {
            // Check if this height exists in canonical chain
            if !height_to_blockhash.contains_key(&(current_height as i64)) {
                // Height not in canonical chain - mark as orphaned
                let mut tx_key = vec![b't'];
                tx_key.extend_from_slice(&txid_internal);
                
                if let Ok(Some(tx_data)) = db.get_cf(&cf_transactions, &tx_key) {
                    // Update height to HEIGHT_ORPHAN
                    let mut new_value = tx_data[0..4].to_vec(); // version
                    new_value.extend(&HEIGHT_ORPHAN.to_le_bytes());
                    new_value.extend(&tx_data[8..]); // rest of data
                    
                    validated_batch.put_cf(&cf_transactions, &tx_key, &new_value);
                    validated_orphaned += 1;
                    
                    if validated_orphaned % BATCH_SIZE == 0 {
                        db.write(validated_batch)?;
                        validated_batch = WriteBatch::default();
                        println!("      Marked {} as orphaned...", validated_orphaned);
                    }
                }
            }
        }
        
        // Write final validation batch
        if !validated_batch.is_empty() {
            db.write(validated_batch)?;
        }
        
        println!("   ✅ Marked {} transactions with non-canonical heights as orphaned\n", validated_orphaned);
        newly_orphaned += validated_orphaned;
    }
        // 6. Efficiently update ONLY the transactions that need fixing
    println!("🔍 Updating transaction heights from block index...");
    println!("   Processing only {} transactions with height=0 or HEIGHT_UNRESOLVED", fixed_count);
    
    let mut batch = WriteBatch::default();
    let not_found_in_index = 0;
    
    // Read 'B' entries and update ONLY if txid is in our HashSet
    // Format: 'B' + height(4) + tx_index(8) -> txid_hex
    let block_tx_iter = db.iterator_cf(&cf_transactions, IteratorMode::Start);
    
    for item in block_tx_iter {
        match item {
            Ok((key, value)) => {
                if key.is_empty() || key[0] != b'B' { continue; }
                
                // Extract height from key
                if key.len() >= 5 {
                    let height_bytes = [key[1], key[2], key[3], key[4]];
                    let height = i32::from_le_bytes(height_bytes);
                    
                    // Value is txid as hex string
                    if let Ok(txid_hex) = String::from_utf8(value.to_vec()) {
                        if let Ok(txid_display) = hex::decode(&txid_hex) {
                            // Reverse to get internal format
                            let txid_internal: Vec<u8> = txid_display.iter().rev().cloned().collect();
                            
                            // ONLY process if this txid needs fixing
                            if !txids_needing_fix.contains(&txid_internal) {
                                continue;
                            }
                            
                            // Build transaction key
                            let mut tx_key = vec![b't'];
                            tx_key.extend_from_slice(&txid_internal);
                            
                            // Read current transaction
                            if let Ok(Some(tx_data)) = db.get_cf(&cf_transactions, &tx_key) {
                                // Verify it still has height=0 or HEIGHT_UNRESOLVED (shouldn't have changed)
                                let current_height = i32::from_le_bytes([
                                    tx_data[4], tx_data[5], tx_data[6], tx_data[7]
                                ]);
                                
                                if current_height == 0 || current_height == HEIGHT_UNRESOLVED {
                                    // Check if this height is in canonical chain
                                    if let Some(_block_hash) = height_to_blockhash.get(&(height as i64)) {
                                        // Block is in canonical chain - use its height
                                        let mut new_value = tx_data[0..4].to_vec(); // version
                                        new_value.extend(&height.to_le_bytes());
                                        new_value.extend(&tx_data[8..]); // rest of data
                                        
                                        batch.put_cf(&cf_transactions, &tx_key, &new_value);
                                        updated += 1;
                                        
                                        // Remove from set so we don't process it again
                                        txids_needing_fix.remove(&txid_internal);
                                        
                                        if updated % BATCH_SIZE == 0 {
                                            db.write(batch)?;
                                            batch = WriteBatch::default();
                                            println!("      Updated {} transactions ({} remaining)...", 
                                                     updated, txids_needing_fix.len());
                                        }
                                    } else {
                                        // Height not in canonical chain - mark as orphaned
                                        let mut new_value = tx_data[0..4].to_vec(); // version
                                        new_value.extend(&(-1i32).to_le_bytes());
                                        new_value.extend(&tx_data[8..]); // rest of data
                                        
                                        batch.put_cf(&cf_transactions, &tx_key, &new_value);
                                        newly_orphaned += 1;
                                        
                                        // Remove from set
                                        txids_needing_fix.remove(&txid_internal);
                                        
                                        if (updated + newly_orphaned) % BATCH_SIZE == 0 {
                                            db.write(batch)?;
                                            batch = WriteBatch::default();
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠️  Error reading block-tx index: {}", e);
                break;
            }
        }
    }
    
    // Write final batch
    if !batch.is_empty() {
        db.write(batch)?;
    }
    
    // Mark any remaining transactions (not found in block index) as orphaned
    if !txids_needing_fix.is_empty() {
        println!("\n⚠️  {} transactions with height=0 or {} not found in block index", 
                 txids_needing_fix.len(), HEIGHT_UNRESOLVED);
        println!("   These are from orphaned blocks - marking as HEIGHT_ORPHAN...");
        
        let mut orphan_batch = WriteBatch::default();
        let mut marked_orphan = 0;
        
        for txid_internal in txids_needing_fix {
            let mut tx_key = vec![b't'];
            tx_key.extend_from_slice(&txid_internal);
            
            if let Ok(Some(tx_data)) = db.get_cf(&cf_transactions, &tx_key) {
                if tx_data.len() >= 8 {
                    let current_height = i32::from_le_bytes([
                        tx_data[4], tx_data[5], tx_data[6], tx_data[7]
                    ]);
                    
                    // Only mark as orphan if still has height=0 or HEIGHT_UNRESOLVED
                    if current_height == HEIGHT_GENESIS || current_height == HEIGHT_UNRESOLVED {
                        let mut new_value = tx_data[0..4].to_vec(); // version
                        new_value.extend(&HEIGHT_ORPHAN.to_le_bytes());
                        new_value.extend(&tx_data[8..]); // rest of data
                        
                        orphan_batch.put_cf(&cf_transactions, &tx_key, &new_value);
                        marked_orphan += 1;
                        
                        if marked_orphan % BATCH_SIZE == 0 {
                            db.write(orphan_batch)?;
                            orphan_batch = WriteBatch::default();
                            println!("      Marked {} as orphaned...", marked_orphan);
                        }
                    }
                }
            }
        }
        
        if !orphan_batch.is_empty() {
            db.write(orphan_batch)?;
        }
        
        newly_orphaned += marked_orphan;
        println!("   ✅ Marked {} additional transactions as HEIGHT_ORPHAN", marked_orphan);
    }
    
    println!("\n✅ Height resolution complete!");
    println!("   Updated: {} transactions", updated);
    println!("   Orphaned: {} transactions", newly_orphaned);
    println!();
    
    Ok((updated, newly_orphaned))
}

#[cfg(test)]
mod tests {
    
    
    #[tokio::test]
    async fn test_resolve_heights() {
        // This would require a test database
        // For now, just ensure it compiles
        assert!(true);
    }
}
