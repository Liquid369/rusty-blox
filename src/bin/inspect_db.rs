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
            println!("Database opened successfully");
            println!("\nDatabase Contents:\n");
            
            let cf = match db.cf_handle("blocks") {
                Some(handle) => {
                    println!("Found 'blocks' column family\n");
                    handle
                },
                None => {
                    println!("No 'blocks' column family found");
                    return;
                }
            };
            
            // Iterate through all entries
            let mut count = 0;
            for result in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
                count += 1;
                if let Ok((key, value)) = result {
                    let key_hex = hex::encode(&key);
                    let key_len = key.len();
                    let value_len = value.len();
                    
                    println!("Entry {}: key_len={} key_hex={} value_len={}", 
                        count, key_len, key_hex, value_len);
                    
                    if count >= 20 {
                        println!("\n... (showing first 20 entries)");
                        break;
                    }
                }
            }
            
            if count == 0 {
                println!("No entries found in 'blocks' column family!");
                println!("Database may still be processing files...");
            }
        }
        Err(e) => {
            eprintln!("Failed to open database: {}", e);
        }
    }
}
