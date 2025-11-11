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
    
    println!("=== TRANSACTION COUNT ===");
    let iter = db_arc.iterator_cf(&cf, IteratorMode::Start);
    let tx_count = iter.count();
    println!("Total transactions: {}", tx_count);
    
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
    let iter = db_arc.iterator_cf(&addr_cf, IteratorMode::Start);
    let addr_count = iter.count();
    println!("\n=== ADDRESS INDEX COUNT ===");
    println!("Total addresses indexed: {}", addr_count);
    
    // Show first 5 addresses
    if addr_count > 0 {
        println!("\n=== FIRST 5 ADDRESSES ===");
        let iter = db_arc.iterator_cf(&addr_cf, IteratorMode::Start);
        for (i, result) in iter.enumerate() {
            if i >= 5 {
                break;
            }
            
            if let Ok((key, value)) = result {
                // Key format: 'a' + address
                let addr = String::from_utf8_lossy(&key[1..]);
                println!("Address {}: {}", i, addr);
                println!("  UTXOs: {} bytes", value.len());
            }
        }
    }
}
