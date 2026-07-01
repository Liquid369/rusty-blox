use rocksdb::DB;
use serde::{Deserialize, Serialize};
/// Chain State Tracking
///
/// Manages blockchain state metadata:
/// - Current sync height and hash
/// - Sync progress percentage
/// - Network statistics
/// - Reorg detection markers
use std::sync::Arc;
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainState {
    pub height: i32,
    pub hash: String,
    pub synced: bool,
    pub sync_percentage: f64,
    pub network_height: Option<i32>,
}

/// Get current chain state
pub fn get_chain_state(db: &Arc<DB>) -> Result<ChainState, Box<dyn std::error::Error>> {
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;

    // Get sync height
    let height = match db.get_cf(&cf_state, b"sync_height")? {
        Some(bytes) => i32::from_le_bytes(bytes.as_slice().try_into()?),
        None => 0,
    };

    // Get block hash at current height
    let cf_metadata = db
        .cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;

    let height_key = height.to_le_bytes().to_vec();
    let hash = match db.get_cf(&cf_metadata, &height_key)? {
        Some(hash_bytes) => hex::encode(&hash_bytes),
        None => String::new(),
    };

    // Get network height if available
    let network_height = match db.get_cf(&cf_state, b"network_height")? {
        Some(bytes) => Some(i32::from_le_bytes(bytes.as_slice().try_into()?)),
        None => None,
    };

    // Calculate sync percentage
    let sync_percentage = if let Some(net_height) = network_height {
        if net_height > 0 {
            (height as f64 / net_height as f64) * 100.0
        } else {
            100.0
        }
    } else {
        100.0
    };

    let synced = network_height.map(|nh| height >= nh - 2).unwrap_or(true);

    Ok(ChainState {
        height,
        hash,
        synced,
        sync_percentage,
        network_height,
    })
}

/// A tip older than this (seconds) with a complete index means the monitor has
/// stopped connecting blocks — a frozen tip (dead RPC / stalled sync). ~20 PIVX
/// blocks at the 60s target. ponytail: tune to the chain's real block cadence.
pub const STALE_TIP_THRESHOLD_SECS: i64 = 1200;

/// Age of the indexed tip in seconds, or `None` when no staleness judgement can
/// be made — the index is still building (so a slow initial sync isn't reported
/// "stale") or no block time has been recorded yet. Clamped at 0 for clock skew.
pub fn tip_age_seconds(last_block_time: i64, now: i64, index_complete: bool) -> Option<i64> {
    if !index_complete || last_block_time <= 0 {
        return None;
    }
    Some((now - last_block_time).max(0))
}

/// Update network height (from RPC)
pub fn set_network_height(db: &Arc<DB>, height: i32) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;

    db.put_cf(&cf_state, b"network_height", height.to_le_bytes())?;
    Ok(())
}

/// Update sync height
pub fn set_sync_height(db: &Arc<DB>, height: i32) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;

    db.put_cf(&cf_state, b"sync_height", height.to_le_bytes())?;
    Ok(())
}

/// Read the current stored sync height (0 if unset). Cheap point lookup,
/// used to seed the throttled progress writer so it stays monotonic.
pub fn get_sync_height(db: &Arc<DB>) -> Result<i32, Box<dyn std::error::Error>> {
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    match db.get_cf(&cf_state, b"sync_height")? {
        Some(bytes) => Ok(i32::from_le_bytes(bytes.as_slice().try_into()?)),
        None => Ok(0),
    }
}

/// chain_state key: the addr_index on-disk format version. Stamped to
/// `parser::ADDR_INDEX_FORMAT_VERSION` ONLY by a successful leveldb-backed full enrich
/// (never the chainstate or !fast_sync paths). Absent / < CURRENT ⇒ a legacy index.
pub const ADDR_INDEX_VERSION_KEY: &[u8] = b"addr_index_format_version";

/// Read the persisted addr_index format version (0 if unset/legacy).
pub fn get_addr_index_version(db: &Arc<DB>) -> u32 {
    db.cf_handle("chain_state")
        .and_then(|cf| db.get_cf(&cf, ADDR_INDEX_VERSION_KEY).ok().flatten())
        .filter(|v| v.len() == 4)
        .map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
        .unwrap_or(0)
}

