use rocksdb::{DB, Options};
use std::error::Error;

fn deserialize_tx_height(data: &[u8]) -> Result<i32, Box<dyn Error>> {
    if data.len() < 8 {
        return Err("Transaction data too short".into());
    }
    
    let height = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    Ok(height)
}

fn main() -> Result<(), Box<dyn Error>> {
    let test_address = "DCSAJGThtCnDokqawZehRvVjdms9XLL6J6";
    
    println!("Counting ALL orphaned transactions for: {}", test_address);
    println!("=============================================================\n");
    
    let opts = Options::default();
    let db = DB::open_cf_for_read_only(
        &opts,
        "data/blocks.db",
        vec!["transactions", "addr_index"],
        false
    )?;
    
    let tx_cf = db.cf_handle("transactions").unwrap();
    let addr_cf = db.cf_handle("addr_index").unwrap();
    
    // Get transaction list for this address
    let addr_txs_key = format!("t{}", test_address);
    let txids = if let Some(data) = db.get_cf(addr_cf, addr_txs_key.as_bytes())? {
        let mut txids = Vec::new();
        for chunk in data.chunks(32) {
            if chunk.len() == 32 {
                txids.push(chunk.to_vec());
            }
        }
        txids
    } else {
        Vec::new()
    };
    
    println!("Total transactions in addr_index: {}", txids.len());
    
    let mut orphan_count = 0;
    let mut valid_count = 0;
    
    for txid in &txids {
        let tx_key = [b"t".as_slice(), &txid[..]].concat();
        
        if let Some(tx_data) = db.get_cf(tx_cf, &tx_key)? {
            let height = deserialize_tx_height(&tx_data)?;
            
            if height == -1 {
                orphan_count += 1;
            } else {
                valid_count += 1;
            }
        }
    }
    
    println!("Valid transactions (height >= 0): {}", valid_count);
    println!("Orphaned transactions (height == -1): {}", orphan_count);
    println!("\nExpected from PIVX Core: 2445 transactions");
    println!("Our valid count: {}", valid_count);
    println!("Difference: {}", 2445 - valid_count);
    
    Ok(())
}
