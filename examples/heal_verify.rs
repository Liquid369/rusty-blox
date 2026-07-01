//! Localhost end-to-end verification of the heightless-tx heal against the REAL local DB
//! + node. Injects a synthetic `-1` height on a known-canonical tx (simulating the
//! production stuck state), runs `repair::reresolve_heightless_blocks` (nominate → RPC
//! `getblock` → promote), and asserts the height is restored byte-exactly. The inject+heal
//! round-trips, so the DB is left byte-identical to how it started.
//!
//! Run from the repo root (needs config.toml + local DB + a synced pivxd), with the
//! backend NOT running (single-writer lock):
//!   cargo +1.88.0 run --release --offline --locked --example heal_verify

use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use std::sync::Arc;

const TXID_DISPLAY: &str = "6d04c36f3f6f4adaf55ce658169d67327c4b56fe172cc5b7d2ad6e8a24d4cded";

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rustyblox::config::init_global_config()?;
    let db_path = rustyblox::config::get_global_config().get_string("paths.db_path")?;

    let mut db_opts = Options::default();
    db_opts.create_if_missing(false);
    let mut cfds = vec![ColumnFamilyDescriptor::new("default", Options::default())];
    for name in rustyblox::COLUMN_FAMILIES {
        cfds.push(ColumnFamilyDescriptor::new(
            name.to_string(),
            Options::default(),
        ));
    }
    let db = Arc::new(DB::open_cf_descriptors(&db_opts, &db_path, cfds)?);
    let cf = db.cf_handle("transactions").unwrap();

    // Historic records are internal-keyed (reversed display); live-tip records are
    // display-keyed. Find whichever this DB used.
    let display = hex::decode(TXID_DISPLAY)?;
    let internal: Vec<u8> = display.iter().rev().cloned().collect();
    let key_internal = {
        let mut k = vec![b't'];
        k.extend_from_slice(&internal);
        k
    };
    let key_display = {
        let mut k = vec![b't'];
        k.extend_from_slice(&display);
        k
    };
    let (key, order) = if db.get_cf(&cf, &key_internal)?.is_some() {
        (key_internal, "internal")
    } else if db.get_cf(&cf, &key_display)?.is_some() {
        (key_display, "display")
    } else {
        panic!("tx 6d04c36f not present in local DB at either key order");
    };
    println!("[0] tx found at the {order} key");

    let orig = db.get_cf(&cf, &key)?.unwrap();
    let orig_h = i32::from_le_bytes([orig[4], orig[5], orig[6], orig[7]]);
    println!("[1] original stored height = {orig_h}");
    assert!(
        orig_h > 0,
        "need a canonical tx to inject into (got {orig_h})"
    );

    // Inject the production stuck state: height -> -1.
    let mut broken = orig[0..4].to_vec();
    broken.extend((-1i32).to_le_bytes());
    broken.extend(&orig[8..]);
    db.put_cf(&cf, &key, &broken)?;
    println!("[2] injected height = -1 (simulating the production stuck state)");

    // Run the REAL heal: full CF scan → nominate the block via 'B' → RPC getblock → promote.
    println!("[3] running reresolve_heightless_blocks (scans transactions CF + RPC getblock)…");
    let (blocks, promoted) = rustyblox::repair::reresolve_heightless_blocks(&db).await?;
    println!("    heal result: blocks_processed={blocks}, txs_promoted={promoted}");

    let fixed = db.get_cf(&cf, &key)?.unwrap();
    let fixed_h = i32::from_le_bytes([fixed[4], fixed[5], fixed[6], fixed[7]]);
    println!("[4] healed stored height = {fixed_h}");

    assert!(promoted >= 1, "expected >= 1 promotion, got {promoted}");
    assert_eq!(fixed_h, orig_h, "height not restored to {orig_h}");
    assert_eq!(&fixed[0..4], &orig[0..4], "version bytes changed");
    assert_eq!(&fixed[8..], &orig[8..], "tx bytes changed");
    println!("\n✅ VERIFIED end-to-end: -1 → {fixed_h}, version+tx bytes byte-identical. DB round-tripped.");
    Ok(())
}
