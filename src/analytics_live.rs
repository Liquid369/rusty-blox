//! Live daily-analytics updater — Lane I (per-block incremental).
//!
//! See `DESIGN-live-analytics-update.md`. Lane I keeps the `analytics_tx_day:
//! <date>` blobs current as the monitor connects canonical blocks, reusing the
//! EXACT shared per-tx primitives the full enrich uses (`accumulate_tx_inline` +
//! `compute_tx_join`) over the SAME source bytes (the `transactions` CF), so the
//! incremental result matches a full enrich. The set fields + orphan_blocks are
//! Lane R (window recompute, separate); `new_addresses` is full-enrich-owned.
//!
//! Concurrency: Lane I HARD-NO-OPS unless `analytics_live_ready == 1` (set only by
//! a completed FULL enrich), so it can never write a day blob concurrently with
//! the enrich's detached daily-series thread or onto a degraded (join-zeroed) base.

use rocksdb::{WriteBatch, DB};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Opt-in: auto-trigger a full re-enrich to re-green the gate after a deep reorg
/// (`sync.live_analytics_auto_reenrich`, default false — a full enrich is ~heavy,
/// so default to logging + a manual re-enrich).
pub const AUTO_REENRICH_KEY: &str = "sync.live_analytics_auto_reenrich";

/// Process-local single-flight guard for the full join enrich (shared by
/// `run_full_analytics_enrich` and the startup interrupted-enrich recovery).
static REENRICH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// RAII single-flight guard for the full join enrich. Acquire via
/// `ReenrichGuard::try_acquire`; it releases automatically on drop — covering EVERY
/// exit path (normal return, `?` error propagation, panic unwind) — so the guard can
/// never leak and permanently brick future re-enrich. Shares the same atomic as
/// `run_full_analytics_enrich`, so the startup interrupted-enrich recovery and the
/// monitor's auto-reenrich can never overlap.
pub struct ReenrichGuard(());

impl ReenrichGuard {
    /// Take the guard if no full enrich is already running in this process.
    pub fn try_acquire() -> Option<ReenrichGuard> {
        if REENRICH_IN_PROGRESS.swap(true, Ordering::SeqCst) {
            None
        } else {
            Some(ReenrichGuard(()))
        }
    }
}

impl Drop for ReenrichGuard {
    fn drop(&mut self) {
        REENRICH_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

/// Is auto-reenrich opted in?
fn auto_reenrich() -> bool {
    crate::config::get_global_config()
        .get_bool(AUTO_REENRICH_KEY)
        .unwrap_or(false)
}

/// Detached, single-flight FULL re-enrich (the join-producing `enrich_all_addresses`,
/// NOT the consume-only daily-series tail). Its daily-series handoff re-greens the
/// live gate strictly-last. Used to recover from a deep reorg / degraded base. The
/// process-local guard prevents a concurrent heavy Pass-1/2b overlap.
pub fn run_full_analytics_enrich(db: &Arc<DB>) {
    let Some(guard) = ReenrichGuard::try_acquire() else {
        return; // already running in this process
    };
    let db_bg = Arc::clone(db);
    std::thread::spawn(move || {
        // Moved into the thread: the guard releases when this thread ends, covering
        // every exit (incl. the runtime-build-fail early return below).
        let _guard = guard;
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                warn!(error = %e, "run_full_analytics_enrich: runtime build failed");
                return;
            }
        };
        rt.block_on(async move {
            info!("live-analytics: full re-enrich starting (re-green)");
            if let Err(e) = crate::enrich_addresses::enrich_all_addresses(Arc::clone(&db_bg)).await
            {
                warn!(error = %e, "live-analytics: full re-enrich failed");
            }
        });
    });
}

/// Lane-R live window in calendar days (set fields + orphan_blocks recompute).
pub const R_DAYS: i64 = 3;
/// Reorg-depth threshold in BLOCKS (distinct from R_DAYS) — deeper ⇒ full enrich.
pub const R_BLOCKS: i32 = 4320;
/// Lane-R recompute + orphan-mark cadence in blocks (~hourly).
pub const R_INTERVAL: i32 = 60;
const SECS_PER_DAY: u32 = 86_400;

use crate::enrich_addresses::{
    accumulate_tx_inline, classify_output, compute_tx_join, nbits_to_difficulty, unix_to_date,
    TreasuryPayout, TxDayAgg, TxJoinInputs,
};
use crate::types::ScriptClassification;

/// chain_state key: 1 ⇒ a FULL enrich completed and the live baseline (incl. the
/// join fields) is valid. Live no-ops while != 1.
pub const K_READY: &[u8] = b"analytics_live_ready";
/// chain_state key: highest height whose Lane-I contribution is committed (i32 LE).
pub const K_WATERMARK: &[u8] = b"analytics_live_height";
/// Per-day private side keyspaces (real vs shadow): the exact interval array
/// (later-block height → dt secs, overwrite-keyed for replay idempotency) and the
/// running Σ difficulty (f64 LE; `avg = sum/blocks`). Namespaced per mode so a
/// shadow run never disturbs the real Lane-I running state.
const PFX_INTERVALS: &[u8] = b"live_day_intervals:";
const PFX_DIFFSUM: &[u8] = b"live_day_diffsum:";
const PFX_INTERVALS_SHADOW: &[u8] = b"shadow_intervals:";
const PFX_DIFFSUM_SHADOW: &[u8] = b"shadow_diffsum:";

/// (diffsum_prefix, intervals_prefix) for the chosen day-blob keyspace.
fn side_prefixes(key_prefix: &str) -> (&'static [u8], &'static [u8]) {
    if key_prefix.starts_with("shadow") {
        (PFX_DIFFSUM_SHADOW, PFX_INTERVALS_SHADOW)
    } else {
        (PFX_DIFFSUM, PFX_INTERVALS)
    }
}

/// The treasury blob key for the chosen mode (real vs shadow).
fn treasury_key(key_prefix: &str) -> &'static [u8] {
    if key_prefix.starts_with("shadow") {
        b"shadow_treasury"
    } else {
        b"analytics_treasury"
    }
}

