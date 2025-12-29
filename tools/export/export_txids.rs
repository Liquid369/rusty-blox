use rocksdb::{DB, Options};
use std::error::Error;
use std::fs::File;
use std::io::Write;

fn main() -> Result<(), Box<dyn Error>> {
    let test_address = "DCSAJGThtCnDokqawZehRvVjdms9XLL6J6";
    
    let opts = Options::default();
    let db = DB::open_cf_for_read_only(
        &opts,
        "data/blocks.db",
        vec!["transactions", "addr_index"],
        false
    )?;
    
    let addr_cf = db.cf_handle("addr_index").unwrap();
    
    // Get indexed transaction list
    let addr_txs_key = format!("t{}", test_address);
    let indexed_txids: Vec<Vec<u8>> = if let Some(data) = db.get_cf(addr_cf, addr_txs_key.as_bytes())? {
        data.chunks(32)
            .filter(|c| c.len() == 32)
            .map(|c| c.to_vec())
            .collect()
    } else {
        Vec::new()
    };
    
    // Convert to hex and sort
    // Database stores txids in REVERSED byte order (display format)
    // External explorer uses FORWARD byte order (network/internal format)
    // So we need to reverse the bytes to match external format
    let mut txid_list: Vec<String> = indexed_txids.iter()
        .map(|txid| {
            let mut reversed = txid.clone();
            reversed.reverse();
            reversed.iter().map(|b| format!("{:02x}", b)).collect::<String>()
        })
        .collect();
    
    txid_list.sort();
    
    // Write to file
    let mut file = File::create("/tmp/our_txids.txt")?;
    for txid in &txid_list {
        writeln!(file, "{}", txid)?;
    }
    
    println!("Exported {} transactions to /tmp/our_txids.txt", txid_list.len());
    
    Ok(())
}
