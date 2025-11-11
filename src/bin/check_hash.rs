use rocksdb::{DB, Options};

fn main() {
    // Open with all column families
    let mut opts = Options::default();
    opts.create_if_missing(false);
    
    let cf_names = vec!["blocks", "transactions", "addr_index", "utxo", "chain_metadata", "pubkey", "chain_state"];
    let db = DB::open_cf(&opts, "data/blocks.db", &cf_names)
        .expect("Failed to open DB");
    
    let cf = db.cf_handle("blocks").expect("blocks CF not found");
    
    // Check height 0 and 1
    for height in 0..5i32 {
        let height_key = height.to_le_bytes().to_vec();
        match db.get_cf(&cf, &height_key) {
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
    
    // Check if expected hash exists
    let expected_hash = hex::decode("000005504fa4a6766e854b2a2c3f21cd276fd7305b84f416241fd4431acbd12d")
        .expect("Failed to parse expected hash");
    println!("\nLooking for expected block 1 hash: {}", hex::encode(&expected_hash));
    
    match db.get_cf(&cf, &expected_hash) {
        Ok(Some(value)) => {
            println!("Found expected hash in DB!");
            println!("Header size: {} bytes", value.len());
        }
        Ok(None) => {
            println!("Expected hash NOT in DB");
        }
        Err(e) => {
            println!("Error checking expected hash: {}", e);
        }
    }
}
