/// Sync Service - Manages blockchain synchronization
/// 
/// Two modes:
/// 1. Initial Sync: Fast bulk import from .dat files
/// 2. Live Sync: Real-time monitoring via RPC connection
/// 
/// Automatically detects which mode to use based on current chain state

use std::sync::Arc;
use std::path::PathBuf;
use std::time::Duration;
use rocksdb::DB;
use tokio::fs;

use crate::types::AppState;
use crate::parallel::process_files_parallel;
use crate::config::get_global_config;
use crate::monitor::run_block_monitor;
use crate::websocket::EventBroadcaster;

/// Get current sync status from database
async fn get_sync_status(db: &Arc<DB>) -> Result<SyncStatus, Box<dyn std::error::Error>> {
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    // First check chain_state CF for sync marker
    match db.get_cf(&cf_state, b"sync_height")? {
        Some(bytes) => {
            let height = i32::from_le_bytes(bytes.as_slice().try_into()?);
            return Ok(SyncStatus::Synced { height });
        }
        None => {
            // No sync marker, check if we have any blocks indexed
            let cf_metadata = db.cf_handle("chain_metadata")
                .ok_or("chain_metadata CF not found")?;
            
            // Check if block at height 0 exists
            let height_key = 0i32.to_le_bytes().to_vec();
            match db.get_cf(&cf_metadata, &height_key)? {
                Some(_) => {
                    // We have blocks but no sync marker - find highest block
                    let mut height: i32 = 0;
                    loop {
                        let key = height.to_le_bytes().to_vec();
                        match db.get_cf(&cf_metadata, &key)? {
                            Some(_) => height += 1,
                            None => break,
                        }
                        
                        // Safety limit
                        if height > 10_000_000 {
                            break;
                        }
                    }
                    
                    let final_height = height - 1;
                    
                    // Set the sync marker for next time
                    set_sync_height(db, final_height).await?;
                    
                    return Ok(SyncStatus::Synced { height: final_height });
                }
                None => {
                    return Ok(SyncStatus::NeedInitialSync);
                }
            }
        }
    }
}

/// Set sync status in database
async fn set_sync_height(db: &Arc<DB>, height: i32) -> Result<(), Box<dyn std::error::Error>> {
    let cf = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    db.put_cf(&cf, b"sync_height", &height.to_le_bytes())?;
    Ok(())
}

#[derive(Debug, Clone)]
pub enum SyncStatus {
    NeedInitialSync,
    Synced { height: i32 },
}

/// Run initial sync from .dat files
async fn run_initial_sync(
    blk_dir: PathBuf,
    db: Arc<DB>,
    state: AppState,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = get_global_config();
    let max_concurrent = config.get_int("sync.parallel_files").unwrap_or(8) as usize;
    
    // Read all .dat files
    let mut dir_entries = fs::read_dir(&blk_dir).await?;
    let mut entries = Vec::new();
    
    while let Ok(Some(entry)) = dir_entries.next_entry().await {
        entries.push(entry.path());
    }
    
    // Process files in parallel
    process_files_parallel(entries, Arc::clone(&db), state.clone(), max_concurrent).await?;
    
    // Find highest block height
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    let mut height: i32 = 0;
    loop {
        let height_key = height.to_le_bytes().to_vec();
        match db.get_cf(&cf_metadata, &height_key)? {
            Some(_) => height += 1,
            None => break,
        }
        
        // Safety limit
        if height > 10_000_000 {
            break;
        }
    }
    
    let final_height = height - 1;
    // Mark sync complete
    set_sync_height(&db, final_height).await?;
    
    Ok(())
}

/// Monitor for new blocks via RPC or file polling
async fn run_live_sync(
    _blk_dir: PathBuf,
    db: Arc<DB>,
    _state: AppState,
    _current_height: i32,
    broadcaster: Option<Arc<EventBroadcaster>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Use RPC-based block monitor
    run_block_monitor(db, 5, broadcaster).await?;
    
    Ok(())
}

/// Main sync service - automatically chooses mode
pub async fn run_sync_service(
    blk_dir: PathBuf,
    db: Arc<DB>,
    broadcaster: Option<Arc<EventBroadcaster>>,
) -> Result<(), Box<dyn std::error::Error>> {
    
    let state = AppState {
        db: Arc::clone(&db),
    };
    
    // Check sync status
    match get_sync_status(&db).await? {
        SyncStatus::NeedInitialSync => {
            // Run initial sync once
            run_initial_sync(blk_dir.clone(), Arc::clone(&db), state.clone()).await?;
            
            // Find final height after sync
            let cf_metadata = db.cf_handle("chain_metadata")
                .ok_or("chain_metadata CF not found")?;
            
            let mut height: i32 = 0;
            loop {
                let key = height.to_le_bytes().to_vec();
                match db.get_cf(&cf_metadata, &key)? {
                    Some(_) => height += 1,
                    None => break,
                }
                if height > 10_000_000 {
                    break;
                }
            }
            let final_height = height - 1;
            
            // Then switch to live mode
            run_live_sync(blk_dir, db, state, final_height, broadcaster).await?;
        }
        SyncStatus::Synced { height } => {
            // Go straight to live mode
            run_live_sync(blk_dir, db, state, height, broadcaster).await?;
        }
    }
    
    Ok(())
}
