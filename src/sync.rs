/// Sync Service - Manages blockchain synchronization
/// 
/// Two modes:
/// 1. Initial Sync: Fast bulk import from leveldb block index + blk*.dat files
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
use crate::leveldb_index::build_canonical_chain_from_leveldb;
use crate::chain_state::set_network_height;
use pivx_rpc_rs::BitcoinRpcClient;

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

/// Fetch and store network height from RPC
async fn update_network_height(db: &Arc<DB>) {
    let config = get_global_config();
    
    // Get RPC configuration
    let rpc_host = match config.get_string("rpc.host") {
        Ok(host) => host,
        Err(_) => return,
    };
    let rpc_user = match config.get_string("rpc.user") {
        Ok(user) => user,
        Err(_) => return,
    };
    let rpc_pass = match config.get_string("rpc.pass") {
        Ok(pass) => pass,
        Err(_) => return,
    };
    
    // Create RPC client
    let rpc_client = BitcoinRpcClient::new(
        rpc_host,
        Some(rpc_user),
        Some(rpc_pass),
        3,     // Max retries
        10,    // Connection timeout
        30000, // Read timeout
    );
    
    // Get network height
    match rpc_client.getblockcount() {
        Ok(height) => {
            if let Err(e) = set_network_height(db, height as i32) {
                eprintln!("Failed to set network height: {}", e);
            } else {
                println!("📡 Network height: {}", height);
            }
        }
        Err(e) => {
            eprintln!("⚠️  Failed to get network height from RPC: {}", e);
        }
    }
}

/// Run initial sync from leveldb block index + blk*.dat files
async fn run_initial_sync_leveldb(
    blk_dir: PathBuf,
    db: Arc<DB>,
    state: AppState,
) -> Result<i32, Box<dyn std::error::Error>> {
    println!("\n🚀 Starting FAST initial sync using leveldb block index...\n");
    
    // Get PIVX data directory for leveldb
    let config = get_global_config();
    let pivx_data_dir = config
        .get_string("paths.pivx_data_dir")
        .unwrap_or_else(|_| {
            // Default macOS location
            let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/liquid".to_string());
            format!("{}/Library/Application Support/PIVX", home)
        });
    
    let leveldb_path = format!("{}/blocks/index", pivx_data_dir);
    let copy_leveldb_path = "/tmp/pivx_index_current";
    
    println!("📍 PIVX data directory: {}", pivx_data_dir);
    println!("📍 Source leveldb: {}", leveldb_path);
    println!("📍 Blk files directory: {}", blk_dir.display());
    
    // Check if we already have a copy
    let needs_copy = if std::path::Path::new(copy_leveldb_path).exists() {
        println!("📦 Found existing leveldb copy at {}", copy_leveldb_path);
        println!("   Using existing copy (delete it to force a fresh copy)");
        false
    } else {
        println!("📋 No leveldb copy found, will create one...");
        true
    };
    
    let final_leveldb_path = if needs_copy {
        // Copy leveldb to /tmp so we can read it while daemon is running
        println!("📋 Copying leveldb from {} to {}", leveldb_path, copy_leveldb_path);
        println!("   This may take a minute or two...");
        
        // Remove old copy if exists
        if std::path::Path::new(copy_leveldb_path).exists() {
            std::fs::remove_dir_all(copy_leveldb_path)?;
        }
        
        // Use cp command for faster copy
        let copy_result = std::process::Command::new("cp")
            .arg("-R")
            .arg(&leveldb_path)
            .arg(copy_leveldb_path)
            .output()?;
        
        if !copy_result.status.success() {
            return Err(format!("Failed to copy leveldb: {}", 
                String::from_utf8_lossy(&copy_result.stderr)).into());
        }
        
        println!("✅ Leveldb copy complete!");
        copy_leveldb_path.to_string()
    } else {
        copy_leveldb_path.to_string()
    };
    
    println!("📍 Reading from leveldb: {}", final_leveldb_path);
    
    // Build canonical chain from leveldb
    let canonical_chain = build_canonical_chain_from_leveldb(&final_leveldb_path)?;
    
    let chain_len = canonical_chain.len();
    println!("\n📊 Canonical chain has {} blocks", chain_len);
    
    // Store canonical chain metadata in our DB
    // This will guide the parallel block processor to know which blocks to index
    println!("\n📦 Storing canonical chain metadata...");
    
    let cf_metadata = db.cf_handle("chain_metadata").ok_or("chain_metadata CF not found")?;
    
    for (height, hash) in &canonical_chain {
        let height_key = (*height as i32).to_le_bytes();
        // Store reversed hash (display format) for consistency with existing code
        let mut display_hash = hash.clone();
        display_hash.reverse();
        db.put_cf(&cf_metadata, &height_key, &display_hash)?;
        
        // ALSO store the reverse mapping: 'h' + internal_hash → height
        // This allows blk file processing to look up heights efficiently
        let mut hash_key = vec![b'h'];
        hash_key.extend_from_slice(hash);  // Internal format (not reversed)
        db.put_cf(&cf_metadata, &hash_key, &height_key)?;
        
        if *height % 500_000 == 0 {
            println!("  Stored metadata for height {}", height);
        }
    }
    
    println!("✅ Canonical chain metadata stored!");
    println!("   The parallel block processor will now index {} blocks from blk*.dat files", chain_len);
    
    // Now process blk*.dat files to get the actual block data
    // The parallel processor will index all blocks it finds
    // Because we stored the canonical chain metadata above, we know exactly which blocks to index
    println!("\n📂 Processing blk*.dat files for block data...");
    println!("   This will read all blocks and index the {} canonical blocks", chain_len);
    
    let max_concurrent = config.get_int("sync.parallel_files").unwrap_or(8) as usize;
    
    let mut dir_entries = fs::read_dir(&blk_dir).await?;
    let mut entries = Vec::new();
    
    while let Ok(Some(entry)) = dir_entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("dat") {
            if path.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("blk")).unwrap_or(false) {
                entries.push(path);
            }
        }
    }
    
    println!("Found {} blk*.dat files", entries.len());
    
    // Process files in parallel (this will index all blocks, including orphans)
    // The chainwork calculation later will determine which ones are canonical
    process_files_parallel(entries, Arc::clone(&db), state, max_concurrent).await?;
    
    let final_height = (chain_len - 1) as i32;
    
    println!("\n✅ Initial sync complete! Final height: {}", final_height);
    println!("   All {} canonical blocks are now indexed", chain_len);
    println!("   The explorer will now switch to RPC mode to catch new blocks");
    
    // Mark sync complete
    set_sync_height(&db, final_height).await?;
    
    Ok(final_height)
}

