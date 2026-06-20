//! One-shot migration: build the persistent orphan index (orphanseen:/orphancount:)
//! from the existing blocks CF + tail_blocks, then rewrite every analytics_tx_day
//! blob's orphan_blocks from the persistent count. RUN ONCE WITH THE BACKEND
//! STOPPED. After this, the live tail-only path maintains the index — no resync.
use rustyblox::config::{load_config, get_db_path};
use rustyblox::enrich_addresses::{mark_orphans, orphan_count, TxDayAgg};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config()?;
    let db_path = get_db_path(&config)?;
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(false);
    let cfs = rocksdb::DB::list_cf(&opts, &db_path)?;
    let db = std::sync::Arc::new(rocksdb::DB::open_cf(&opts, &db_path, &cfs)?);
    let cf_state = db.cf_handle("chain_state").ok_or("no chain_state")?;
    let cf_meta = db.cf_handle("chain_metadata").ok_or("no chain_metadata")?;
    let cf_blocks = db.cf_handle("blocks").ok_or("no blocks")?;

    let tip = db
        .get_cf(&cf_state, b"sync_height")?
        .filter(|v| v.len() == 4)
        .map(|v| i32::from_le_bytes([v[0], v[1], v[2], v[3]]))
        .ok_or("no sync_height")?;
    let tip_time = db
        .get_cf(&cf_meta, tip.to_le_bytes())?
        .and_then(|d| {
            let internal: Vec<u8> = d.iter().rev().cloned().collect();
            db.get_cf(&cf_blocks, &internal).ok().flatten()
        })
        .filter(|h| h.len() >= 72)
        .map(|h| u32::from_le_bytes(h[68..72].try_into().unwrap()))
        .unwrap_or(0);
    println!("tip={tip} tip_time={tip_time} — building orphan index (full scan)…");

    mark_orphans(&db, tip, tip_time, false)?;
    println!("orphan index built. rewriting analytics_tx_day orphan_blocks…");

    // Rewrite orphan_blocks in every day blob from the persistent count.
    let mut updated = 0u64;
    let dates: Vec<String> = db
        .get_cf(&cf_state, b"analytics_tx_days")?
        .and_then(|b| bincode::deserialize(&b).ok())
        .unwrap_or_default();
    let mut batch = rocksdb::WriteBatch::default();
    for date in &dates {
        let mut k = b"analytics_tx_day:".to_vec();
        k.extend_from_slice(date.as_bytes());
        let Some(b) = db.get_cf(&cf_state, &k)? else { continue };
        let Ok(mut agg) = bincode::deserialize::<TxDayAgg>(&b) else { continue };
        let count = orphan_count(&db, date);
        if agg.orphan_blocks != count {
            agg.orphan_blocks = count;
            batch.put_cf(&cf_state, &k, bincode::serialize(&agg)?);
            updated += 1;
        }
    }
    db.write(batch)?;
    println!("done — {updated}/{} day blobs updated. restart the backend.", dates.len());
    Ok(())
}
