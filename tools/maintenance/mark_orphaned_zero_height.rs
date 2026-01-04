/// Mark all height=0 transactions as orphaned (height=-1)
/// 
/// These are transactions that were indexed during reorgs or with incomplete data
/// and never made it into the canonical chain. They should be excluded from UTXO sets.

use rocksdb::{DB, WriteBatch, Options, ColumnFamilyDescriptor, IteratorMode};
use std::sync::Arc;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║     MARK ORPHANED TRANSACTIONS (HEIGHT=0)          ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    
    let db_path = "data/pivx";
    println!("📂 Opening database: {}", db_path);
    
    let opts = Options::default();
    let cfs = vec![
        ColumnFamilyDescriptor::new("default", Options::default()),
        ColumnFamilyDescriptor::new("blocks", Options::default()),
        ColumnFamilyDescriptor::new("transactions", Options::default()),
        ColumnFamilyDescriptor::new("chain_metadata", Options::default()),
        ColumnFamilyDescriptor::new("chain_state", Options::default()),
        ColumnFamilyDescriptor::new("addr_index", Options::default()),
        ColumnFamilyDescriptor::new("utxo", Options::default()),
        ColumnFamilyDescriptor::new("utxo_undo", Options::default()),
    ];
    
    let db = Arc::new(DB::open_cf_descriptors(&opts, db_path, cfs)?);
    println!("✅ Database opened\n");
    
    // Step 1: Build canonical chain txid set from 'B' index
    println!("📖 Building canonical chain transaction set...");
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    let mut canonical_txids = HashMap::new();
    let mut block_count = 0;
    
    let iter = db.iterator_cf(&cf_transactions, IteratorMode::Start);
    for item in iter {
        match item {
            Ok((key, value)) => {
                // Only process 'B' prefix entries (block transaction index)
                if !key.is_empty() && key[0] == b'B' {
                    // Key format: 'B' + height(4) + tx_index(8)
                    // Value: txid_hex string
                    if key.len() == 13 {
                        let height = i32::from_le_bytes([key[1], key[2], key[3], key[4]]);
                        if let Ok(txid_hex) = std::str::from_utf8(&value) {
                            if let Ok(txid_bytes) = hex::decode(txid_hex) {
                                // Reverse to internal format for key matching
                                let txid_internal: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
                                canonical_txids.insert(txid_internal, height);
                                
                                if canonical_txids.len() % 100_000 == 0 {
                                    println!("   Found {} canonical transactions...", canonical_txids.len());
                                }
                            }
                        }
                        block_count += 1;
                    }
                }
            }
            Err(_) => break,
        }
    }
    
    println!("✅ Found {} transactions in canonical chain ({} block entries)", 
             canonical_txids.len(), block_count);
    
    // Step 2: Find all height=0 transactions
    println!("\n🔍 Finding height=0 transactions...");
    let mut zero_height_txs = Vec::new();
    let mut tx_scanned = 0;
    
    let iter = db.iterator_cf(&cf_transactions, IteratorMode::Start);
    for item in iter {
        match item {
            Ok((key, value)) => {
                // Only process 't' prefix entries (transaction data)
                if !key.is_empty() && key[0] == b't' && value.len() >= 8 {
                    tx_scanned += 1;
                    
                    let height = i32::from_le_bytes([value[4], value[5], value[6], value[7]]);
                    
                    if height == 0 {
                        let txid = &key[1..];
                        zero_height_txs.push((key.to_vec(), value.to_vec(), txid.to_vec()));
                        
                        if zero_height_txs.len() % 1000 == 0 {
                            println!("   Found {} height=0 transactions (scanned {})...", 
                                     zero_height_txs.len(), tx_scanned);
                        }
                    }
                    
                    if tx_scanned % 500_000 == 0 {
                        println!("   Scanned {} transactions...", tx_scanned);
                    }
                }
            }
            Err(_) => break,
        }
    }
    
    println!("✅ Found {} transactions with height=0 (scanned {} total)\n", 
             zero_height_txs.len(), tx_scanned);
    
    if zero_height_txs.is_empty() {
        println!("🎉 No height=0 transactions found - database is clean!");
        return Ok(());
    }
    
    // Step 3: Mark orphaned transactions (not in canonical chain) as height=-1
    println!("🔧 Marking orphaned transactions as height=-1...");
    let mut batch = WriteBatch::default();
    let mut orphaned_count = 0;
    let mut canonical_count = 0;
    const BATCH_SIZE: usize = 10_000;
    
    for (tx_key, tx_value, txid) in &zero_height_txs {
        if canonical_txids.contains_key(txid) {
            // This transaction IS in canonical chain, should have correct height
            let correct_height = canonical_txids[txid];
            canonical_count += 1;
            
            // Fix it with the correct height
            let version_bytes = &tx_value[0..4];
            let raw_tx = &tx_value[8..];
            
            let mut new_value = version_bytes.to_vec();
            new_value.extend(&correct_height.to_le_bytes());
            new_value.extend(raw_tx);
            
            batch.put_cf(&cf_transactions, tx_key, &new_value);
            
            if canonical_count <= 5 {
                let txid_hex: Vec<u8> = txid.iter().rev().cloned().collect();
                println!("   ✅ Fixed canonical tx {} → height {}", hex::encode(&txid_hex), correct_height);
            }
        } else {
            // This transaction is NOT in canonical chain = orphaned
            let version_bytes = &tx_value[0..4];
            let raw_tx = &tx_value[8..];
            
            let mut new_value = version_bytes.to_vec();
            new_value.extend(&(-1i32).to_le_bytes());  // -1 = orphaned
            new_value.extend(raw_tx);
            
            batch.put_cf(&cf_transactions, tx_key, &new_value);
            orphaned_count += 1;
            
            if orphaned_count <= 5 {
                let txid_hex: Vec<u8> = txid.iter().rev().cloned().collect();
                println!("   ⚠️  Marked orphaned tx {} (not in canonical chain)", hex::encode(&txid_hex));
            }
        }
        
        if (orphaned_count + canonical_count) % BATCH_SIZE == 0 {
            db.write(batch)?;
            batch = WriteBatch::default();
            
            if (orphaned_count + canonical_count) % 50_000 == 0 {
                println!("   Processed {} transactions ({} orphaned, {} fixed)...", 
                         orphaned_count + canonical_count, orphaned_count, canonical_count);
            }
        }
    }
    
    // Write final batch
    if !batch.is_empty() {
        db.write(batch)?;
    }
    
    println!("\n✅ COMPLETE!");
    println!("   Fixed {} canonical transactions with correct heights", canonical_count);
    println!("   Marked {} transactions as orphaned (height=-1)", orphaned_count);
    println!("\n💡 Orphaned transactions are now excluded from:");
    println!("   - UTXO sets (no more 0-confirmation UTXOs)");
    println!("   - Address balances");
    println!("   - Transaction counts");
    
    if orphaned_count > 0 {
        println!("\n⚠️  These orphaned transactions were likely from:");
        println!("   - Chain reorganizations");
        println!("   - Non-canonical blocks");
        println!("   - Incomplete initial sync");
        println!("\n   They are preserved in the database for historical/debugging purposes");
        println!("   but excluded from all user-facing data.");
    }
    
    Ok(())
}
