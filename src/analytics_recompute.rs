//! Periodic recompute of the wealth / rich-list snapshots from the live
//! `addr_index`, decoupled from the heavy full enrich.
//!
//! It reads `balance = received - sent` per address straight from the `'r'`/`'s'`
//! totals — the SAME values the `/address` API serves and the full enrich's
//! rich list is built from — so the recomputed snapshot is byte-identical to the
//! enrich computation at the same tip and always agrees with the address pages.
//!
//! Cold-stake (P2CS) coins are credited to BOTH the staker and owner address
//! (the project's chosen "keep crediting both" semantics); that falls out of the
//! `r`/`s` totals naturally, so there is no special handling here.

use crate::enrich_addresses::{
    compute_wealth_richlist, RichListSnapshotEntry, WealthSnapshot, RICHLIST_KEEP,
};
use rocksdb::{Direction, IteratorMode, WriteBatch, DB};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{error, info, warn};

type DynErr = Box<dyn std::error::Error + Send + Sync>;

/// Compute the rich list + wealth snapshot from the current `addr_index` `r`/`s`
/// totals. Pure read — does NOT persist; the caller decides when to write (the
/// background task writes both blobs in a single batch).
///
/// `balance = received - sent`, exactly as the `/address` API and the full enrich
/// derive it, so the result is consistent with address pages and byte-identical
/// to the enrich rich list at the same tip. The deterministic ordering, i128
/// accumulation and exact Nakamoto threshold all come from `compute_wealth_richlist`.
pub fn recompute_wealth_richlist_from_index(
    db: &Arc<DB>,
) -> Result<(Vec<RichListSnapshotEntry>, WealthSnapshot), DynErr> {
    let cf = db
        .cf_handle("addr_index")
        .ok_or("addr_index CF not found")?;

    // Walk the 'r'-type keys (key = b'r' + address). They sort after 'a' and
    // before 's', so seek to b"r" and stop at the first non-'r' key.
    let mut balances: Vec<(String, i64)> = Vec::new();
    for item in db.iterator_cf(&cf, IteratorMode::From(b"r", Direction::Forward)) {
        let (key, value) = item?;
        if key.first() != Some(&b'r') {
            break;
        }
        let received = <[u8; 8]>::try_from(value.as_ref())
            .map(i64::from_le_bytes)
            .unwrap_or(0);
        let address = String::from_utf8_lossy(&key[1..]).into_owned();
        let mut s_key = Vec::with_capacity(address.len() + 1);
        s_key.push(b's');
        s_key.extend_from_slice(address.as_bytes());
        let sent = db
            .get_cf(&cf, &s_key)
            .ok()
            .flatten()
            .and_then(|b| <[u8; 8]>::try_from(b.as_slice()).ok())
            .map(i64::from_le_bytes)
            .unwrap_or(0);
        // Drop zero/negative (fully-spent) balances here so they never materialize
        // in the Vec; compute_wealth_richlist also filters, but skipping the push
        // keeps the working set to actual holders.
        let bal = received - sent;
        if bal > 0 {
            balances.push((address, bal));
        }
    }

    // tx_count for the kept top-N only: length of the deduped 't' list / 36 (v2 stride).
    let tx_count_of = |address: &str| -> u64 {
        let mut t_key = Vec::with_capacity(address.len() + 1);
        t_key.push(b't');
        t_key.extend_from_slice(address.as_bytes());
        db.get_cf(&cf, &t_key)
            .ok()
            .flatten()
            .map(|b| (b.len() / crate::parser::ADDR_TX_STRIDE) as u64)
            .unwrap_or(0)
    };

    Ok(compute_wealth_richlist(
        balances,
        RICHLIST_KEEP,
        tx_count_of,
    ))
}

// ---- Step 4/5: cadence-driven background recompute + reorg safety gate ----

/// Opt-in: run the periodic richlist/wealth recompute (default false). Until this
/// is true the snapshots are refreshed only by a full enrich, exactly as before.
pub const ENABLED_KEY: &str = "sync.analytics_recompute_enabled";
/// Recompute cadence in blocks (default 60 ≈ hourly at ~1 block/min).
pub const INTERVAL_KEY: &str = "sync.analytics_recompute_interval_blocks";
const DEFAULT_INTERVAL_BLOCKS: i64 = 60;

/// addr_index undo window — the SINGLE source of truth, also used by the monitor's
/// undo-prune (`src/monitor.rs`). A reorg deeper than this can leave `r`/`s`/`a`
/// un-reversed (undo data is pruned below tip-window), so the recompute must not run
/// on a possibly-stale index until a full re-enrich rebuilds it.
pub const ADDR_INDEX_UNDO_WINDOW: i32 = 200;