/// Is the addr_index complete AND in the current on-disk format — i.e. safe to serve?
/// The API data handlers gate on this (return 503 "reindexing" while false), because
/// the web server is released independently of the sync/enrich thread, so an in-place
/// v1→v2 upgrade would otherwise serve old-stride bytes at HTTP 200.
pub fn addr_index_ready(db: &Arc<DB>) -> bool {
    let complete = db
        .cf_handle("chain_state")
        .and_then(|cf| db.get_cf(&cf, b"address_index_complete").ok().flatten())
        .map(|v| v.first() == Some(&1u8))
        .unwrap_or(false);
    complete && get_addr_index_version(db) == crate::parser::ADDR_INDEX_FORMAT_VERSION
}

/// Mark reorg point
pub fn mark_reorg(
    db: &Arc<DB>,
    height: i32,
    reason: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;

    let key = format!("reorg_{height}");
    let value = reason.to_string();

    db.put_cf(&cf_state, key.as_bytes(), value.as_bytes())?;

    warn!(height = height, reason = %reason, "REORG marked in chain state");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocksdb::{Options, DB};

    fn open_db() -> (tempfile::TempDir, Arc<DB>) {
        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = Arc::new(
            DB::open_cf(
                &opts,
                temp.path(),
                ["chain_state", "addr_index", "transactions"],
            )
            .unwrap(),
        );
        (temp, db)
    }

    fn set_complete(db: &Arc<DB>) {
        let cf = db.cf_handle("chain_state").unwrap();
        db.put_cf(&cf, b"address_index_complete", [1u8]).unwrap();
    }

    fn stamp_version(db: &Arc<DB>, v: u32) {
        let cf = db.cf_handle("chain_state").unwrap();
        db.put_cf(&cf, ADDR_INDEX_VERSION_KEY, v.to_le_bytes())
            .unwrap();
    }

    /// Tip-staleness gating: a still-building index or an unrecorded block time
    /// yields no judgement (None) so a syncing node isn't reported "stale"; once
    /// complete, age is now - tip_time, clamped at 0 for clock skew.
    #[test]
    fn tip_age_seconds_gates_and_clamps() {
        // Index not complete yet → no staleness judgement.
        assert_eq!(tip_age_seconds(1_000, 9_999, false), None);
        // Complete but no block time recorded → None.
        assert_eq!(tip_age_seconds(0, 9_999, true), None);
        // Complete + recorded + caught up → small positive age.
        assert_eq!(tip_age_seconds(1_000, 1_050, true), Some(50));
        // Complete + frozen tip → large age (the stale case).
        assert_eq!(tip_age_seconds(1_000, 5_000, true), Some(4_000));
        // Clock skew (tip time ahead of now) → clamped to 0, not negative.
        assert_eq!(tip_age_seconds(9_000, 1_000, true), Some(0));
    }

    /// MS-1: the API 503-gate decision matrix. addr_index_ready is true ONLY when the
    /// index is both complete AND stamped with the current format version — so a legacy
    /// (unstamped or older-version) index, or an in-progress rebuild, returns 503.
    #[test]
    fn addr_index_ready_requires_complete_and_current_version() {
        let (_t, db) = open_db();

        // Fresh DB: nothing set → not ready; absent version reads as 0.
        assert!(!addr_index_ready(&db), "empty DB must not be ready");
        assert_eq!(get_addr_index_version(&db), 0, "absent version reads as 0");

        // Complete but UNSTAMPED (a legacy v1 index, or chainstate/!fast_sync) → 503.
        set_complete(&db);
        assert!(
            !addr_index_ready(&db),
            "complete but unstamped (legacy) must 503"
        );

        // Complete + a WRONG (older) version → 503.
        stamp_version(&db, 1);
        assert_eq!(get_addr_index_version(&db), 1);
        assert!(!addr_index_ready(&db), "complete but v1 must 503");

        // Complete + the CURRENT version → ready (serve).
        stamp_version(&db, crate::parser::ADDR_INDEX_FORMAT_VERSION);
        assert!(
            addr_index_ready(&db),
            "complete + current version must serve"
        );

        // Version current but NOT complete (mid-rebuild) → 503.
        let cf = db.cf_handle("chain_state").unwrap();
        db.delete_cf(&cf, b"address_index_complete").unwrap();
        assert!(
            !addr_index_ready(&db),
            "current version but incomplete must 503"
        );
    }
}
