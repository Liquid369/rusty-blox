//! One-shot repair: reset the live-analytics watermark back before the last N
//! calendar days and delete those days' (possibly partial) blobs, so Lane I
//! rebuilds them completely on the next tick. RUN WITH THE BACKEND STOPPED.
//!
//! Usage: reset-live-watermark [days]   (default 2 = redo the tip day + the day
//! before it)
use rustyblox::config::{get_db_path, load_config};
use rustyblox::enrich_addresses::unix_to_date;
use std::collections::BTreeSet;

fn header_date(db: &rocksdb::DB, height: i32) -> Option<String> {
    let cf_meta = db.cf_handle("chain_metadata")?;
    let cf_blocks = db.cf_handle("blocks")?;
    let display = db.get_cf(&cf_meta, height.to_le_bytes()).ok()??;
    let internal: Vec<u8> = display.iter().rev().cloned().collect();
    let header = db.get_cf(&cf_blocks, &internal).ok()??;
    if header.len() >= 72 {
        let t = u32::from_le_bytes(header[68..72].try_into().ok()?);
        if t != 0 {
            return Some(unix_to_date(t as u64));
        }
    }
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let days: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let config = load_config()?;
    let db_path = get_db_path(&config)?;
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(false);
    let cfs = rocksdb::DB::list_cf(&opts, &db_path)?;
    let db = rocksdb::DB::open_cf(&opts, &db_path, &cfs)?;
    let cf = db.cf_handle("chain_state").ok_or("no chain_state")?;

    let tip = db
        .get_cf(&cf, b"sync_height")?
        .filter(|v| v.len() == 4)
        .map(|v| i32::from_le_bytes([v[0], v[1], v[2], v[3]]))
        .ok_or("no sync_height")?;

    // Walk back from the tip collecting distinct calendar dates; stop once we hit a
    // (days+1)-th date. `h_first` is the lowest height still within the last `days`
    // dates — its predecessor becomes the new watermark.
    let mut redo: BTreeSet<String> = BTreeSet::new();
    let mut h_first = tip;
    let mut h = tip;
    while h > 0 {
        match header_date(&db, h) {
            Some(d) => {
                if !redo.contains(&d) && redo.len() == days {
                    break; // d is an older date beyond the window
                }
                redo.insert(d);
                h_first = h;
            }
            None => {}
        }
        h -= 1;
    }
    if redo.is_empty() {
        println!("no dated blocks found near the tip; nothing to do");
        return Ok(());
    }

    println!(
        "tip={tip}; redoing {} day(s): {:?}; h_first={h_first}",
        redo.len(),
        redo
    );
    println!("watermark resets to {}", h_first - 1);

    // Drop the affected dates from the API index, delete their blobs + side keys,
    // and reset the watermark — all atomically.
    let mut idx: Vec<String> = db
        .get_cf(&cf, b"analytics_tx_days")?
        .and_then(|b| bincode::deserialize(&b).ok())
        .unwrap_or_default();
    idx.retain(|d| !redo.contains(d));

    let mut batch = rocksdb::WriteBatch::default();
    batch.put_cf(&cf, b"analytics_tx_days", bincode::serialize(&idx)?);
    for d in &redo {
        for pfx in [
            "analytics_tx_day:",
            "live_day_diffsum:",
            "live_day_intervals:",
        ] {
            let mut k = pfx.as_bytes().to_vec();
            k.extend_from_slice(d.as_bytes());
            batch.delete_cf(&cf, &k);
        }
    }
    batch.put_cf(&cf, b"analytics_live_height", (h_first - 1).to_le_bytes());
    db.write(batch)?;

    println!(
        "done — restart the backend; Lane I will rebuild those days from {}.",
        h_first
    );
    Ok(())
}