/// chain_state: highest tip whose recompute is committed (i32 LE); -1 if unset.
const K_RECOMPUTE_HEIGHT: &[u8] = b"analytics_recompute_height";
/// chain_state: 1 ⇒ addr_index may be reorg-stale beyond the undo window; the
/// recompute skips until a full re-enrich clears it.
pub const K_ADDR_INDEX_DIRTY: &[u8] = b"addr_index_dirty";

fn is_enabled() -> bool {
    crate::config::get_global_config()
        .get_bool(ENABLED_KEY)
        .unwrap_or(false)
}

fn interval_blocks() -> i32 {
    crate::config::get_global_config()
        .get_int(INTERVAL_KEY)
        .unwrap_or(DEFAULT_INTERVAL_BLOCKS)
        .clamp(1, i32::MAX as i64) as i32
}

/// Is the addr_index flagged reorg-stale (Step 5)?
pub fn is_dirty(db: &Arc<DB>) -> bool {
    db.cf_handle("chain_state")
        .and_then(|cf| db.get_cf(&cf, K_ADDR_INDEX_DIRTY).ok().flatten())
        .map(|v| v.first() == Some(&1u8))
        .unwrap_or(false)
}

/// The committed recompute watermark (-1 if unset).
fn recompute_height(db: &Arc<DB>) -> i32 {
    db.cf_handle("chain_state")
        .and_then(|cf| db.get_cf(&cf, K_RECOMPUTE_HEIGHT).ok().flatten())
        .filter(|v| v.len() == 4)
        .map(|v| i32::from_le_bytes([v[0], v[1], v[2], v[3]]))
        .unwrap_or(-1)
}

/// Pure cadence/gate decision, factored out for testing.
fn should_recompute(enabled: bool, dirty: bool, tip: i32, last: i32, interval: i32) -> bool {
    enabled && !dirty && tip >= 0 && (last < 0 || tip - last >= interval)
}

/// Reorg safety (Step 5): a reorg deeper than the undo window can leave `r`/`s`/`a`
/// un-reversed, so flag the index dirty and tell the operator to re-enrich.
/// Shallow reorgs are fully reversed by the undo machinery → no-op here. Called
/// INLINE from the monitor's reorg handler.
pub fn on_reorg(db: &Arc<DB>, orphaned_blocks: i32) {
    if orphaned_blocks <= ADDR_INDEX_UNDO_WINDOW {
        return;
    }
    if let Some(cf) = db.cf_handle("chain_state") {
        // The dirty flag is the ONLY thing stopping the periodic recompute from
        // refreshing richlist/wealth off a known-un-reversed index. A failed put
        // must not be logged as "paused" — surface it as the failure it is.
        if let Err(e) = db.put_cf(&cf, K_ADDR_INDEX_DIRTY, [1u8]) {
            error!(orphaned_blocks, error = %e,
                "analytics-recompute: FAILED to set dirty flag after deep reorg — recompute is NOT paused and may refresh from an un-reversed index");
            return;
        }
        warn!(
            orphaned_blocks,
            undo_window = ADDR_INDEX_UNDO_WINDOW,
            "analytics-recompute: reorg deeper than addr_index undo window — richlist/wealth recompute paused; run a full re-enrich to refresh + clear"
        );
    }
}

/// Mark the addr_index clean after a full enrich rebuilt `a`/`r`/`s`/`t` and wrote a
/// fresh richlist/wealth snapshot from them, in ONE durable batch: (1) clear the
/// reorg-stale flag so the periodic recompute resumes, and (2) set the recompute
/// watermark to this tip so the next tick doesn't redundantly re-scan the snapshot
/// the enrich just wrote. Called from the MAIN enrich pass (not the crash-prone
/// detached daily-series tail), so a dirty flag can't wedge the recompute off.
pub fn mark_index_clean(db: &Arc<DB>, tip: i32) {
    if let Some(cf) = db.cf_handle("chain_state") {
        let mut batch = WriteBatch::default();
        batch.delete_cf(&cf, K_ADDR_INDEX_DIRTY);
        batch.put_cf(&cf, K_RECOMPUTE_HEIGHT, tip.to_le_bytes());
        // Stamp the addr_index format version. This runs ONLY inside the v2
        // transaction-enrich (enrich_all_addresses), so a successful enrich marks the
        // index v2-ready; the chainstate / !fast_sync writers never reach here, so they
        // never stamp and the API 503-gate keeps them non-served (MS-4 / BE-3).
        batch.put_cf(
            &cf,
            crate::chain_state::ADDR_INDEX_VERSION_KEY,
            crate::parser::ADDR_INDEX_FORMAT_VERSION.to_le_bytes(),
        );
        if let Err(e) = db.write(batch) {
            tracing::error!(error = %e, "failed to persist addr_index clean + version markers; the index stays 503 and re-enriches on next boot (fail-closed)");
        }
    }
}

