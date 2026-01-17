/// Database Size Sampler - Background task for monitoring RocksDB size
/// 
/// Efficiently samples database size per column family without blocking
/// the main thread. Updates metrics periodically for Prometheus/Grafana.

use std::sync::Arc;
use std::time::Duration;
use rocksdb::DB;
use tokio::time::interval;
use tracing::{info, warn};
use crate::metrics;

/// Column families to monitor
const COLUMN_FAMILIES: &[&str] = &[
    "blocks",
    "transactions",
    "addr_index",
    "utxo",
    "utxo_undo",
    "chain_metadata",
    "chain_state",
    "pubkey",
];

/// Background task that samples database size every N seconds
/// 
/// Uses RocksDB property APIs for efficient size estimation:
/// - "rocksdb.estimate-live-data-size" - fast estimate
/// - "rocksdb.total-sst-files-size" - SST file size
/// 
/// Runs in a separate tokio task and never blocks the main thread.
pub async fn start_db_size_sampler(db: Arc<DB>, interval_secs: u64) {
    let mut ticker = interval(Duration::from_secs(interval_secs));
    
    info!(
        interval_seconds = interval_secs,
        column_families = ?COLUMN_FAMILIES,
        "Starting database size sampler"
    );
    
    loop {
        ticker.tick().await;
        
        // Sample each column family
        for cf_name in COLUMN_FAMILIES {
            if let Some(cf_handle) = db.cf_handle(cf_name) {
                // Try to get estimate-live-data-size (fast, approximate)
                match db.property_int_value_cf(&cf_handle, "rocksdb.estimate-live-data-size") {
                    Ok(Some(size_bytes)) => {
                        metrics::set_db_size_bytes(cf_name, size_bytes);
                    }
                    Ok(None) => {
                        // Property not available, try total-sst-files-size
                        match db.property_int_value_cf(&cf_handle, "rocksdb.total-sst-files-size") {
                            Ok(Some(size_bytes)) => {
                                metrics::set_db_size_bytes(cf_name, size_bytes);
                            }
                            _ => {
                                warn!(cf = cf_name, "Could not get database size");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(cf = cf_name, error = ?e, "Failed to query database size");
                    }
                }
            } else {
                warn!(cf = cf_name, "Column family handle not found");
            }
        }
    }
}

/// Sample database size once (synchronous) - useful for on-demand checks
/// Returns total size across all column families in bytes
pub fn sample_total_db_size_sync(db: &Arc<DB>) -> u64 {
    let mut total_size = 0u64;
    
    for cf_name in COLUMN_FAMILIES {
        if let Some(cf_handle) = db.cf_handle(cf_name) {
            if let Ok(Some(size)) = db.property_int_value_cf(&cf_handle, "rocksdb.estimate-live-data-size") {
                total_size += size;
                metrics::set_db_size_bytes(cf_name, size);
            }
        }
    }
    
    total_size
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_column_families_list() {
        // Ensure we're tracking all known CFs
        assert!(COLUMN_FAMILIES.contains(&"blocks"));
        assert!(COLUMN_FAMILIES.contains(&"transactions"));
        assert!(COLUMN_FAMILIES.contains(&"addr_index"));
        assert_eq!(COLUMN_FAMILIES.len(), 8);
    }
}
