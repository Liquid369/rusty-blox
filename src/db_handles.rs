use std::sync::Arc;
use rocksdb::DB;

/// Cached column family handles for efficient access
/// 
/// Stores CF names and provides methods to get handles.
/// This eliminates repeated HashMap lookups in hot paths.
#[derive(Clone)]
pub struct DbHandles {
    db: Arc<DB>,
}

impl DbHandles {
    /// Create new DbHandles
    /// 
    /// Validates that all required column families exist at startup
    pub fn new(db: Arc<DB>) -> Result<Self, String> {
        // Validate all required CFs exist
        let required_cfs = vec![
            "blocks",
            "transactions",
            "addr_index",
            "utxo",
            "chain_metadata",
            "chain_state",
            "pubkey",
        ];
        
        for cf_name in required_cfs {
            if db.cf_handle(cf_name).is_none() {
                return Err(format!("{} column family not found", cf_name));
            }
        }
        
        Ok(Self { db })
    }
    
    /// Get database reference
    pub fn db(&self) -> &Arc<DB> {
        &self.db
    }
}