/// Add `date` to the API's `analytics_tx_days` index (sorted, deduped) within
/// `batch`, if not already present. The index is the only date source the
/// analytics endpoints read, so a live-created day MUST be registered.
fn register_date(
    db: &Arc<DB>,
    cf_state: &impl rocksdb::AsColumnFamilyRef,
    batch: &mut WriteBatch,
    date: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut dates: Vec<String> = db
        .get_cf(cf_state, b"analytics_tx_days")?
        .and_then(|b| bincode::deserialize(&b).ok())
        .unwrap_or_default();
    if !dates.iter().any(|d| d == date) {
        dates.push(date.to_string());
        dates.sort();
        dates.dedup();
        batch.put_cf(cf_state, b"analytics_tx_days", bincode::serialize(&dates)?);
    }
    Ok(())
}

/// Is the live baseline valid? (the §0 gate). Cheap; read fresh per call.
pub fn is_ready(db: &Arc<DB>) -> bool {
    db.cf_handle("chain_state")
        .and_then(|cf| db.get_cf(&cf, K_READY).ok().flatten())
        .map(|v| v.first() == Some(&1u8))
        .unwrap_or(false)
}

/// Set/clear the live-ready gate (used by the enrich handoff + reorg).
pub fn set_ready(batch: &mut WriteBatch, cf_state: &impl rocksdb::AsColumnFamilyRef, ready: bool) {
    batch.put_cf(cf_state, K_READY, [ready as u8]);
}

/// The committed Lane-I watermark (−1 if unset).
pub fn watermark(db: &Arc<DB>) -> i32 {
    db.cf_handle("chain_state")
        .and_then(|cf| db.get_cf(&cf, K_WATERMARK).ok().flatten())
        .filter(|v| v.len() == 4)
        .map(|v| i32::from_le_bytes([v[0], v[1], v[2], v[3]]))
        .unwrap_or(-1)
}

/// Feature flag: `sync.live_analytics` (default false).
pub const FLAG_KEY: &str = "sync.live_analytics";
/// Phase-0 shadow flag: `sync.live_analytics_shadow` (default true).
pub const SHADOW_FLAG_KEY: &str = "sync.live_analytics_shadow";

/// Is the live daily-analytics updater enabled?
pub fn is_enabled() -> bool {
    crate::config::get_global_config()
        .get_bool(FLAG_KEY)
        .unwrap_or(false)
}

/// Phase-0 shadow mode (default ON): Lane I/R write `shadow_tx_day:` blobs and
/// leave the real `analytics_tx_day:` untouched, for diffing against a full enrich.
pub fn shadow_mode() -> bool {
    crate::config::get_global_config()
        .get_bool(SHADOW_FLAG_KEY)
        .unwrap_or(true)
}

/// The day-blob keyspace prefix for the current mode.
pub fn key_prefix() -> &'static str {
    if shadow_mode() {
        "shadow_tx_day:"
    } else {
        "analytics_tx_day:"
    }
}

/// Steady-state live tick: drive Lane I forward over `(watermark, sync_height]`
/// (idempotent — `apply_block` no-ops at/under the watermark, so this also
/// backfills blocks the monitor connected during the detached enrich window), then
/// run the Lane-R recompute on an `R_INTERVAL`-block cadence. No-ops unless the
/// feature is enabled AND the live baseline is ready. Call INLINE from the
/// steady-state monitor only (never the pre-enrich catchup).
pub async fn tick(db: &Arc<DB>) {
    if !is_enabled() || !is_ready(db) {
        return;
    }
    let prefix = key_prefix();
    let tip = crate::chain_state::get_sync_height(db).unwrap_or(-1);
    let start = watermark(db);
    if tip <= start {
        return;
    }
    let count = tip - start;
    if count > 10 {
        // A real backfill (gap > 10 blocks) — make it visible.
        info!(
            from = start + 1,
            to = tip,
            count,
            prefix,
            "live-analytics: backfilling blocks"
        );
    } else {
        debug!(
            from = start + 1,
            to = tip,
            count,
            prefix,
            "live-analytics: applying blocks"
        );
    }
    let mut wm = start;
    while wm < tip {
        let h = wm + 1;
        if let Err(e) = apply_block(db, h, prefix).await {
            warn!(height = h, error = %e, "live-analytics: apply_block failed");
            return;
        }
        wm = h;
    }
    if count > 10 {
        info!(watermark = wm, "live-analytics: backfill complete");
    }
    // Lane R every R_INTERVAL: recompute the O(window) set fields AND persist any
    // new orphans from blk-tail (tail-only, cheap — no blocks-CF iterate), so
    // orphan_blocks is at most ~1h stale.
    if wm / R_INTERVAL != start / R_INTERVAL {
        if let Err(e) = recompute_window(db, tip, prefix, true).await {
            warn!(error = %e, "live-analytics: recompute_window failed");
        }
    }
}

