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
    
    // Count total blocks with heights
    let iter = db.iterator_cf(&cf_metadata, rocksdb::IteratorMode::Start);
    let mut count_heights = 0;
    let mut count_hash_mappings = 0;
    
    for item in iter {
        match item {
            Ok((key, _value)) => {
                if key.len() == 4 && key[0] != b'h' && key[0] != b'p' {
                    count_heights += 1;
                } else if key.len() == 33 && key[0] == b'h' {
                    count_hash_mappings += 1;
                }
            }
            Err(e) => {
                eprintln!("Iterator error: {}", e);
                break;
            }
        }
    }
    
    println!("\nTotal height mappings: {}", count_heights);
    println!("Total hash->height mappings: {}", count_hash_mappings);
}
