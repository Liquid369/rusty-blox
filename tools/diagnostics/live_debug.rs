//! Throwaway: dump live-analytics state (read-only secondary; safe while running).
use rustyblox::enrich_addresses::TxDayAgg;

fn i32_at(db: &rocksdb::DB, cf: &impl rocksdb::AsColumnFamilyRef, k: &[u8]) -> Option<i32> {
    db.get_cf(cf, k)
        .ok()
        .flatten()
        .filter(|v| v.len() == 4)
        .map(|v| i32::from_le_bytes([v[0], v[1], v[2], v[3]]))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = "./data/blocks.db";
    let opts = rocksdb::Options::default();
    let cfs = rocksdb::DB::list_cf(&opts, db_path)?;
    let secondary = "/tmp/rustyblox-secondary-livedebug";
    let db = rocksdb::DB::open_cf_as_secondary(&opts, db_path, secondary, &cfs)?;
    let _ = db.try_catch_up_with_primary();
    let cf = db.cf_handle("chain_state").ok_or("no chain_state")?;

    println!("sync_height          = {:?}", i32_at(&db, &cf, b"sync_height"));
    println!("analytics_live_height= {:?}  (watermark)", i32_at(&db, &cf, b"analytics_live_height"));
    println!(
        "analytics_live_ready = {:?}",
        db.get_cf(&cf, b"analytics_live_ready")?.and_then(|v| v.first().copied())
    );
    println!(
        "analytics_complete   = {:?}",
        db.get_cf(&cf, b"analytics_complete")?.and_then(|v| v.first().copied())
    );

    if let Some(v) = db.get_cf(&cf, b"analytics_tx_days")? {
        let dates: Vec<String> = bincode::deserialize(&v).unwrap_or_default();
        let n = dates.len();
        println!("analytics_tx_days index: {n} dates; last 6 = {:?}", &dates[n.saturating_sub(6)..]);
    } else {
        println!("analytics_tx_days index: ABSENT");
    }

    for d in ["2026-06-17", "2026-06-18", "2026-06-19", "2026-06-20"] {
        let mut k = b"analytics_tx_day:".to_vec();
        k.extend_from_slice(d.as_bytes());
        match db.get_cf(&cf, &k)? {
            Some(v) => match bincode::deserialize::<TxDayAgg>(&v) {
                Ok(a) => println!(
                    "blob {d}: EXISTS  blocks={} tx_count={} fees={} orphan={}",
                    a.blocks, a.tx_count, a.fees_total, a.orphan_blocks
                ),
                Err(e) => println!("blob {d}: EXISTS but undecodable ({e})"),
            },
            None => println!("blob {d}: ABSENT"),
        }
    }
    Ok(())
}
