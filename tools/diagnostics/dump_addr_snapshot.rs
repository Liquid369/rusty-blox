/// Byte-diff gate for the memory refactor: dump the entire addr_index column
/// family as sorted `hex(key) hex(sha256(value)[..8])` lines. Run it (server
/// DOWN) after each refactor step's re-enrichment; `diff before.txt after.txt`
/// must be EMPTY — any address whose balance / UTXO set / txid list changed
/// produces a differing line, localizing the regression. Opened read-only so it
/// does not need the write lock.
use rocksdb::{DB, Options, ColumnFamilyDescriptor, IteratorMode};
use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/blocks.db".to_string());
    let mut opts = Options::default();
    opts.create_if_missing(false);
    opts.create_missing_column_families(false);
    // RocksDB requires every existing CF to be listed even read-only.
    let cf_names = [
        "default", "blocks", "transactions", "addr_index", "utxo",
        "chain_metadata", "pubkey", "chain_state", "utxo_undo",
    ];
    let cfs: Vec<_> = cf_names
        .iter()
        .map(|n| ColumnFamilyDescriptor::new(*n, Options::default()))
        .collect();
    let db = DB::open_cf_descriptors_read_only(&opts, &db_path, cfs, false)?;

    let cf = db.cf_handle("addr_index").ok_or("addr_index CF not found")?;
    let mut lines: Vec<String> = Vec::new();
    for item in db.iterator_cf(&cf, IteratorMode::Start) {
        let (k, v) = item?;
        let h = Sha256::digest(&v);
        lines.push(format!("{} {}", hex::encode(&k), hex::encode(&h[..8])));
    }
    lines.sort_unstable();
    eprintln!("addr_index entries: {}", lines.len());
    let out = lines.join("\n");
    println!("{out}");
    Ok(())
}
