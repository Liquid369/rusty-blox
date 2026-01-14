use rocksdb::DB;
use hex;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Open database
    let db_path = "data/blocks.db";
    let cf_names = vec!["default", "blocks", "transactions", "addr_index", "utxo", 
                        "chain_metadata", "pubkey", "chain_state", "utxo_undo"];
    
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(false);
    
    let db = DB::open_cf_for_read_only(&opts, &db_path, &cf_names, false)?;
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    // Get first transaction
    let iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    
    for (i, item) in iter.enumerate() {
        if i >= 5 { break; }
        
        let (key, _value) = item?;
        
        // Skip non-transaction entries
        if key.first() != Some(&b't') || key.len() != 33 {
            continue;
        }
        
        // Extract TXID from key (skip 't' prefix)
        let txid_from_db_key = &key[1..33];
        let txid_hex = hex::encode(txid_from_db_key);
        
        // Also show what it looks like reversed
        let txid_reversed: Vec<u8> = txid_from_db_key.iter().rev().cloned().collect();
        let txid_reversed_hex = hex::encode(&txid_reversed);
        
        println!("Transaction {}:", i + 1);
        println!("  Key bytes (from DB): {}", &txid_hex[..32]);
        println!("  Reversed:            {}", &txid_reversed_hex[..32]);
        println!();
    }
    
    Ok(())
}
