use rocksdb::{DB, Options};
use rustyblox::config::{load_config, get_db_path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config()?;
    let db_path = get_db_path(&config)?;
    
    let mut cf_names = vec!["default"];
    cf_names.extend(["blocks", "transactions", "addr_index", "utxo", "chain_metadata", "pubkey", "chain_state"]);
    
    let opts = Options::default();
    let db = DB::open_cf_for_read_only(&opts, db_path, &cf_names, false)?;
    
    let cf_metadata = db.cf_handle("chain_metadata").unwrap();
    let cf_state = db.cf_handle("chain_state").unwrap();
    
    println!("=== CHAIN STATE ===");
    if let Some(bytes) = db.get_cf(&cf_state, b"sync_height")? {
        let height = i32::from_le_bytes(bytes.as_slice().try_into()?);
        println!("Sync height: {}", height);
    }
    
    println!("\n=== CHAIN METADATA (sample heights) ===");
    let test_heights = vec![0, 1, 100, 1000, 100000, 1000000, 2800000, 2800554, 5156658];
    
    for h in test_heights {
        let key = (h as i32).to_le_bytes();
        match db.get_cf(&cf_metadata, &key)? {
            Some(hash) => println!("Height {}: {}", h, hex::encode(&hash)),
            None => println!("Height {}: NOT FOUND ❌", h),
        }
    }
    
    println!("\n=== Finding highest metadata height ===");
    let mut highest = 0;
    let mut found_count = 0;
    
    // Sample every 100k blocks to find range
    for h in (0..10_000_000).step_by(100_000) {
        let key = (h as i32).to_le_bytes();
        if db.get_cf(&cf_metadata, &key)?.is_some() {
            highest = h;
            found_count += 1;
        } else if found_count > 0 {
            break; // Found the gap
        }
    }
    
    // Now binary search in the last 100k range
    if highest > 0 {
        for h in (highest..highest + 100_000).rev() {
            let key = (h as i32).to_le_bytes();
            if let Some(hash) = db.get_cf(&cf_metadata, &key)? {
                println!("Highest metadata height: {}", h);
                println!("Hash: {}", hex::encode(&hash));
                break;
            }
        }
    }
    
    // Count total metadata entries
    println!("\n=== Counting metadata entries ===");
    let iter = db.iterator_cf(&cf_metadata, rocksdb::IteratorMode::Start);
    let count: usize = iter.count();
    println!("Total metadata entries: {}", count);
    
    // Check transactions column family
    println!("\n=== TRANSACTIONS CHECK ===");
    let cf_transactions = db.cf_handle("transactions").unwrap();
    let tx_iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    let tx_count: usize = tx_iter.count();
    println!("Total transaction entries: {}", tx_count);
    
    // Count by type
    let mut count_t = 0;  // 't' prefix (transaction data)
    let mut count_b = 0;  // 'B' prefix (block tx index)
    
    let iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    for item in iter.take(10000) {
        match item {
            Ok((key, _)) => {
                if key.len() > 0 {
                    match key[0] {
                        b't' => count_t += 1,
                        b'B' => count_b += 1,
                        _ => {}
                    }
                }
            }
            Err(_) => break,
        }
    }
    
    println!("In first 10,000 entries:");
    println!("  't' prefix (tx data): {}", count_t);
    println!("  'B' prefix (block index): {}", count_b);
    
    // Check first transaction details
    let mut tx_iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    if let Some(Ok((key, value))) = tx_iter.next() {
        println!("First transaction:");
        println!("  Key prefix: {} ({})", key[0] as char, key[0]);
        println!("  Key (hex): {}", hex::encode(&key[..key.len().min(40)]));
        println!("  Value size: {} bytes", value.len());
        if value.len() > 0 {
            println!("  First 100 bytes: {}", hex::encode(&value[..value.len().min(100)]));
        }
    }
    
    // Check blocks column family
    println!("\n=== BLOCKS CHECK ===");
    let cf_blocks = db.cf_handle("blocks").unwrap();
    let blocks_iter = db.iterator_cf(&cf_blocks, rocksdb::IteratorMode::Start);
    let blocks_count: usize = blocks_iter.count();
    println!("Total block entries: {}", blocks_count);
    
    // Check first block details
    let mut blocks_iter = db.iterator_cf(&cf_blocks, rocksdb::IteratorMode::Start);
    if let Some(Ok((key, value))) = blocks_iter.next() {
        println!("First block:");
        println!("  Key (hash): {}", hex::encode(&key));
        println!("  Value size: {} bytes", value.len());
        if value.len() > 0 {
            println!("  First 100 bytes: {}", hex::encode(&value[..value.len().min(100)]));
        }
    }
    
    Ok(())
}
