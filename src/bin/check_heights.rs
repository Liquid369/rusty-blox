use rocksdb::{DB, Options};

fn main() {
    // Open with all column families
    let mut opts = Options::default();
    opts.create_if_missing(false);
    
    let cf_names = vec!["blocks", "transactions", "addr_index", "utxo", "chain_metadata", "pubkey", "chain_state"];
    let db = DB::open_cf(&opts, "data/blocks.db", &cf_names)
        .expect("Failed to open DB");
    
    let cf_metadata = db.cf_handle("chain_metadata").expect("chain_metadata CF not found");
    
    // Check heights 0-10
    println!("=== HEIGHT -> BLOCK_HASH MAPPINGS ===");
    for height in 0..10i32 {
        let height_key = height.to_le_bytes().to_vec();
        match db.get_cf(&cf_metadata, &height_key) {
            Ok(Some(value)) => {
                let hash_hex = hex::encode(&value);
                println!("Height {}: {}", height, hash_hex);
            }
            Ok(None) => {
                println!("Height {}: NOT FOUND", height);
            }
            Err(e) => {
                println!("Height {} ERROR: {}", height, e);
            }
        }
    }
    
    println!("\n=== CHECKING GENESIS BLOCK ===");
    // Expected genesis hash (internal byte order)
    let genesis_hash_internal = hex::decode("18f80e68784ac79737df761bc38e0b5c407384b4ef8ed991969b2b481e040000").unwrap();
    
    // Check if genesis has height mapping
    let mut height_key = vec![b'h'];
    height_key.extend_from_slice(&genesis_hash_internal);
    
    match db.get_cf(&cf_metadata, &height_key) {
        Ok(Some(height_bytes)) if height_bytes.len() == 4 => {
            let height = i32::from_le_bytes(height_bytes.as_slice().try_into().unwrap());
            println!("Genesis block height: {}", height);
        }
        Ok(Some(bytes)) => {
            println!("Genesis block has invalid height data: {} bytes", bytes.len());
        }
        Ok(None) => {
            println!("Genesis block height NOT FOUND");
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
    
    // Quick sample check: test some heights
    println!("\n=== SAMPLE HEIGHT CHECKS ===");
    let sample_heights: Vec<i32> = vec![0, 1, 10, 100, 1000, 10000, 50000, 75000, 99000];
    let mut found_count = 0;
    let mut highest_found = 0;
    
    for height in sample_heights {
        let height_key = height.to_le_bytes().to_vec();
        match db.get_cf(&cf_metadata, &height_key) {
            Ok(Some(value)) => {
                let hash_hex = hex::encode(&value);
                println!("✓ Height {}: {}", height, hash_hex);
                found_count += 1;
                if height > highest_found {
                    highest_found = height;
                }
            }
            Ok(None) => {
                println!("✗ Height {}: NOT FOUND", height);
            }
            Err(e) => {
                println!("✗ Height {} ERROR: {}", height, e);
            }
        }
    }
    
    println!("\n=== SUMMARY ===");
    println!("Sample checks: {} / {} found", found_count, 9);
    println!("Highest sampled height found: {}", highest_found);
    
    // Quick estimate by checking sequential heights from 0 until we find a gap
    println!("\n=== FINDING ACTUAL HIGHEST HEIGHT ===");
    let mut height: i32 = 0;
    let mut consecutive_missing = 0;
    let max_missing = 10; // Stop after 10 consecutive missing
    
    loop {
        let height_key = height.to_le_bytes().to_vec();
        match db.get_cf(&cf_metadata, &height_key) {
            Ok(Some(_)) => {
                consecutive_missing = 0;
                height += 1;
            }
            Ok(None) => {
                consecutive_missing += 1;
                if consecutive_missing >= max_missing {
                    println!("Highest height found: {} (stopped after {} missing)", height - max_missing, max_missing);
                    break;
                }
                height += 1;
            }
            Err(e) => {
                println!("Error at height {}: {}", height, e);
                break;
            }
        }
        
        // Also show progress every 10K blocks
        if height % 10000 == 0 && height > 0 {
            println!("  ... checked up to height {}", height);
        }
        
        // Safety limit to prevent infinite loop
        if height > 10_000_000 {
            println!("Reached safety limit at height {}", height);
            break;
        }
    }
}