/// Clear the reorg-stale flag WITHOUT advancing the recompute watermark — used when
/// the index was rebuilt but writing the fresh snapshot failed, so the watermark
/// never asserts "recompute fresh at tip" without a blob and the next periodic
/// recompute still runs to refresh it.
pub fn clear_dirty_only(db: &Arc<DB>) {
    if let Some(cf) = db.cf_handle("chain_state") {
        // The a/r/s/t index is v2-complete even when the wealth snapshot write failed,
        // so stamp the version (so it can serve) while clearing the dirty flag.
        let mut batch = WriteBatch::default();
        batch.delete_cf(&cf, K_ADDR_INDEX_DIRTY);
        batch.put_cf(
            &cf,
            crate::chain_state::ADDR_INDEX_VERSION_KEY,
            crate::parser::ADDR_INDEX_FORMAT_VERSION.to_le_bytes(),
        );
        if let Err(e) = db.write(batch) {
            tracing::error!(error = %e, "failed to persist addr_index clean + version markers; the index stays 503 and re-enriches on next boot (fail-closed)");
        }
    }
}

/// Write the recomputed snapshots and advance the watermark in ONE batch, so the
/// richlist, wealth blob and watermark can never be torn apart.
fn persist(
    db: &Arc<DB>,
    richlist: &[RichListSnapshotEntry],
    wealth: &WealthSnapshot,
    height: i32,
) -> Result<(), DynErr> {
    let cf = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    let mut batch = WriteBatch::default();
    batch.put_cf(&cf, b"analytics_richlist", bincode::serialize(richlist)?);
    batch.put_cf(&cf, b"analytics_wealth", bincode::serialize(wealth)?);
    batch.put_cf(&cf, K_RECOMPUTE_HEIGHT, height.to_le_bytes());
    db.write(batch)?;
    Ok(())
}

/// Process-local single-flight guard so a slow recompute can't pile up across
/// monitor iterations (only one scan runs at a time).
static RECOMPUTE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

