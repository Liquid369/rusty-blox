//! Deterministic byte-exact gate for the parallel enrichment rewrite.
//!
//! Two independent full syncs cannot be diffed — the live daemon advances the
//! tip between runs, so recently-active addresses legitimately differ. Instead:
//! take ONE fully-synced DB (fixed `transactions` CF), COPY it, clear the copy's
//! `addr_index` + enrichment flags, and re-run `enrich_all_addresses` with a
//! chosen `sync.enrich_parallel_shards`. Snapshot the copy's addr_index and diff
//! against the original's — same input CF, no catchup, so ANY difference is a
//! real serial-vs-parallel divergence.
//!
//! Usage: DB_PATH=/path/to/copy/blocks.db reenrich
//!   (shard count comes from config.toml `sync.enrich_parallel_shards`, default 1)

use rocksdb::{Options, WriteBatch, DB};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Load config.toml from CWD so `effective_enrich_shards()` (read inside
    // enrich_all_addresses) sees `sync.enrich_parallel_shards`.
    rustyblox::config::init_global_config()?;

    let db_path = std::env::var("DB_PATH").map_err(|_| "set DB_PATH=/path/to/copy/blocks.db")?;

    let mut opts = Options::default();
    opts.create_if_missing(false);
    // Reopen read-write with default per-CF options. The CFs use no merge
    // operator or custom comparator, so logical key/value content is unaffected
    // by the option override (only file layout / compaction differ, which the
    // logical snapshot never sees).
    let cfs = DB::list_cf(&opts, &db_path)?;
    let db = Arc::new(DB::open_cf(&opts, &db_path, &cfs)?);

    // Clear addr_index so enrichment rebuilds it from scratch on this copy.
    {
        let cf = db.cf_handle("addr_index").ok_or("addr_index CF not found")?;
        let mut batch = WriteBatch::default();
        let mut n: u64 = 0;
        for item in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
            let (k, _) = item?;
            batch.delete_cf(&cf, &k);
            n += 1;
            if batch.len() >= 50_000 {
                db.write(batch)?;
                batch = WriteBatch::default();
            }
        }
        if !batch.is_empty() {
            db.write(batch)?;
        }
        eprintln!("reenrich: cleared {n} addr_index entries");
    }

    // Reset the enrichment completion flags so a fresh build runs.
    if let Some(cs) = db.cf_handle("chain_state") {
        let _ = db.delete_cf(&cs, b"address_index_complete");
        let _ = db.delete_cf(&cs, b"analytics_complete");
    }

    let shards = rustyblox::config::get_global_config()
        .get_int("sync.enrich_parallel_shards")
        .unwrap_or(1);
    eprintln!("reenrich: enrich_all_addresses (enrich_parallel_shards={shards}) ...");

    rustyblox::enrich_addresses::enrich_all_addresses(Arc::clone(&db)).await?;

    eprintln!("reenrich: enrichment complete (addr_index rebuilt; snapshot + diff now)");
    Ok(())
}
