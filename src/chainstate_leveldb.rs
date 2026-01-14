use rusty_leveldb::{DB, Options, LdbIterator};
use std::path::Path;
use std::collections::HashMap;

/// Lightweight chainstate LevelDB reader.
///
/// This module iterates the PIVX `chainstate` LevelDB and returns raw coin
/// entries. Exact CCoins deserialization (amount compression, script
/// compression) is non-trivial and intentionally left as a focused TODO. The
/// functions here provide the LevelDB iteration and surface raw keys/values so
/// downstream modules can implement exact PIVX Core-compatible decoding.

/// A chainstate entry with raw bytes. `outpoint_key` is the LevelDB key (raw
/// bytes after the leading 'C' prefix) and `raw_value` is the associated
/// LevelDB value.
pub type ChainstateRawEntry = (Vec<u8>, Vec<u8>);

/// Iterate the LevelDB at `chainstate_path` and collect all entries whose key
/// starts with 'C' (the UTXO/coin prefix in Bitcoin-derived clients).
pub fn read_chainstate_raw(chainstate_path: &str) -> Result<Vec<ChainstateRawEntry>, Box<dyn std::error::Error>> {
    let opts = Options::default();
    let mut db = DB::open(Path::new(chainstate_path), opts)?;
    let mut iter = db.new_iter()?;

    let mut entries: Vec<ChainstateRawEntry> = Vec::new();

    while let Some((key, value)) = LdbIterator::next(&mut iter) {
        if key.is_empty() { continue; }
        if key[0] != b'C' { continue; }

        // Store key without the prefix (convenience)
        let outpoint_key = key[1..].to_vec();
        entries.push((outpoint_key, value));
    }

    Ok(entries)
}

/// A convenience function to map raw entries to a simple hash map keyed by the
/// hex representation of the raw key. This is useful for quick lookups and for
/// passing into UTXO aggregation logic.
pub fn read_chainstate_map(chainstate_path: &str) -> Result<HashMap<String, Vec<u8>>, Box<dyn std::error::Error>> {
    let raw = read_chainstate_raw(chainstate_path)?;
    let mut map = HashMap::new();
    for (k, v) in raw {
        map.insert(hex::encode(k), v);
    }
    Ok(map)
}