/// Live reorg handler — keep the Lane-I day blobs correct across a rollback. Runs
/// INLINE in the monitor task (right after `handle_reorg`, before the new chain is
/// re-indexed), so it cannot interleave with `tick`. It DELETES every live day blob
/// (+ its difficulty/interval side keys) on or after the fork date and resets the
/// watermark to just before the fork date's first block, so subsequent ticks
/// REBUILD those days from scratch as the new chain is indexed (delete-then-rebuild
/// ⇒ no double-count, and no need for old-chain data). A DEEP reorg (> `R_BLOCKS`)
/// clears the gate instead — older affected days fall outside Lane R's window, so a
/// full re-enrich must re-green it. No-ops unless the feature is enabled.
pub fn on_reorg(db: &Arc<DB>, fork_height: i32, orphaned_blocks: i32) {
    if !is_enabled() {
        return;
    }
    // A no-op reorg (fork == tip, nothing rolled back — e.g. the canonical-tip
    // probe fired on a flapping node and find_fork_point resolved at the tip)
    // changes no chain state: deleting day blobs / rewinding the watermark here
    // would thrash today's analytics for nothing.
    if orphaned_blocks <= 0 {
        return;
    }
    let Some(cf_state) = db.cf_handle("chain_state") else {
        return;
    };
    if orphaned_blocks > R_BLOCKS {
        let _ = db.put_cf(&cf_state, K_READY, [0u8]);
        if auto_reenrich() {
            warn!(
                orphaned_blocks,
                "live-analytics: deep reorg — gate cleared, auto re-enrich starting"
            );
            run_full_analytics_enrich(db);
        } else {
            warn!(
                orphaned_blocks,
                "live-analytics: deep reorg — gate cleared; run a full re-enrich to re-green (or set sync.live_analytics_auto_reenrich=true)"
            );
        }
        return;
    }
    let Some((fork_time, _)) = header_time_bits(db, fork_height) else {
        return;
    };
    if fork_time == 0 {
        return;
    }
    // Delete from one day BEFORE the fork date, so a non-monotonic header time that
    // mapped a rolled-back block to a date < fork_date is still cleaned + rebuilt.
    let delete_from = unix_to_date((fork_time as u64).saturating_sub(SECS_PER_DAY as u64));
    // H_first = first canonical block whose date >= delete_from.
    let mut h_first = fork_height;
    while h_first > 0 {
        match header_time_bits(db, h_first - 1) {
            Some((t, _)) if t != 0 && unix_to_date(t as u64).as_str() >= delete_from.as_str() => {
                h_first -= 1
            }
            _ => break,
        }
    }
    let prefix = key_prefix();
    let (diff_pfx, int_pfx) = side_prefixes(prefix);
    let mut batch = WriteBatch::default();
    for item in db.prefix_iterator_cf(&cf_state, prefix.as_bytes()) {
        let Ok((key, _)) = item else { break };
        if !key.starts_with(prefix.as_bytes()) {
            break;
        }
        let date_str = String::from_utf8_lossy(&key[prefix.len()..]).to_string();
        if date_str.as_str() >= delete_from.as_str() {
            batch.delete_cf(&cf_state, &key[..]);
            batch.delete_cf(&cf_state, side_key(diff_pfx, &date_str));
            batch.delete_cf(&cf_state, side_key(int_pfx, &date_str));
        }
    }
    // Prune treasury payouts at heights the rollback orphaned (>= h_first); the
    // re-applied new chain re-adds the correct ones (dedup-by-height is then safe).
    let tkey = treasury_key(prefix);
    if let Ok(Some(b)) = db.get_cf(&cf_state, tkey) {
        if let Ok(mut v) = bincode::deserialize::<Vec<TreasuryPayout>>(&b) {
            let before = v.len();
            v.retain(|t| t.height < h_first);
            if v.len() != before {
                if let Ok(ser) = bincode::serialize(&v) {
                    batch.put_cf(&cf_state, tkey, ser);
                }
            }
        }
    }
    batch.put_cf(&cf_state, K_WATERMARK, (h_first - 1).to_le_bytes());
    let _ = db.write(batch);
    warn!(fork_height, h_first, %delete_from, "live-analytics: reorg — cleared affected days, watermark reset (will rebuild)");
}

/// Read a canonical block header's (nTime, nBits), mirroring `build_block_times`
/// EXACTLY (chain_metadata height→display_hash → blocks[internal] → header bytes;
/// nTime@68..72, nBits@72..76). Returns None if the header is missing/short.
fn header_time_bits(db: &Arc<DB>, height: i32) -> Option<(u32, u32)> {
    let cf_metadata = db.cf_handle("chain_metadata")?;
    let cf_blocks = db.cf_handle("blocks")?;
    let display_hash = db.get_cf(&cf_metadata, height.to_le_bytes()).ok()??;
    let internal_hash: Vec<u8> = display_hash.iter().rev().cloned().collect();
    let header = db.get_cf(&cf_blocks, &internal_hash).ok()??;
    if header.len() >= 76 {
        Some((
            u32::from_le_bytes(header[68..72].try_into().ok()?),
            u32::from_le_bytes(header[72..76].try_into().ok()?),
        ))
    } else {
        None
    }
}

/// Reconstruct a day's Lane-I running state (Σ difficulty, the per-height interval
/// map incl. the cross-midnight first interval) from canonical headers, over
/// `[first block of `date` .. up_to_height]`. Used to resume on the enrich→live
/// boundary day, where the enrich wrote the day blob but not the side keys.
fn seed_day_state(db: &Arc<DB>, date: &str, up_to_height: i32) -> (f64, HashMap<i32, u32>) {
    let mut diffsum = 0.0f64;
    let mut intervals: HashMap<i32, u32> = HashMap::new();
    if up_to_height < 0 {
        return (diffsum, intervals);
    }
    // First block of `date` (walk back while the date matches).
    let mut lo = up_to_height;
    while lo > 0 {
        match header_time_bits(db, lo - 1) {
            Some((t, _)) if t != 0 && unix_to_date(t as u64) == date => lo -= 1,
            _ => break,
        }
    }
    for h in lo..=up_to_height {
        let Some((t, bits)) = header_time_bits(db, h) else {
            continue;
        };
        if t == 0 {
            continue;
        }
        diffsum += nbits_to_difficulty(bits);
        if h >= 1 {
            if let Some((pt, _)) = header_time_bits(db, h - 1) {
                if pt != 0 {
                    intervals.insert(h, t.saturating_sub(pt));
                }
            }
        }
    }
    (diffsum, intervals)
}

/// The resolved `transactions`-CF VALUE bytes (`[version 4][height 4][raw tx]`) of
/// each tx in a canonical block, deduped, via the `'B' + height(4 LE) + index`
/// block index. The `'B'` VALUE orientation is NOT uniform — the parse/`.blk` path
/// stores hex-of-internal while the monitor/RPC path stores hex-of-display (both
/// 13-byte key), and the degraded-rebuild path stores the raw 32-byte suffix
/// (9-byte key). So for each entry we DUAL-PROBE both byte orderings of the decoded
/// txid against the CF and return the `'t'` value of whichever `b't'+suffix`
/// actually exists — robust to every writer. Probe order is RAW-FIRST: the live hot
/// path (`apply_block` via the monitor tick) processes monitor-written values
/// (hex-of-display), so raw=display hits on the first probe; the parse path
/// resolves on the reversed probe. Returning the resolved value (not the suffix)
/// also saves the caller a second lookup; dedup (by resolved suffix) guards against
/// a stale `'B'` entry at a shifted index. **Fallible:** a real RocksDB read error
/// propagates (it must NOT collapse into a silent miss, which would under-count the
/// day and then advance the watermark past the gap — the enrich propagates such
/// errors via `?`, so Lane I must too).
fn block_tx_values(
    db: &Arc<DB>,
    cf_tx: &impl rocksdb::AsColumnFamilyRef,
    height: i32,
) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error>> {
    let mut prefix = vec![b'B'];
    prefix.extend_from_slice(&height.to_le_bytes());
    let mut out = Vec::new();
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let mut unresolved = 0u32;
    for item in db.prefix_iterator_cf(cf_tx, &prefix) {
        let (key, value) = item?; // a real iterator error must not truncate the block
        if key.first() != Some(&b'B') || !key.starts_with(&prefix) {
            break;
        }
        let raw: Vec<u8> = if key.len() == 13 {
            match hex::decode(&value) {
                Ok(b) => b,
                Err(_) => continue,
            }
        } else if key.len() == 9 && value.len() == 32 {
            value.to_vec()
        } else {
            continue;
        };
        let reversed: Vec<u8> = raw.iter().rev().cloned().collect();
        let mut resolved = false;
        for cand in [raw, reversed] {
            let mut k = vec![b't'];
            k.extend_from_slice(&cand);
            if let Some(v) = db.get_cf(cf_tx, &k)? {
                if seen.insert(cand) {
                    out.push(v.to_vec());
                }
                resolved = true;
                break;
            }
        }
        if !resolved {
            unresolved += 1;
        }
    }
    if unresolved > 0 {
        warn!(
            height,
            unresolved, "live-analytics: 'B' index entries with no matching 't' tx (corruption?)"
        );
    }
    Ok(out)
}