/// Run initial sync from .dat files (OLD METHOD - kept as fallback)
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

/// Monitor for new blocks - HYBRID approach
async fn run_live_sync(
    blk_dir: PathBuf,
    db: Arc<DB>,
    state: AppState,
    current_height: i32,
    broadcaster: Option<Arc<EventBroadcaster>>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting HYBRID sync mode...");
    println!("Current DB height: {}", current_height);
    
    // Step 1: Sync from blk*.dat files to get caught up faster
    println!("Phase 1: Syncing from blk*.dat files (fast)...");
    
    let config = get_global_config();
    let max_concurrent = config.get_int("sync.parallel_files").unwrap_or(8) as usize;
    
    // Read all .dat files
    let mut dir_entries = fs::read_dir(&blk_dir).await?;
    let mut entries = Vec::new();
    
    while let Ok(Some(entry)) = dir_entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("dat") {
            entries.push(path);
        }
    }
    
    if !entries.is_empty() {
        println!("Found {} blk*.dat files, processing in parallel...", entries.len());
        process_files_parallel(entries, Arc::clone(&db), state.clone(), max_concurrent).await?;
        
        println!("✅ Finished processing blk*.dat files!");
    }
    
    // Step 2: Switch to RPC for new blocks (blk files won't have the latest)
    println!("Phase 2: Switching to RPC for new blocks...");
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
    
    // Fetch and store network height from RPC early
    update_network_height(&db).await;
    
    // Check sync status
    match get_sync_status(&db).await? {
        SyncStatus::NeedInitialSync => {
            println!("\n🆕 No existing index found - running initial sync\n");
            
            // Try leveldb-based sync first (MUCH faster!)
            let final_height = match run_initial_sync_leveldb(blk_dir.clone(), Arc::clone(&db), state.clone()).await {
                Ok(height) => {
                    println!("✅ Leveldb-based sync succeeded! Final height: {}", height);
                    height
                }
                Err(e) => {
                    println!("⚠️  Leveldb sync failed: {}", e);
                    println!("⚠️  Falling back to traditional blk file scan...");
                    
                    // Fallback to traditional method
                    run_initial_sync(blk_dir.clone(), Arc::clone(&db), state.clone()).await?;
                    
                    // Find final height
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
                    height - 1
                }
            };
            
            // Then switch to live mode
            run_live_sync(blk_dir, db, state, final_height, broadcaster).await?;
        }
        SyncStatus::Synced { height } => {
            println!("\n✅ Existing index found at height {}\n", height);
            // Go straight to live mode
            run_live_sync(blk_dir, db, state, height, broadcaster).await?;
        }
    }
    
    Ok(())
}
