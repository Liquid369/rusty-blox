/// Analyze Orphaned Transactions
/// 
/// Provides detailed analysis of transactions marked with height=-1

use rocksdb::{DB, IteratorMode};
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n═══════════════════════════════════════════════════════════");
    println!("  ORPHANED TRANSACTION ANALYSIS");
    println!("═══════════════════════════════════════════════════════════\n");
    
    let db_path = std::env::var("DB_PATH")
        .unwrap_or_else(|_| "/Users/liquid/Projects/rusty-blox/data/db".to_string());
    
    println!("Opening database: {}\n", db_path);
    
    let mut cf_opts = rocksdb::Options::default();
    cf_opts.create_if_missing(false);
    
    let cf_names = vec!["default", "blocks", "transactions", "addr_index", "utxo", 
                        "chain_metadata", "pubkey", "chain_state", "utxo_undo"];
    
    let db = DB::open_cf_for_read_only(&cf_opts, &db_path, &cf_names, false)?;
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    println!("Scanning all transactions...");
    
    let mut height_distribution: HashMap<i32, usize> = HashMap::new();
    let mut orphaned_txids: Vec<String> = Vec::new();
    let mut total_txs = 0;
    
    let iter = db.iterator_cf(&cf_transactions, IteratorMode::Start);
    
    for item in iter {
        if let Ok((key, value)) = item {
            // Only process 't' prefix entries (transaction data)
            if key.first() != Some(&b't') || value.len() < 8 {
                continue;
            }
            
            total_txs += 1;
            
            let height = i32::from_le_bytes([value[4], value[5], value[6], value[7]]);
            *height_distribution.entry(height).or_insert(0) += 1;
            
            // Collect first 100 orphaned txids for analysis
            if height == -1 && orphaned_txids.len() < 100 {
                let txid_internal = &key[1..];
                // TXIDs are stored in reversed (internal) format, so reverse them
                // back to display format for human readability
                let txid_display: Vec<u8> = txid_internal.iter().rev().cloned().collect();
                orphaned_txids.push(hex::encode(&txid_display));
            }
            
            if total_txs % 100_000 == 0 {
                println!("  Scanned {} transactions...", total_txs);
            }
        }
    }
    
    println!("\n✅ Scan complete!\n");
    println!("═══════════════════════════════════════════════════════════");
    println!("  RESULTS");
    println!("═══════════════════════════════════════════════════════════\n");
    
    println!("Total transactions: {}", total_txs);
    println!("Unique heights: {}\n", height_distribution.len());
    
    // Special heights
    let orphaned = height_distribution.get(&-1).copied().unwrap_or(0);
    let zero_height = height_distribution.get(&0).copied().unwrap_or(0);
    
    println!("Problematic transactions:");
    println!("  Height -1 (orphaned): {} ({:.2}%)", 
             orphaned, 
             (orphaned as f64 / total_txs as f64) * 100.0);
    println!("  Height 0 (suspicious): {} ({:.2}%)", 
             zero_height,
             (zero_height as f64 / total_txs as f64) * 100.0);
    
    let valid_txs = total_txs - orphaned - zero_height;
    println!("\nValid transactions: {} ({:.2}%)\n", 
             valid_txs,
             (valid_txs as f64 / total_txs as f64) * 100.0);
    
    // Show height distribution
    let mut heights: Vec<_> = height_distribution.iter().collect();
    heights.sort_by_key(|(h, _)| *h);
    
    println!("Height distribution (first 10 and last 10):");
    for (height, count) in heights.iter().take(10) {
        println!("  Height {:>8}: {} transactions", height, count);
    }
    if heights.len() > 20 {
        println!("  ...");
        for (height, count) in heights.iter().rev().take(10).rev() {
            println!("  Height {:>8}: {} transactions", height, count);
        }
    }
    
    // Sample orphaned transactions
    if !orphaned_txids.is_empty() {
        println!("\n═══════════════════════════════════════════════════════════");
        println!("  SAMPLE ORPHANED TRANSACTIONS");
        println!("═══════════════════════════════════════════════════════════\n");
        
        println!("First 10 orphaned TXIDs (height=-1):");
        for (i, txid) in orphaned_txids.iter().take(10).enumerate() {
            println!("  {}. {}", i + 1, txid);
        }
        
        println!("\nYou can check these on a block explorer to verify if they");
        println!("should truly be orphaned or are in the canonical chain.");
    }
    
    // Assessment
    println!("\n═══════════════════════════════════════════════════════════");
    println!("  ASSESSMENT");
    println!("═══════════════════════════════════════════════════════════\n");
    
    if orphaned > 1000 && orphaned > (total_txs / 100) {
        println!("⚠️  HIGH orphan count detected!");
        println!("   {} orphaned transactions is unusually high.", orphaned);
        println!("   This suggests:");
        println!("   1. False orphan classification during sync");
        println!("   2. Stale leveldb copy used for canonical chain");
        println!("   3. Race condition in height assignment");
        println!();
        println!("   RECOMMENDED ACTION:");
        println!("   - Review height resolution logic");
        println!("   - Apply Fix #2 (sync refactor) from FINAL_DELIVERABLES.md");
    } else {
        println!("✅ Orphan count appears reasonable");
        println!("   {} orphaned transactions ({:.2}%) is within expected range",
                 orphaned,
                 (orphaned as f64 / total_txs as f64) * 100.0);
        println!("   for a blockchain with occasional forks.");
    }
    
    if zero_height > 0 && zero_height != 1 {
        println!("\n⚠️  Found {} transactions with height=0", zero_height);
        println!("   (Expected only 1 for genesis block coinbase)");
        println!("   This indicates incomplete height resolution.");
    }
    
    Ok(())
}