/// Per-input resolved prevout (value, creation height, is-coldstake) — resolved
/// via the `transactions` CF, the same bytes the enrich's `packed` is built from.
struct ResolvedInput {
    value: i64,
    prev_height: i32,
    is_p2cs: bool,
}

/// Resolve one input's prevout through the `transactions` CF. Returns None for an
/// UNRESOLVED input (lookup MISS — incl. zPoS/zerocoin null prevouts — or a
/// NEGATIVE stored height), exactly as the enrich's `tx_index` miss / sentinel
/// path, so the clamp suppresses fee/minted. A resolved height of 0 (genesis) is
/// counted.
async fn resolve_prevout(
    db: &Arc<DB>,
    cf_tx: &impl rocksdb::AsColumnFamilyRef,
    prev_txid_display_hex: &str,
    vout: u32,
) -> Option<ResolvedInput> {
    let prev_txid_bytes = hex::decode(prev_txid_display_hex).ok()?;
    let mut key = vec![b't'];
    key.extend_from_slice(&prev_txid_bytes);
    let data = db.get_cf(cf_tx, &key).ok()??;
    if data.len() < 8 {
        return None;
    }
    let prev_height = i32::from_le_bytes(data[4..8].try_into().ok()?);
    if prev_height < 0 {
        return None; // negative sentinel (−1/−2) ⇒ unresolved
    }
    let mut with_header = Vec::with_capacity(4 + data.len() - 8);
    with_header.extend_from_slice(&[0u8; 4]);
    with_header.extend_from_slice(&data[8..]);
    let prev_tx = crate::parser::deserialize_transaction(&with_header)
        .await
        .ok()?;
    let out = prev_tx.outputs.get(vout as usize)?;
    Some(ResolvedInput {
        value: out.value,
        prev_height,
        is_p2cs: matches!(classify_output(out), ScriptClassification::ColdStake { .. }),
    })
}

/// Apply ONE canonical block's Lane-I contribution to its day blob (RMW), plus
/// the running difficulty sum, the interval array, and the watermark — all in one
/// `WriteBatch`. No-ops unless `analytics_live_ready == 1` and `height >
/// watermark`. `key_prefix` selects the live blobs (`analytics_tx_day:`) or the
/// Phase-0 shadow blobs (`shadow_tx_day:`).
pub async fn apply_block(
    db: &Arc<DB>,
    height: i32,
    key_prefix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if !is_ready(db) || height <= watermark(db) || height < 0 {
        return Ok(());
    }
    apply_block_core(db, height, key_prefix, true).await
}