struct RecomputeGuard(());
impl RecomputeGuard {
    fn try_acquire() -> Option<RecomputeGuard> {
        if RECOMPUTE_IN_FLIGHT.swap(true, Ordering::SeqCst) {
            None
        } else {
            Some(RecomputeGuard(()))
        }
    }
}
impl Drop for RecomputeGuard {
    fn drop(&mut self) {
        RECOMPUTE_IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}

/// Steady-state hook: call from the monitor loop (right after `analytics_live::tick`).
/// Returns before any DB read on the default path (feature disabled). When a recompute
/// is due it DETACHES the O(addresses) scan onto a blocking worker and returns
/// immediately, so the monitor keeps polling/indexing while the snapshot recomputes; a
/// single-flight guard prevents overlapping scans. The scan is eventually-consistent
/// (a concurrent block write may be seen by only part of it); that self-heals on the
/// next interval and never touches canonical state — the snapshot blobs are a derived
/// cache. The watermark advances only on a successful persist, so a failure retries
/// next interval. Must be called from within a Tokio runtime (the monitor task).
pub fn maybe_recompute(db: &Arc<DB>) {
    if !is_enabled() {
        return;
    }
    let tip = crate::chain_state::get_sync_height(db).unwrap_or(-1);
    if !should_recompute(
        true,
        is_dirty(db),
        tip,
        recompute_height(db),
        interval_blocks(),
    ) {
        return;
    }
    let Some(guard) = RecomputeGuard::try_acquire() else {
        return; // a recompute is already running
    };
    let db2 = Arc::clone(db);
    tokio::task::spawn_blocking(move || {
        // Released on drop, covering every exit (normal, error, panic).
        let _guard = guard;
        match recompute_wealth_richlist_from_index(&db2) {
            Ok((rl, w)) => {
                // Re-check the dirty gate AFTER the scan: a deep reorg may have flagged
                // the index mid-scan, in which case this result reflects the pre-reorg
                // chain — discard it (also closes the TOCTOU vs the pre-dispatch check).
                if is_dirty(&db2) {
                    warn!("analytics-recompute: index flagged dirty mid-scan; discarding result");
                } else if let Err(e) = persist(&db2, &rl, &w, tip) {
                    warn!(error = %e, "analytics-recompute: persist failed (will retry next interval)");
                } else {
                    info!(
                        tip,
                        holders = w.address_count,
                        richlist = rl.len(),
                        "analytics-recompute: richlist/wealth snapshot refreshed"
                    );
                }
            }
            Err(e) => warn!(error = %e, "analytics-recompute: compute failed"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocksdb::{Options, DB};
    use std::sync::Arc;

    fn open_db() -> (tempfile::TempDir, Arc<DB>) {
        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = Arc::new(DB::open_cf(&opts, temp.path(), &["addr_index", "chain_state"]).unwrap());
        (temp, db)
    }

    fn put_addr(db: &Arc<DB>, addr: &str, received: i64, sent: i64, n_txs: usize) {
        let cf = db.cf_handle("addr_index").unwrap();
        db.put_cf(&cf, format!("r{addr}").as_bytes(), received.to_le_bytes())
            .unwrap();
        db.put_cf(&cf, format!("s{addr}").as_bytes(), sent.to_le_bytes())
            .unwrap();
        // v2 't' list = n_txs * 36-byte records; only the length matters for tx_count.
        db.put_cf(&cf, format!("t{addr}").as_bytes(), vec![0u8; n_txs * 36])
            .unwrap();
    }

    #[test]
    fn recompute_reads_r_minus_s_and_ranks_deterministically() {
        let (_t, db) = open_db();
        // alpha = 900-100 = 800, bravo = 500-0 = 500, zero = 0 (dropped),
        // neg = 5-20 = -15 (dropped).
        put_addr(&db, "alpha", 900, 100, 3);
        put_addr(&db, "bravo", 500, 0, 1);
        put_addr(&db, "zero", 10, 10, 2);
        put_addr(&db, "neg", 5, 20, 1);

        let (rl, w) = recompute_wealth_richlist_from_index(&db).unwrap();

        assert_eq!(rl.len(), 2);
        assert_eq!(rl[0].address, "alpha");
        assert_eq!(rl[0].balance, 800);
        assert_eq!(rl[0].tx_count, 3);
        assert_eq!(rl[1].address, "bravo");
        assert_eq!(rl[1].balance, 500);
        assert_eq!(rl[1].tx_count, 1);
        assert_eq!(w.address_count, 2);
        assert_eq!(w.total_balance, 1300);
    }

    #[test]
    fn recompute_keeps_both_staker_and_owner_for_cold_stake() {
        // A P2CS coin is credited to BOTH staker and owner in r/s (credit-both),
        // so the recompute lists both and the total double-counts — by design.
        let (_t, db) = open_db();
        put_addr(&db, "owner_addr", 1000, 0, 1);
        put_addr(&db, "staker_addr", 1000, 0, 1);

        let (rl, w) = recompute_wealth_richlist_from_index(&db).unwrap();

        assert_eq!(rl.len(), 2);
        assert_eq!(w.total_balance, 2000);
        let addrs: Vec<&str> = rl.iter().map(|e| e.address.as_str()).collect();
        assert!(addrs.contains(&"owner_addr"));
        assert!(addrs.contains(&"staker_addr"));
    }

    #[test]
    fn recompute_empty_index_yields_empty_snapshot() {
        let (_t, db) = open_db();
        let (rl, w) = recompute_wealth_richlist_from_index(&db).unwrap();
        assert!(rl.is_empty());
        assert_eq!(w.total_balance, 0);
        assert_eq!(w.address_count, 0);
    }

    // ---- Step 4/5: cadence gate, reorg dirty-gate, persist ----

    #[test]
    fn should_recompute_gates() {
        // disabled -> never; dirty -> never.
        assert!(!should_recompute(false, false, 100, -1, 60));
        assert!(!should_recompute(true, true, 100, -1, 60));
        // first run (no prior watermark) -> yes.
        assert!(should_recompute(true, false, 100, -1, 60));
        // below / at / above the interval.
        assert!(!should_recompute(true, false, 100, 50, 60));
        assert!(should_recompute(true, false, 110, 50, 60));
        assert!(should_recompute(true, false, 200, 50, 60));
        // no tip yet -> no.
        assert!(!should_recompute(true, false, -1, -1, 60));
    }

    #[test]
    fn on_reorg_marks_dirty_only_beyond_undo_window() {
        let (_t, db) = open_db();
        on_reorg(&db, ADDR_INDEX_UNDO_WINDOW); // exactly the window: still reversible
        assert!(!is_dirty(&db));
        on_reorg(&db, ADDR_INDEX_UNDO_WINDOW + 1); // beyond: not reversed
        assert!(is_dirty(&db));
    }

    #[test]
    fn persist_writes_blobs_and_advances_watermark() {
        let (_t, db) = open_db();
        put_addr(&db, "a", 500, 0, 1);
        let (rl, w) = recompute_wealth_richlist_from_index(&db).unwrap();
        persist(&db, &rl, &w, 1234).unwrap();

        assert_eq!(recompute_height(&db), 1234);
        let cf = db.cf_handle("chain_state").unwrap();
        let stored: Vec<RichListSnapshotEntry> =
            bincode::deserialize(&db.get_cf(&cf, b"analytics_richlist").unwrap().unwrap()).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].address, "a");
        assert!(db.get_cf(&cf, b"analytics_wealth").unwrap().is_some());
    }
}
