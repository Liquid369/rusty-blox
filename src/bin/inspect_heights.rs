use rocksdb::{DB, ColumnFamilyDescriptor, Options};

fn main() {
    let db_path = "./data/blocks.db";
    
    // Open with all column families
    let cf_names = vec!["blocks", "transactions", "addr_index", "utxo", "chain_metadata", "pubkey", "chain_state"];
    let mut cf_descriptors = vec![];
    for cf_name in &cf_names {
        cf_descriptors.push(ColumnFamilyDescriptor::new(*cf_name, Options::default()));
    }
    
    match DB::open_cf_descriptors(&Options::default(), db_path, cf_descriptors) {
        Ok(db) => {
            println!("Database opened successfully\n");
            
            let cf = match db.cf_handle("blocks") {
                Some(handle) => handle,
                None => {
                    println!("No 'blocks' column family found");
                    return;
                }
            };
            
            println!("Height → Hash Mappings:\n");
            
            // Look for 4-byte keys (height keys)
            let mut height_count = 0;
            for result in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
                if let Ok((key, value)) = result {
                    if key.len() == 4 {
                        // This is a height key
                        let height_bytes: [u8; 4] = key[..4].try_into().unwrap();
                        let height = u32::from_le_bytes(height_bytes);
                        let hash_hex = hex::encode(&value);
                        println!("Height {}: {}", height, hash_hex);
                        height_count += 1;
                        
                        if height_count >= 50 {
                            println!("\n... (showing first 50 heights)");
                            break;
                        }
                    }
                }
            }
            
            if height_count == 0 {
                println!("No height keys found (all entries are block hashes)\n");
            } else {
                println!("\nFound {} height entries", height_count);
            }
        }
        Err(e) => {
            eprintln!("Failed to open database: {}", e);
        }
    }
}