/// The Lane-I work for one block, gate/watermark-agnostic. `advance_watermark`
/// commits the watermark (true from `apply_block`; false for shadow validation,
/// which drives an explicit range and must not move the live watermark).
pub(crate) async fn apply_block_core(
    db: &Arc<DB>,
    height: i32,
    key_prefix: &str,
    advance_watermark: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if height < 0 {
        return Ok(());
    }
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    let cf_tx = db
        .cf_handle("transactions")
        .ok_or("transactions CF not found")?;

    let Some((ntime, nbits)) = header_time_bits(db, height) else {
        return Ok(()); // can't date the block yet — leave the watermark, retry later
    };
    if ntime == 0 {
        return Ok(());
    }
    let date = unix_to_date(ntime as u64);

    // Load (or default) this day's blob under the chosen keyspace.
    let day_key = {
        let mut k = key_prefix.as_bytes().to_vec();
        k.extend_from_slice(date.as_bytes());
        k
    };
    let day_blob = db.get_cf(&cf_state, &day_key)?;
    let day_existed = day_blob.is_some();
    let mut agg: TxDayAgg = day_blob
        .and_then(|b| bincode::deserialize(&b).ok())
        .unwrap_or_default();

    let mut treasuries: Vec<crate::enrich_addresses::TreasuryPayout> = Vec::new();

    // Per-tx: inline accumulation + the prevout-join arithmetic, via the SHARED
    // primitives over the SAME source bytes the enrich reads.
    for data in block_tx_values(db, &cf_tx, height)? {
        if data.len() < 8 {
            continue;
        }
        let tx_height = i32::from_le_bytes(data[4..8].try_into().unwrap_or([0; 4]));
        // Only this block's txs, and only those the enrich would index.
        if tx_height != height || !crate::constants::should_index_transaction(tx_height) {
            continue;
        }
        let raw_tx = &data[8..];
        let mut with_header = Vec::with_capacity(4 + raw_tx.len());
        with_header.extend_from_slice(&[0u8; 4]);
        with_header.extend_from_slice(raw_tx);
        let tx = match crate::parser::deserialize_transaction(&with_header).await {
            Ok(t) => t,
            Err(_) => continue,
        };
        let tx_type = crate::tx_type::detect_transaction_type(&tx);

        // Inline counts/volumes + PoW-coinbase budget (shared with the enrich).
        if let Some(t) = accumulate_tx_inline(&mut agg, &tx, tx_type, raw_tx.len(), height, &date) {
            treasuries.push(t);
        }

        // Prevout joins: resolve each non-coinbase input, then the SHARED
        // arithmetic (identical to the enrich's pass2b → compute_tx_join).
        let mut input_sum: i64 = 0;
        let mut inputs_with_prevout: u64 = 0;
        let mut inputs_resolved: u64 = 0;
        let mut coin_days: f64 = 0.0;
        let mut p2cs_spent: i64 = 0;
        for input in &tx.inputs {
            if input.coinbase.is_some() {
                continue;
            }
            let Some(prevout) = &input.prevout else {
                continue;
            };
            inputs_with_prevout += 1;
            if let Some(r) = resolve_prevout(db, &cf_tx, &prevout.hash, prevout.n).await {
                inputs_resolved += 1;
                input_sum = input_sum.saturating_add(r.value);
                if r.prev_height >= 0 && height >= r.prev_height {
                    coin_days += (r.value as f64 / 100_000_000.0)
                        * ((height - r.prev_height) as f64 / 1440.0);
                }
                if r.is_p2cs {
                    p2cs_spent = p2cs_spent.saturating_add(r.value);
                }
            }
        }
        let out_sum: i64 = tx.outputs.iter().map(|o| o.value).sum();
        let p2cs_created: i64 = tx
            .outputs
            .iter()
            .filter(|o| matches!(classify_output(o), ScriptClassification::ColdStake { .. }))
            .map(|o| o.value)
            .sum();
        let value_balance = tx
            .sapling_data
            .as_ref()
            .map(|s| s.value_balance)
            .unwrap_or(0);
        let contrib = compute_tx_join(
            &TxJoinInputs {
                height,
                ty: tx_type,
                out_sum,
                p2cs_created,
                value_balance,
                // The enrich passes the FULL CF value length (8-byte prefix + raw
                // tx); compute_tx_join subtracts 8 for normal_tx_bytes. Match it.
                value_len: raw_tx.len() + 8,
                n_outputs: tx.outputs.len() as u32,
                input_sum,
                inputs_with_prevout,
                inputs_resolved,
                coin_days,
                p2cs_spent,
            },
            &date,
        );
        agg.p2cs_created = agg.p2cs_created.saturating_add(contrib.p2cs_created);
        agg.p2cs_spent = agg.p2cs_spent.saturating_add(contrib.p2cs_spent);
        agg.coin_days_destroyed += contrib.coin_days;
        agg.normal_tx_bytes += contrib.normal_tx_bytes;
        agg.fees_total = agg.fees_total.saturating_add(contrib.fees_total);
        agg.rewards_total = agg.rewards_total.saturating_add(contrib.rewards_total);
        agg.staker_rewards_total = agg
            .staker_rewards_total
            .saturating_add(contrib.staker_rewards_total);
        if let Some(t) = contrib.treasury {
            treasuries.push(t);
        }
    }

    // Per-block: difficulty running sum (height-ordered → byte-exact avg) + the
    // interval array (dt vs the predecessor's nTime, keyed by THIS height).
    // Side keys are namespaced per mode so shadow + real Lane I never collide.
    let (diff_pfx, int_pfx) = side_prefixes(key_prefix);
    let diffsum_key = side_key(diff_pfx, &date);
    let intervals_key = side_key(int_pfx, &date);
    let prior_blocks = agg.blocks;
    let diffsum_raw = db.get_cf(&cf_state, &diffsum_key)?;
    let intervals_raw = db.get_cf(&cf_state, &intervals_key)?;
    // Boundary-day resume: the enrich populates the day blob's avg_difficulty +
    // intervals but NOT these side keys, so on the first post-handoff block of the
    // tip's date (side keys absent, but the blob already counted blocks) we must
    // reconstruct the running state from the date's headers — otherwise the running
    // sum restarts at 0 and clobbers the enrich's correct boundary-day values.
    let (mut diffsum, mut intervals): (f64, HashMap<i32, u32>) =
        if diffsum_raw.is_none() && intervals_raw.is_none() && prior_blocks > 0 {
            seed_day_state(db, &date, height - 1)
        } else {
            (
                diffsum_raw
                    .filter(|v| v.len() == 8)
                    .map(|v| f64::from_le_bytes(v[..8].try_into().unwrap()))
                    .unwrap_or(0.0),
                intervals_raw
                    .and_then(|b| bincode::deserialize(&b).ok())
                    .unwrap_or_default(),
            )
        };
    diffsum += nbits_to_difficulty(nbits);
    agg.blocks += 1;
    agg.avg_difficulty = if agg.blocks > 0 {
        diffsum / agg.blocks as f64
    } else {
        0.0
    };

    if height >= 1 {
        if let Some((prev_t, _)) = header_time_bits(db, height - 1) {
            if prev_t != 0 {
                intervals.insert(height, ntime.saturating_sub(prev_t)); // overwrite-keyed
            }
        }
    }
    if !intervals.is_empty() {
        let mut v: Vec<u32> = intervals.values().copied().collect();
        v.sort_unstable();
        let n = v.len();
        agg.interval_p95_secs = v[(n * 95 / 100).min(n - 1)] as u64;
        agg.interval_max_secs = v[n - 1] as u64;
    }

    // Commit: day blob + side keys + watermark in ONE batch (overwrite-keyed
    // interval ⇒ replay-idempotent).
    let mut batch = WriteBatch::default();
    batch.put_cf(&cf_state, &day_key, bincode::serialize(&agg)?);
    batch.put_cf(&cf_state, &diffsum_key, diffsum.to_le_bytes());
    batch.put_cf(&cf_state, &intervals_key, bincode::serialize(&intervals)?);
    // A newly-created REAL day must be registered in the API's date index
    // (`analytics_tx_days`) or the blob is invisible to the analytics endpoints.
    // Shadow days are not API-visible, so they are not registered.
    if !day_existed && key_prefix == "analytics_tx_day:" {
        register_date(db, &cf_state, &mut batch, &date)?;
    }
    // Treasury (budget/superblock payouts): append this block's payouts to the
    // single `analytics_treasury` (or `shadow_treasury`) Vec, deduped by height so
    // replay is idempotent, in the SAME batch as the watermark.
    if !treasuries.is_empty() {
        let tkey = treasury_key(key_prefix);
        let mut existing: Vec<TreasuryPayout> = db
            .get_cf(&cf_state, tkey)?
            .and_then(|b| bincode::deserialize(&b).ok())
            .unwrap_or_default();
        let mut heights: HashSet<i32> = existing.iter().map(|t| t.height).collect();
        for p in treasuries {
            if heights.insert(p.height) {
                existing.push(p);
            }
        }
        existing.sort_by_key(|t| t.height);
        batch.put_cf(&cf_state, tkey, bincode::serialize(&existing)?);
    }
    if advance_watermark {
        batch.put_cf(&cf_state, K_WATERMARK, height.to_le_bytes());
    }
    db.write(batch)?;
    Ok(())
}

