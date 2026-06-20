//! One-off read-only diagnostic: look up a spent-UTXO undo record while the
//! explorer is running (opens RocksDB as a secondary instance).
//! Usage: db-query spent <txid-hex-display> <vout>

use rustyblox::config::{get_db_path, load_config};
use rustyblox::spent_utxo::SpentUtxo;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 || !matches!(args[1].as_str(), "spent" | "find-spender") {
        eprintln!("usage: db-query <spent|find-spender> <txid-hex> <vout>");
        std::process::exit(2);
    }
    let txid_display = hex::decode(&args[2])?;
    let txid_internal: Vec<u8> = txid_display.iter().rev().cloned().collect();
    let vout: u64 = args[3].parse()?;

    let config = load_config()?;
    let db_path = get_db_path(&config)?;

    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(false);
    let cfs = rocksdb::DB::list_cf(&opts, &db_path)?;
    let secondary = String::from("/tmp/rustyblox-secondary");
    let db = rocksdb::DB::open_cf_as_secondary(&opts, &db_path, &secondary, &cfs)?;
    db.try_catch_up_with_primary()?;
    if args[1] == "find-spender" {
        return find_spender(&db, &txid_internal, vout as u32);
    }
    let cf = db.cf_handle("utxo_undo").ok_or("utxo_undo CF not found")?;

    for (label, t) in [("internal", &txid_internal), ("display", &txid_display)] {
        let mut key = vec![b'S'];
        key.extend_from_slice(t);
        key.extend_from_slice(&vout.to_le_bytes());
        match db.get_cf(&cf, &key)? {
            Some(bytes) => {
                let su = SpentUtxo::from_bytes(&bytes).map_err(|e| e.to_string())?;
                let mut spender_display = su.spending_txid.clone();
                spender_display.reverse();
                println!("[{label} key] FOUND undo record:");
                println!("  value: {}", su.value);
                println!("  created_height: {}", su.created_height);
                println!("  spent_height: {}", su.spent_height);
                println!("  spending_txid: {}", hex::encode(spender_display));
            }
            None => println!("[{label} key] no undo record"),
        }
    }
    Ok(())
}

// (extended below via second binary mode in main — see find_spender)

fn read_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let first = *data.get(*pos)?; *pos += 1;
    Some(match first {
        0..=0xfc => first as u64,
        0xfd => { let v = u16::from_le_bytes(data.get(*pos..*pos+2)?.try_into().ok()?) as u64; *pos += 2; v }
        0xfe => { let v = u32::from_le_bytes(data.get(*pos..*pos+4)?.try_into().ok()?) as u64; *pos += 4; v }
        0xff => { let v = u64::from_le_bytes(data.get(*pos..*pos+8)?.try_into().ok()?); *pos += 8; v }
    })
}

fn find_spender(db: &rocksdb::DB, target_internal: &[u8], target_vout: u32) -> Result<(), Box<dyn std::error::Error>> {
    let cf = db.cf_handle("transactions").ok_or("transactions CF not found")?;
    let mut scanned = 0u64;
    let mut hits = 0;
    let iter = db.iterator_cf(cf, rocksdb::IteratorMode::Start);
    for item in iter {
        let (key, value) = item?;
        if key.first() == Some(&b'B') || value.len() < 12 { continue; }
        scanned += 1;
        let raw = &value[8..];
        // parse: version u16 + type u16, vin count, inputs
        let mut pos = 4usize;
        let vin_count = match read_varint(raw, &mut pos) { Some(v) if v <= 100_000 => v, _ => continue };
        for _ in 0..vin_count {
            if pos + 36 > raw.len() { break; }
            let prevhash = &raw[pos..pos+32];
            let n = u32::from_le_bytes(raw[pos+4+28..pos+36].try_into().unwrap());
            if prevhash == target_internal && n == target_vout {
                let height = i32::from_le_bytes(value[4..8].try_into().unwrap());
                let txid_internal = if key.first() == Some(&b't') { &key[1..] } else { &key[..] };
                let mut disp = txid_internal.to_vec(); disp.reverse();
                println!("SPENDER: txid={} stored_height={}", hex::encode(disp), height);
                hits += 1;
            }
            pos += 36;
            let slen = match read_varint(raw, &mut pos) { Some(v) => v as usize, None => break };
            pos += slen + 4;
        }
    }
    println!("scanned {scanned} txs, {hits} spender(s) found");
    Ok(())
}
