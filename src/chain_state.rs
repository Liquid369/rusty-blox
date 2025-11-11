/// Chain State Tracking
/// 
/// Manages blockchain state metadata:
/// - Current sync height and hash
/// - Sync progress percentage
/// - Network statistics
/// - Reorg detection markers

use std::sync::Arc;
use rocksdb::DB;
use serde::{Serialize, Deserialize};

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
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    // Get sync height
    let height = match db.get_cf(&cf_state, b"sync_height")? {
        Some(bytes) => i32::from_le_bytes(bytes.as_slice().try_into()?),
        None => 0,
    };
    
    // Get block hash at current height
    let cf_metadata = db.cf_handle("chain_metadata")
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

/// Update network height (from RPC)
pub fn set_network_height(db: &Arc<DB>, height: i32) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    db.put_cf(&cf_state, b"network_height", &height.to_le_bytes())?;
    Ok(())
}

/// Update sync height
pub fn set_sync_height(db: &Arc<DB>, height: i32) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    db.put_cf(&cf_state, b"sync_height", &height.to_le_bytes())?;
    Ok(())
}

/// Mark reorg point
pub fn mark_reorg(db: &Arc<DB>, height: i32, reason: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    let key = format!("reorg_{}", height);
    let value = format!("{}", reason);
    
    db.put_cf(&cf_state, key.as_bytes(), value.as_bytes())?;
    
    println!("REORG marked at height {}: {}", height, reason);
    
    Ok(())
}