fn side_key(prefix: &[u8], date: &str) -> Vec<u8> {
    let mut k = prefix.to_vec();
    k.extend_from_slice(date.as_bytes());
    k
}

/// Lane R — recompute the day-local SET fields (`unique_stakers`, `top10_blocks`,
/// `active_addresses`) for the last `R_DAYS` calendar days from the canonical DB,
/// and set `orphan_blocks` from the PERSISTENT orphan index; RMW ONLY those four
/// fields into each window day's blob (preserving every Lane-I field and
/// `new_addresses` byte-exact). Re-deriving the sets from the canonical DB makes it
/// self-correcting on reorg. `do_mark` (live path only) first persists any NEW
/// orphans from blk-tail's ephemeral `tail_blocks` (cheap, no blocks-CF iterate);
/// `orphan_blocks` is always read from the persistent count, so a reorg-rebuilt or
/// freshly-created day blob is restored to the correct count.
pub async fn recompute_window(
    db: &Arc<DB>,
    tip: i32,
    key_prefix: &str,
    do_mark: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Gate-agnostic: `tick` checks readiness before calling; `shadow_validate`
    // calls this directly over an explicit window (with do_mark=false, so it never
    // mutates the real orphan index). The set fields are O(window) and recomputed
    // every call.
    if tip <= 0 {
        return Ok(());
    }
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    let cf_tx = db
        .cf_handle("transactions")
        .ok_or("transactions CF not found")?;

    let Some((tip_time, _)) = header_time_bits(db, tip) else {
        return Ok(());
    };
    if tip_time == 0 {
        return Ok(());
    }
    // Window = dates >= window_start (R_DAYS back from the tip's day).
    let window_start = unix_to_date(tip_time.saturating_sub(R_DAYS as u32 * SECS_PER_DAY) as u64);
    let h_lo = (tip - (R_DAYS as i32 + 1) * 1440).max(0);

    // staker counts + active address sets, per window date, from each block's txs.
    let mut day_stakers: HashMap<String, HashMap<String, u64>> = HashMap::new();
    let mut day_active: HashMap<String, HashSet<String>> = HashMap::new();
    let mut window_dates: HashSet<String> = HashSet::new();

    for h in h_lo..=tip {
        let Some((t, _)) = header_time_bits(db, h) else {
            continue;
        };
        if t == 0 {
            continue;
        }
        let date = unix_to_date(t as u64);
        if date < window_start {
            continue;
        }
        window_dates.insert(date.clone());
        for data in block_tx_values(db, &cf_tx, h)? {
            if data.len() < 8 {
                continue;
            }
            let th = i32::from_le_bytes(data[4..8].try_into().unwrap_or([0; 4]));
            if th != h || !crate::constants::should_index_transaction(th) {
                continue;
            }
            let mut with_header = Vec::with_capacity(4 + data.len() - 8);
            with_header.extend_from_slice(&[0u8; 4]);
            with_header.extend_from_slice(&data[8..]);
            let Ok(tx) = crate::parser::deserialize_transaction(&with_header).await else {
                continue;
            };
            let tx_type = crate::tx_type::detect_transaction_type(&tx);
            if tx_type == crate::tx_type::TransactionType::Coinstake {
                if let Some(addr) = tx
                    .outputs
                    .iter()
                    .find(|o| !o.address.is_empty())
                    .and_then(|o| o.address.first())
                {
                    *day_stakers
                        .entry(date.clone())
                        .or_default()
                        .entry(addr.clone())
                        .or_insert(0) += 1;
                }
            }
            for (vout_index, output) in tx.outputs.iter().enumerate() {
                if tx_type == crate::tx_type::TransactionType::Coinstake && vout_index == 0 {
                    continue;
                }
                match classify_output(output) {
                    ScriptClassification::P2PKH(a)
                    | ScriptClassification::P2SH(a)
                    | ScriptClassification::P2PK(a) => {
                        day_active.entry(date.clone()).or_default().insert(a);
                    }
                    ScriptClassification::ColdStake { staker, owner } => {
                        let e = day_active.entry(date.clone()).or_default();
                        e.insert(staker);
                        e.insert(owner);
                    }
                    _ => {}
                }
            }
        }
    }

    // Persist any NEW orphans from blk-tail's ephemeral tail_blocks (live path
    // only; cheap — no blocks-CF iterate). The count is read per-day from the
    // persistent index below, so it is restored even on a fresh/rebuilt blob.
    if do_mark {
        crate::enrich_addresses::mark_orphans(db, tip, tip_time, true)?;
    }

    // RMW the Lane-R fields into each window day's blob; orphan_blocks always comes
    // from the persistent count.
    let mut batch = WriteBatch::default();
    for date in &window_dates {
        let mut day_key = key_prefix.as_bytes().to_vec();
        day_key.extend_from_slice(date.as_bytes());
        let mut agg: TxDayAgg = match db.get_cf(&cf_state, &day_key)? {
            Some(b) => match bincode::deserialize(&b) {
                Ok(a) => a,
                Err(_) => continue,
            },
            None => continue, // Lane I creates the blob first; nothing to RMW yet
        };
        if let Some(stakers) = day_stakers.get(date) {
            agg.unique_stakers = stakers.len() as u64;
            let mut counts: Vec<u64> = stakers.values().copied().collect();
            counts.sort_unstable_by(|a, b| b.cmp(a));
            agg.top10_blocks = counts.iter().take(10).sum();
        } else {
            agg.unique_stakers = 0;
            agg.top10_blocks = 0;
        }
        agg.active_addresses = day_active.get(date).map(|s| s.len() as u64).unwrap_or(0);
        agg.orphan_blocks = crate::enrich_addresses::orphan_count(db, date);
        batch.put_cf(&cf_state, &day_key, bincode::serialize(&agg)?);
    }
    db.write(batch)?;
    Ok(())
}

