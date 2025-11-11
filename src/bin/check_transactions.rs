use rocksdb::{DB, Options, IteratorMode};
use std::sync::Arc;

fn main() {
    let db_path = "data/blocks.db";
    
    let mut db_options = Options::default();
    db_options.create_if_missing(false);
    db_options.create_missing_column_families(true);
    
    let cfs = vec!["blocks", "chain_metadata", "transactions", "addr_index", "utxo", "pubkey", "chain_state"];
    let db = DB::open_cf(&db_options, db_path, cfs.clone())
        .expect("Failed to open database");
    
    let db_arc = Arc::new(db);
    
    // Check transactions CF
    let cf = db_arc.cf_handle("transactions").expect("transactions CF not found");
    
    println!("=== TRANSACTION COUNT (QUICK SAMPLE) ===");
    // Instead of counting ALL transactions (very slow), sample and estimate
    let iter = db_arc.iterator_cf(&cf, IteratorMode::Start);
    let sample_size = 1000; // Count first 1000 to verify DB has data
    let mut count = 0;
    
    for (i, result) in iter.enumerate() {
        if i >= sample_size {
            break;
        }
        if result.is_ok() {
            count += 1;
        }
    }
    
    println!("Sample: {} transactions in first {} entries", count, sample_size);
    if count == sample_size {
        println!("✓ Transaction database appears populated (at least {} transactions)", sample_size);
    } else {
        println!("⚠ Only found {} transactions in sample", count);
    }
    
    // Show first 10 transactions
    println!("\n=== FIRST 10 TRANSACTIONS ===");
    let iter = db_arc.iterator_cf(&cf, IteratorMode::Start);
    for (i, result) in iter.enumerate() {
        if i >= 10 {
            break;
        }
        
        if let Ok((key, value)) = result {
            // Key format: 't' + txid (32 bytes reversed)
            let key_str = hex::encode(&key[1..]); // Skip the 't' prefix
            
            // Value format: block_version (4 bytes LE) + tx_bytes
            if value.len() >= 4 {
                let version_bytes = &value[0..4];
                let version = u32::from_le_bytes([version_bytes[0], version_bytes[1], version_bytes[2], version_bytes[3]]);
                let tx_size = value.len() - 4;
                println!("TX {}: {}", i, key_str);
                println!("  Block version: {}, Transaction size: {} bytes", version, tx_size);
            }
        }
    }
    
    // Check addr_index CF
    let addr_cf = db_arc.cf_handle("addr_index").expect("addr_index CF not found");
    
    println!("\n=== ADDRESS INDEX COUNT (QUICK SAMPLE) ===");
    // Sample first 1000 address entries instead of counting all
    let iter = db_arc.iterator_cf(&addr_cf, IteratorMode::Start);
    let sample_size = 1000;
    let mut count = 0;
    
    for (i, result) in iter.enumerate() {
        if i >= sample_size {
            break;
        }
        if result.is_ok() {
            count += 1;
        }
    }
    
    println!("Sample: {} addresses in first {} entries", count, sample_size);
    if count == sample_size {
        println!("✓ Address index appears populated (at least {} addresses)", sample_size);
    } else if count > 0 {
        println!("⚠ Found {} addresses in sample", count);
    } else {
        println!("✗ No addresses found (fast_sync mode skips UTXO/address tracking)");
    }
    
    // Show first 5 addresses
    println!("\n=== FIRST 5 ADDRESSES ===");
    let iter = db_arc.iterator_cf(&addr_cf, IteratorMode::Start);
    let mut found_any = false;
    
    for (i, result) in iter.enumerate() {
        if i >= 5 {
            break;
        }
        
        if let Ok((key, value)) = result {
            found_any = true;
            // Key format: 'a' + address
            let addr = String::from_utf8_lossy(&key[1..]);
            println!("Address {}: {}", i, addr);
            println!("  UTXOs: {} bytes", value.len());
        }
    }
    
    if !found_any {
        println!("(No addresses - this is expected with fast_sync mode)");
    }
}