/// On-demand Phase-0 validator. Re-runs Lane I (`apply_block_core`) + Lane R
/// (`recompute_window`) over the last `days_back` days into the `shadow_tx_day:`
/// keyspace — the EXACT live code, just driven explicitly — then diffs each
/// COMPLETE day against the full-enrich `analytics_tx_day:` blob field-by-field.
/// Integer fields must match exactly; the two f64 fields (`avg_difficulty`,
/// `coin_days_destroyed`) use a relative epsilon (their accumulation order differs
/// from the enrich). `new_addresses` is skipped (full-enrich-owned; Lane I never
/// writes it). Returns a human-readable report. Does NOT touch the real
/// `analytics_tx_day:` blobs or the live watermark.
pub async fn shadow_validate(
    db: &Arc<DB>,
    days_back: i64,
) -> Result<String, Box<dyn std::error::Error>> {
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    let tip = crate::chain_state::get_sync_height(db).unwrap_or(-1);
    if tip <= 0 {
        return Ok("shadow_validate: no synced tip".into());
    }
    let Some((tip_time, _)) = header_time_bits(db, tip) else {
        return Ok("shadow_validate: no tip header".into());
    };
    let tip_date = unix_to_date(tip_time as u64);
    let window_start =
        unix_to_date(tip_time.saturating_sub(days_back as u32 * SECS_PER_DAY) as u64);
    let h_lo = (tip - (days_back as i32 + 1) * 1440).max(0);

    // Collect window dates and clear the shadow keyspace (apply_block_core RMW-
    // accumulates, so a re-run must start from a clean slate).
    let mut dates: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for h in h_lo..=tip {
        if let Some((t, _)) = header_time_bits(db, h) {
            if t != 0 {
                let d = unix_to_date(t as u64);
                if d >= window_start {
                    dates.insert(d);
                }
            }
        }
    }
    {
        let mut batch = WriteBatch::default();
        for d in &dates {
            batch.delete_cf(&cf_state, side_key(b"shadow_tx_day:", d));
            batch.delete_cf(&cf_state, side_key(PFX_DIFFSUM_SHADOW, d));
            batch.delete_cf(&cf_state, side_key(PFX_INTERVALS_SHADOW, d));
        }
        db.write(batch)?;
    }

    // Lane I + Lane R into the shadow keyspace (gate/watermark-agnostic).
    for h in h_lo..=tip {
        match header_time_bits(db, h) {
            Some((t, _)) if t != 0 && unix_to_date(t as u64) >= window_start => {}
            _ => continue,
        }
        apply_block_core(db, h, "shadow_tx_day:", false).await?;
    }
    // do_mark=false: validation must not mutate the real orphan index. NOTE: this
    // means orphan_blocks is NOT meaningfully diffed by shadow_validate — both the
    // shadow and the real blob read the SAME persistent orphan_count, so the diff is
    // structurally always 0 for that field (a coverage gap, not a correctness risk).
    recompute_window(db, tip, "shadow_tx_day:", false).await?;

    // Diff each COMPLETE day (exclude the partial tip day) vs the full enrich.
    let mut report = String::new();
    let mut n_days = 0u32;
    let mut n_mismatch = 0u32;
    let mut shadow_tx_total = 0u64;
    let mut enrich_tx_total = 0u64;
    for d in &dates {
        if *d >= tip_date {
            continue; // tip day is partial — not comparable
        }
        let shadow: Option<TxDayAgg> = db
            .get_cf(&cf_state, side_key(b"shadow_tx_day:", d))?
            .and_then(|b| bincode::deserialize(&b).ok());
        let real: Option<TxDayAgg> = db
            .get_cf(&cf_state, side_key(b"analytics_tx_day:", d))?
            .and_then(|b| bincode::deserialize(&b).ok());
        match (shadow, real) {
            (Some(s), Some(r)) => {
                n_days += 1;
                shadow_tx_total += s.tx_count;
                enrich_tx_total += r.tx_count;
                let diffs = diff_agg(&s, &r);
                if diffs.is_empty() {
                    report.push_str(&format!("  {d}: OK\n"));
                } else {
                    n_mismatch += 1;
                    report.push_str(&format!("  {d}: MISMATCH\n"));
                    for line in diffs {
                        report.push_str(&format!("      {line}\n"));
                    }
                }
            }
            (None, _) => {
                report.push_str(&format!("  {d}: shadow blob MISSING\n"));
            }
            (_, None) => {
                report.push_str(&format!("  {d}: (no enrich blob — skipped)\n"));
            }
        }
    }
    // Byte-order / wiring sanity: if the enrich has txs but Lane I resolved none,
    // the 'B'→'t' lookup is broken (the whole per-tx pipeline is inert).
    let sanity = if enrich_tx_total > 0 && shadow_tx_total == 0 {
        format!(
            "  *** SANITY FAIL: Lane I resolved 0 txs but the enrich has {enrich_tx_total} — \
             txid byte-order / 'B'-index wiring is broken ***\n"
        )
    } else {
        String::new()
    };
    Ok(format!(
        "shadow_validate over {days_back}d ({} dates, {n_days} complete compared, {n_mismatch} mismatched):\n{sanity}{report}",
        dates.len()
    ))
}

/// Field-by-field diff of a shadow blob vs a full-enrich blob. Integer fields must
/// be exactly equal; f64 fields use a relative epsilon; `new_addresses` is skipped.
fn diff_agg(s: &TxDayAgg, r: &TxDayAgg) -> Vec<String> {
    let mut out = Vec::new();
    macro_rules! eq_i {
        ($f:ident) => {
            if s.$f != r.$f {
                out.push(format!(
                    "{}: shadow={} enrich={}",
                    stringify!($f),
                    s.$f,
                    r.$f
                ));
            }
        };
    }
    eq_i!(tx_count);
    eq_i!(coinbase);
    eq_i!(coinstake);
    eq_i!(payment);
    eq_i!(volume);
    eq_i!(stake_volume);
    eq_i!(blocks);
    eq_i!(tx_bytes);
    eq_i!(sapling_txs);
    eq_i!(coldstake_txs);
    eq_i!(fees_total);
    eq_i!(rewards_total);
    eq_i!(staker_rewards_total);
    eq_i!(normal_tx_bytes);
    eq_i!(p2cs_created);
    eq_i!(p2cs_spent);
    eq_i!(unique_stakers);
    eq_i!(active_addresses);
    eq_i!(top10_blocks);
    eq_i!(interval_p95_secs);
    eq_i!(interval_max_secs);
    eq_i!(orphan_blocks);
    // f64 fields: relative epsilon (accumulation order differs from the enrich).
    let d1 = s.avg_difficulty.abs().max(r.avg_difficulty.abs()).max(1.0);
    if (s.avg_difficulty - r.avg_difficulty).abs() / d1 > 1e-6 {
        out.push(format!(
            "avg_difficulty: shadow={} enrich={}",
            s.avg_difficulty, r.avg_difficulty
        ));
    }
    let d2 = s
        .coin_days_destroyed
        .abs()
        .max(r.coin_days_destroyed.abs())
        .max(1.0);
    if (s.coin_days_destroyed - r.coin_days_destroyed).abs() / d2 > 1e-6 {
        out.push(format!(
            "coin_days_destroyed: shadow={} enrich={}",
            s.coin_days_destroyed, r.coin_days_destroyed
        ));
    }
    // new_addresses intentionally skipped (full-enrich-owned; Lane I never writes it).
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocksdb::{Options, DB};

    /// The single-flight guard must release on drop and be re-acquirable — pins the
    /// RAII contract that closes the prior put_cf(...)? guard-leak.
    #[test]
    fn reenrich_guard_releases_on_drop() {
        let g = ReenrichGuard::try_acquire().expect("first acquire succeeds");
        assert!(ReenrichGuard::try_acquire().is_none(), "blocked while held");
        drop(g);
        let g2 = ReenrichGuard::try_acquire().expect("re-acquirable after drop");
        drop(g2);
        assert!(ReenrichGuard::try_acquire().is_some(), "free again");
    }

    /// A `'t'`-CF value: `[version 4][height 4 LE][raw tx bytes]`.
    fn t_value(height: i32) -> Vec<u8> {
        let mut v = vec![0u8; 4];
        v.extend_from_slice(&height.to_le_bytes());
        v.extend_from_slice(b"raw-tx-placeholder-bytes");
        v
    }

    /// A non-palindromic 32-byte display-order txid (display != reverse).
    fn mk_txid(seed: u8) -> [u8; 32] {
        let mut t = [seed; 32];
        t[0] = seed;
        t[31] = seed ^ 0xff;
        t
    }

    // block_tx_values must resolve the correct 't' value for ALL THREE 'B' layouts:
    // parse 13-byte (hex-of-internal), monitor 13-byte (hex-of-display), and the
    // degraded-rebuild 9-byte (raw display). The 't' key is always b't'+display.
    #[test]
    fn block_tx_values_resolves_all_b_layouts() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = std::sync::Arc::new(DB::open_cf(&opts, temp.path(), ["transactions"]).unwrap());
        let cf = db.cf_handle("transactions").unwrap();

        let (h_p, d_p) = (100i32, mk_txid(1)); // parse: hex-of-internal
        let (h_m, d_m) = (200i32, mk_txid(2)); // monitor: hex-of-display
        let (h_r, d_r) = (300i32, mk_txid(3)); // rebuild: raw display (9-byte key)

        // The 't' key is b't'+display for every writer.
        for (d, h) in [(d_p, h_p), (d_m, h_m), (d_r, h_r)] {
            let mut k = vec![b't'];
            k.extend_from_slice(&d);
            db.put_cf(&cf, &k, t_value(h)).unwrap();
        }

        let b13 = |h: i32, idx: u64| {
            let mut k = vec![b'B'];
            k.extend_from_slice(&h.to_le_bytes());
            k.extend_from_slice(&idx.to_le_bytes());
            k
        };
        let b9 = |h: i32, idx: u32| {
            let mut k = vec![b'B'];
            k.extend_from_slice(&h.to_le_bytes());
            k.extend_from_slice(&idx.to_le_bytes());
            k
        };
        // parse path: value = hex(internal) where internal = display.rev()
        let internal_p: Vec<u8> = d_p.iter().rev().cloned().collect();
        db.put_cf(&cf, b13(h_p, 0), hex::encode(&internal_p).as_bytes())
            .unwrap();
        // monitor path: value = hex(display)
        db.put_cf(&cf, b13(h_m, 0), hex::encode(d_m).as_bytes())
            .unwrap();
        // rebuild path: value = raw 32-byte display suffix (9-byte key)
        db.put_cf(&cf, b9(h_r, 0), d_r).unwrap();

        assert_eq!(
            block_tx_values(&db, &cf, h_p).unwrap(),
            vec![t_value(h_p)],
            "parse 13-byte (hex-of-internal) must resolve via the reversed probe"
        );
        assert_eq!(
            block_tx_values(&db, &cf, h_m).unwrap(),
            vec![t_value(h_m)],
            "monitor 13-byte (hex-of-display) must resolve via the raw probe"
        );
        assert_eq!(
            block_tx_values(&db, &cf, h_r).unwrap(),
            vec![t_value(h_r)],
            "rebuild 9-byte (raw display) must resolve directly"
        );
    }

    // A duplicate 'B' entry (same txid at a shifted index, e.g. a stale reorg
    // remnant) must be deduped to a single resolved value.
    #[test]
    fn block_tx_values_dedups_shifted_index() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = std::sync::Arc::new(DB::open_cf(&opts, temp.path(), ["transactions"]).unwrap());
        let cf = db.cf_handle("transactions").unwrap();

        let (h, d) = (100i32, mk_txid(7));
        let mut k = vec![b't'];
        k.extend_from_slice(&d);
        db.put_cf(&cf, &k, t_value(h)).unwrap();

        for idx in [0u64, 5u64] {
            let mut bk = vec![b'B'];
            bk.extend_from_slice(&h.to_le_bytes());
            bk.extend_from_slice(&idx.to_le_bytes());
            db.put_cf(&cf, &bk, hex::encode(d).as_bytes()).unwrap();
        }
        assert_eq!(
            block_tx_values(&db, &cf, h).unwrap().len(),
            1,
            "duplicate txid deduped"
        );
    }
}
