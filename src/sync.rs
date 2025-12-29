/// Sync Service - Manages blockchain synchronization
/// 
/// Two modes:
/// 1. Initial Sync: Fast bulk import from leveldb block index + blk*.dat files
/// 2. Live Sync: Real-time monitoring via RPC connection
/// 
/// Automatically detects which mode to use based on current chain state

use std::sync::Arc;
use std::path::PathBuf;
use rocksdb::DB;
use tokio::fs;

use crate::types::AppState;
use crate::cache::CacheManager;
use crate::parallel::process_files_parallel;
use crate::config::get_global_config;
use crate::monitor::run_block_monitor;
use crate::websocket::EventBroadcaster;
use crate::leveldb_index::build_canonical_chain_from_leveldb;
use crate::chain_state::set_network_height;
use crate::pivx_copy::get_block_index_path;
use pivx_rpc_rs::BitcoinRpcClient;
use crate::repair;

/// Get current sync status from database
async fn get_sync_status(db: &Arc<DB>) -> Result<SyncStatus, Box<dyn std::error::Error>> {
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    // First check chain_state CF for sync marker
    match db.get_cf(&cf_state, b"sync_height")? {
        Some(bytes) => {
            let height = i32::from_le_bytes(bytes.as_slice().try_into()?);
            Ok(SyncStatus::Synced { height })
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
                    
                    Ok(SyncStatus::Synced { height: final_height })
                }
                None => {
                    Ok(SyncStatus::NeedInitialSync)
                }
            }
        }
    }
}

/// Set sync status in database
async fn set_sync_height(db: &Arc<DB>, height: i32) -> Result<(), Box<dyn std::error::Error>> {
    let cf = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    db.put_cf(&cf, b"sync_height", height.to_le_bytes())?;
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
    let pivx_blocks_dir = config
        .get_string("paths.blk_dir")
        .unwrap_or_else(|_| {
            // Default macOS location
            let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/liquid".to_string());
            format!("{}/Library/Application Support/PIVX/blocks", home)
        });
    
    // Get copy directory from config
    let block_index_copy_dir = config
        .get_string("paths.block_index_copy_dir")
        .ok();
    
    println!("� PIVX blocks directory: {}", pivx_blocks_dir);
    
    // Get the block index path (copies if needed)
    let leveldb_path = get_block_index_path(
        &pivx_blocks_dir,
        block_index_copy_dir.as_deref(),
    )?;
    
    println!("📍 Reading block index from: {}", leveldb_path);
    println!("📍 Blk files directory: {}", blk_dir.display());
    
    // Build canonical chain from leveldb
    let canonical_chain = build_canonical_chain_from_leveldb(&leveldb_path)?;

    let chain_len = canonical_chain.len();
    let leveldb_height = (chain_len - 1) as i32;
    println!("\n📊 Canonical chain from LevelDB: {} blocks (height 0 to {})", chain_len, leveldb_height);
    
    // Verify against RPC to see if daemon has synced further
    let rpc_host = config.get_string("rpc.host").unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config.get_string("rpc.user").unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config.get_string("rpc.pass").unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    let rpc_client = BitcoinRpcClient::new(rpc_host, Some(rpc_user), Some(rpc_pass), 3, 10, 30000);
    
    match rpc_client.getblockcount() {
        Ok(network_height) => {
            let blocks_behind = network_height as i32 - leveldb_height;
            if blocks_behind > 0 {
                println!("⚠️  LevelDB copy is {} blocks behind current daemon height {}", blocks_behind, network_height);
                println!("   Will fetch missing blocks via RPC after processing blk files");
            } else {
                println!("✅ LevelDB copy is current (matches daemon height {})", network_height);
            }
        }
        Err(e) => {
            println!("⚠️  Could not verify LevelDB freshness via RPC: {}", e);
            println!("   Proceeding with LevelDB data as-is");
        }
    }
    
    println!("\n📊 Using canonical chain: {} blocks", chain_len);
    
    // SET SYNC_HEIGHT IMMEDIATELY from LevelDB
    // This ensures API has correct height even during blk file processing
    let cf_state = db.cf_handle("chain_state").ok_or("chain_state CF not found")?;
    db.put_cf(&cf_state, b"sync_height", leveldb_height.to_le_bytes())?;
    println!("✅ Set sync_height to {} (from LevelDB canonical chain)", leveldb_height);
    
    // Store canonical chain metadata in our DB
    // This will guide the parallel block processor to know which blocks to index
    println!("\n📦 Storing canonical chain metadata...");
    
    let cf_metadata = db.cf_handle("chain_metadata").ok_or("chain_metadata CF not found")?;
    
    let mut offsets_stored = 0;
    let mut offsets_missing = 0;
    
    for (height, hash, opt_file, opt_pos) in &canonical_chain {
        let height_key = (*height as i32).to_le_bytes();
        // Store reversed hash (display format) for consistency with existing code
        let mut display_hash = hash.clone();
        display_hash.reverse();
        db.put_cf(&cf_metadata, height_key, &display_hash)?;
        
        // ALSO store the reverse mapping: 'h' + internal_hash → height
        // This allows blk file processing to look up heights efficiently
        let mut hash_key = vec![b'h'];
        hash_key.extend_from_slice(hash);  // Internal format (not reversed)
        db.put_cf(&cf_metadata, &hash_key, height_key)?;

        // If the leveldb index provided blk file number and data position, store it
        if let (Some(file_num), Some(data_pos)) = (opt_file, opt_pos) {
            let mut off_key = vec![b'o'];
            off_key.extend_from_slice(hash); // internal format
            let mut buf = Vec::with_capacity(12);
            buf.extend_from_slice(&(*file_num as u32).to_le_bytes());
            buf.extend_from_slice(&{ *data_pos }.to_le_bytes());
            db.put_cf(&cf_metadata, &off_key, &buf)?;
            offsets_stored += 1;
        } else {
            offsets_missing += 1;
        }
        
        if *height % 500_000 == 0 {
            println!("  Stored metadata for height {}", height);
        }
    }
    
    println!("✅ Canonical chain metadata stored:");
    println!("   Total blocks: {}", canonical_chain.len());
    println!("   Blocks WITH offsets: {} ({:.2}%)", 
             offsets_stored,
             (offsets_stored as f64 / canonical_chain.len() as f64) * 100.0);
    println!("   Blocks WITHOUT offsets: {} ({:.2}%)", 
             offsets_missing,
             (offsets_missing as f64 / canonical_chain.len() as f64) * 100.0);
    
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
        if path.extension().and_then(|s| s.to_str()) == Some("dat") && path.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("blk")).unwrap_or(false) {
            entries.push(path);
        }
    }
    
    println!("Found {} blk*.dat files", entries.len());
    
    // Process files in parallel (this will index all blocks, including orphans)
    // The chainwork calculation later will determine which ones are canonical
    process_files_parallel(entries, Arc::clone(&db), state.clone(), max_concurrent).await?;
    
    let final_height = (chain_len - 1) as i32;
    
    println!("\n✅ Initial sync complete! Final height: {}", final_height);
    println!("   All {} canonical blocks are now indexed", chain_len);
    println!("   The explorer will now switch to RPC mode to catch new blocks");
    
    // Mark sync complete
    set_sync_height(&db, final_height).await?;
    
    // OPTIONAL: Run Pattern A offset-based indexing for validation
    // This reads blocks directly by offset and compares with scanner results
    let validate_with_offset_indexer = config.get_bool("sync.validate_offset_indexing").unwrap_or(false);
    if validate_with_offset_indexer {
        println!("\n📍 Running Pattern A validation (offset-based indexing)...");
        println!("   This will re-index canonical blocks using file offsets");
        println!("   to validate the offset-based approach works correctly.\n");
        
        use crate::offset_indexer::index_canonical_blocks_by_offset;
        if let Err(e) = index_canonical_blocks_by_offset(blk_dir.clone(), db.clone(), state.clone()).await {
            eprintln!("⚠️  Offset-based validation failed: {}", e);
            eprintln!("   Continuing with scanner-based results");
        } else {
            println!("✅ Offset-based indexing validation complete!");
            println!("   Pattern A approach verified - ready to switch over\n");
        }
    }
    
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

/// Run post-sync enrichment: address indexing + transaction reconciliation
/// This runs after blockchain sync to ensure all explorer data is available
async fn run_post_sync_enrichment(db: &Arc<DB>) -> Result<(), Box<dyn std::error::Error>> {
    let config = get_global_config();
    let enrich_addresses = config.get_bool("sync.enrich_addresses").unwrap_or(false);
    let fast_sync = config.get_bool("sync.fast_sync").unwrap_or(false);
    let use_chainstate = config.get_bool("sync.use_chainstate_for_utxos").unwrap_or(false);
    let use_block_index = config.get_bool("sync.use_block_index_for_heights").unwrap_or(true);
    
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║         POST-SYNC DATA ENRICHMENT                  ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    
    // Check what enrichment steps have been completed
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    let height_resolution_complete = match db.get_cf(&cf_state, b"height_resolution_complete")? {
        Some(bytes) => bytes[0] == 1,
        None => false,
    };
    
    let address_index_complete = match db.get_cf(&cf_state, b"address_index_complete")? {
        Some(bytes) => bytes[0] == 1,
        None => false,
    };
    
    let tx_block_index_complete = match db.get_cf(&cf_state, b"tx_block_index_complete")? {
        Some(bytes) => bytes[0] == 1,
        None => false,
    };
    
    let repair_complete = match db.get_cf(&cf_state, b"repair_complete")? {
        Some(bytes) => bytes[0] == 1,
        None => false,
    };
    
    // PHASE 0: Resolve heights from PIVX Core block index (NEW - Strategy 1)
    // Only run this if we actually have transactions in the database!
    if use_block_index && !height_resolution_complete {
        // Check if we have any transactions to fix
        let cf_transactions = db.cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        
        let mut has_transactions = false;
        if let Some(Ok((_key, _value))) = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start).next() {
            has_transactions = true;
        }
        
        if !has_transactions {
            println!("ℹ️  Phase 0: Height resolution skipped - no transactions in database yet");
            println!("   Height resolution will run after blockchain sync completes\n");
            // Don't mark as complete - we'll run it later
        } else {
            println!("📊 Phase 0: Resolving transaction heights from PIVX Core block index...");
            println!("   This ensures heights are correct BEFORE enrichment");
            println!("   Orphaned transactions will be identified from Core's canonical chain\n");
            
            use crate::height_resolver::resolve_heights_from_block_index;
            
            match resolve_heights_from_block_index(Arc::clone(db), None).await {
                Ok((fixed, orphaned)) => {
                    if fixed > 0 {
                        println!("   ✅ Resolved {} transaction heights from canonical chain", fixed);
                    }
                    if orphaned > 0 {
                        println!("   ⚠️  Identified {} orphaned transactions (not in canonical chain)", orphaned);
                    }
                    if fixed == 0 && orphaned == 0 {
                        println!("   ✅ All transaction heights already correct");
                    }
                    println!();
                    db.put_cf(&cf_state, b"height_resolution_complete", [1u8])?;
                    
                    // Mark repair as complete too since we just did it
                    db.put_cf(&cf_state, b"repair_complete", [1u8])?;
                }
                Err(e) => {
                    eprintln!("⚠️  Height resolution failed: {}", e);
                    eprintln!("   Will fall back to repair phase during enrichment\n");
                }
            }
        }
    } else if height_resolution_complete {
        println!("✅ Phase 0: Height resolution already complete - skipping\n");
    } else {
        println!("ℹ️  Phase 0: Height resolution disabled (use_block_index_for_heights=false)");
        println!("   Will use repair phase instead\n");
    }
    
    // 1. Address enrichment (if fast_sync was used and not already done)
    if fast_sync && enrich_addresses && !address_index_complete {
        if use_chainstate {
            println!("📍 Phase 1: Building address index from PIVX Core chainstate...");
            println!("   Using chainstate as source of truth for current UTXOs");
            println!("   This ensures balances match PIVX Core exactly\n");
            
            use crate::enrich_from_chainstate::enrich_from_chainstate;
            if let Err(e) = enrich_from_chainstate(Arc::clone(db)).await {
                eprintln!("⚠️  Chainstate enrichment failed: {}", e);
                eprintln!("   Falling back to transaction-based enrichment...\n");
                
                use crate::enrich_addresses::enrich_all_addresses;
                if let Err(e) = enrich_all_addresses(Arc::clone(db)).await {
                    eprintln!("⚠️  Address enrichment also failed: {}", e);
                    eprintln!("   Continuing without address data.");
                    eprintln!("   You can retry later by setting enrich_addresses=true\n");
                } else {
                    println!("✅ Address index built successfully (from transactions)!\n");
                    db.put_cf(&cf_state, b"address_index_complete", [1u8])?;
                }
            } else {
                println!("✅ Address index built successfully (from chainstate)!\n");
                db.put_cf(&cf_state, b"address_index_complete", [1u8])?;
            }
        } else {
            println!("📍 Phase 1: Building address index from transactions...");
            println!("   This allows searching by address and viewing address balances\n");
            
            use crate::enrich_addresses::enrich_all_addresses;
            if let Err(e) = enrich_all_addresses(Arc::clone(db)).await {
                eprintln!("⚠️  Address enrichment failed: {}", e);
                eprintln!("   Continuing without address data.");
                eprintln!("   You can retry later by setting enrich_addresses=true\n");
            } else {
                println!("✅ Address index built successfully!\n");
                db.put_cf(&cf_state, b"address_index_complete", [1u8])?;
            }
        }
    } else if address_index_complete {
        println!("✅ Phase 1: Address index already complete - skipping\n");
    } else if fast_sync {
        println!("ℹ️  Address enrichment disabled (enrich_addresses=false in config)");
        println!("   Set enrich_addresses=true to enable address search\n");
    }
    
    // 2. Transaction block index (B prefix entries for faster block tx lookups)
    if !tx_block_index_complete {
        println!("📊 Phase 2: Building transaction block index...");
        println!("   This speeds up block transaction queries\n");
        
        if let Err(e) = rebuild_transaction_block_index(db).await {
            eprintln!("⚠️  Transaction block index failed: {}", e);
            eprintln!("   Continuing without block tx index optimization\n");
        } else {
            println!("✅ Transaction block index built successfully!\n");
            db.put_cf(&cf_state, b"tx_block_index_complete", [1u8])?;
        }
    } else {
        println!("✅ Phase 2: Transaction block index already complete - skipping\n");
    }
    
    // 3. Fix transactions with height=0 (database repair) - skip if height resolution already ran
    if !repair_complete && !height_resolution_complete {
        println!("🔧 Phase 3: Repairing transactions with incorrect heights...");
        println!("   This fixes transactions stored with height=0 during initial sync\n");
        
    match repair::fix_zero_height_transactions(db).await {
            Ok((fixed, orphaned)) => {
                if fixed > 0 {
                    println!("   ✅ Repaired {} transactions with correct heights", fixed);
                }
                if orphaned > 0 {
                    println!("   ⚠️  Marked {} orphaned transactions (height=-1, excluded from balances)", orphaned);
                }
                if fixed == 0 && orphaned == 0 {
                    println!("   ✅ No transactions needed repair");
                }
                println!();
                db.put_cf(&cf_state, b"repair_complete", [1u8])?;
            }
            Err(e) => {
                eprintln!("⚠️  Transaction repair failed: {}", e);
                eprintln!("   Continuing - some transactions may show incorrect data\n");
            }
        }
    } else if height_resolution_complete {
        println!("✅ Phase 3: Transaction repair not needed (heights resolved from block index)\n");
    } else {
        println!("✅ Phase 3: Transaction repair already complete - skipping\n");
    }
    
    println!("╔════════════════════════════════════════════════════╗");
    println!("║     🎉 ENRICHMENT COMPLETE - READY FOR USE 🎉      ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    
    Ok(())
}

/// Rebuild transaction block index (B prefix entries)
/// Creates entries like 'B' + height + tx_index → txid for faster block queries
async fn rebuild_transaction_block_index(db: &Arc<DB>) -> Result<(), Box<dyn std::error::Error>> {
    use rocksdb::WriteBatch;
    use std::collections::HashMap;
    
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    // Map of height -> list of txids
    let mut block_txs: HashMap<i32, Vec<Vec<u8>>> = HashMap::new();
    
    println!("   📖 Reading all transactions...");
    let mut tx_count = 0;
    let mut indexed_count = 0;
    let iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    
    for item in iter {
        match item {
            Ok((key, value)) => {
                // Only process 't' prefix entries (transaction data)
                if !key.is_empty() && key[0] == b't' {
                    tx_count += 1;
                    
                    // Extract txid from key (skip 't' prefix)
                    let txid = &key[1..];
                    
                    // Extract height from value
                    // Format: version (4 bytes) + height (4 bytes) + tx_bytes
                    if value.len() >= 8 {
                        let height = i32::from_le_bytes([value[4], value[5], value[6], value[7]]);
                        
                        // Skip if height is invalid
                        if height > 0 && height < 10_000_000 {
                            block_txs.entry(height).or_default().push(txid.to_vec());
                            indexed_count += 1;
                        }
                    }
                    
                    if tx_count % 100_000 == 0 {
                        println!("      Processed {} transactions...", tx_count);
                    }
                }
            }
            Err(e) => {
                eprintln!("      Error reading transaction: {}", e);
            }
        }
    }
    
    println!("   ✅ Read {} transactions ({} with valid heights)", tx_count, indexed_count);
    println!("   📝 Writing block transaction index...");
    
    let mut batch = WriteBatch::default();
    let mut batch_count = 0;
    const BATCH_SIZE: usize = 10000;
    
    for (height, mut txids) in block_txs {
        // Sort txids to maintain consistent order
        txids.sort();
        
        for (tx_index, txid) in txids.iter().enumerate() {
            // Create key: 'B' + height (4 bytes) + tx_index (4 bytes)
            let mut key = vec![b'B'];
            key.extend_from_slice(&height.to_le_bytes());
            key.extend_from_slice(&(tx_index as u32).to_le_bytes());
            
            batch.put_cf(&cf_transactions, &key, txid);
            batch_count += 1;
            
            // Commit batch periodically
            if batch_count % BATCH_SIZE == 0 {
                db.write(batch)?;
                batch = WriteBatch::default();
                
                if batch_count % 50000 == 0 {
                    println!("      Written {} block-tx mappings...", batch_count);
                }
            }
        }
    }
    
    // Final batch
    if !batch.is_empty() {
        db.write(batch)?;
    }
    
    println!("   ✅ Wrote {} block-tx index entries", batch_count);
    
    Ok(())
}

/// Monitor for new blocks - SMART HYBRID approach
/// Skips blk file processing if we're close to chain tip (within 100 blocks)
async fn run_live_sync(
    blk_dir: PathBuf,
    db: Arc<DB>,
    state: AppState,
    current_height: i32,
    broadcaster: Option<Arc<EventBroadcaster>>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting SMART sync mode...");
    println!("Current DB height: {}", current_height);
    
    // Check network height to determine if we need blk file catchup
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    let network_height = match db.get_cf(&cf_state, b"network_height")? {
        Some(bytes) => i32::from_le_bytes(bytes.as_slice().try_into().unwrap_or([0, 0, 0, 0])),
        None => {
            println!("⚠️  Network height not available, will fetch from RPC");
            current_height // Assume we're current if unknown
        }
    };
    
    let blocks_behind = network_height - current_height;
    println!("📊 Network height: {} | Blocks behind: {}", network_height, blocks_behind);
    
    // Only process blk files if we're significantly behind (>1000 blocks)
    // This makes startup instant when we're nearly synced
    if blocks_behind > 1000 {
        println!("\n⚡ {} blocks behind - will catch up via blk files first (faster)", blocks_behind);
        println!("Phase 1: Syncing from blk*.dat files...");
        
        let config = get_global_config();
        let max_concurrent = config.get_int("sync.parallel_files").unwrap_or(8) as usize;
        
        // Read all .dat files and determine which ones to process
        let mut dir_entries = fs::read_dir(&blk_dir).await?;
        let mut all_entries = Vec::new();
        
        while let Ok(Some(entry)) = dir_entries.next_entry().await {
            let path = entry.path();
            // Only include blk*.dat files (not rev*.dat or other .dat files)
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if filename.starts_with("blk") && filename.ends_with(".dat") {
                    // Get file metadata to sort by modification time
                    if let Ok(metadata) = entry.metadata().await {
                        if let Ok(modified) = metadata.modified() {
                            all_entries.push((path, modified));
                        }
                    }
                }
            }
        }
        
        // Sort by modification time (newest first) instead of filename
        all_entries.sort_by_key(|(_, time)| std::cmp::Reverse(*time));
        let all_paths: Vec<_> = all_entries.into_iter().map(|(path, _)| path).collect();
        
        let entries = if blocks_behind < 1000 {
            // Very close - skip blk files, use RPC only for instant startup
            println!("🚀 Only {} blocks behind - skipping blk files (will catch up via RPC)", blocks_behind);
            Vec::new()
        } else if blocks_behind < 5000 {
            // Close - process last 10 most recent files
            let files_to_check = 10;
            let recent_files: Vec<_> = all_paths.into_iter().take(files_to_check).collect();
            println!("📁 {} blocks behind - processing {} most recent blk files", blocks_behind, recent_files.len());
            recent_files
        } else if blocks_behind < 20000 {
            // Medium distance - process last 30 files
            let files_to_check = 30;
            let recent_files: Vec<_> = all_paths.into_iter().take(files_to_check).collect();
            println!("📁 {} blocks behind - processing {} recent blk files", blocks_behind, recent_files.len());
            recent_files
        } else {
            // Far behind - process ALL files to ensure we catch everything
            println!("📁 {} blocks behind - processing ALL {} blk files (faster than RPC)", blocks_behind, all_paths.len());
            all_paths
        };
        
        if !entries.is_empty() {
            println!("Processing {} blk*.dat files in parallel...", entries.len());
            process_files_parallel(entries, Arc::clone(&db), state.clone(), max_concurrent).await?;
            
            println!("✅ Finished processing blk*.dat files!");
        }
    } else {
        println!("\n🚀 Only {} blocks behind - skipping blk file scan (INSTANT startup!)", blocks_behind);
        println!("   Will catch up via RPC...");
    }
    
    // Switch to RPC for new blocks (or catchup if we skipped blk files)
    println!("Phase 2: Monitoring for new blocks via RPC...");
    run_block_monitor(db, 5, broadcaster).await?;
    
    Ok(())
}

/// Main sync service - automatically chooses mode
pub async fn run_sync_service(
    blk_dir: PathBuf,
    db: Arc<DB>,
    broadcaster: Option<Arc<EventBroadcaster>>,
) -> Result<(), Box<dyn std::error::Error>> {
    
    // Create a dummy cache for sync - sync operations don't use the HTTP cache
    let cache = Arc::new(CacheManager::new());
    
    let state = AppState {
        db: Arc::clone(&db),
        cache,
    };
    
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║          PIVX BLOCKCHAIN EXPLORER SYNC             ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    
    // Check if resync is requested
    let config = get_global_config();
    let resync = config.get_bool("sync.resync").unwrap_or(false);
    
    if resync {
        println!("\n🔄 RESYNC MODE ENABLED");
        println!("   Clearing all databases and rebuilding from scratch...\n");
        
        // Clear all column families
        let cf_names = vec![
            "blocks", "transactions", "chain_metadata", 
            "addr_index", "utxo", "pubkey", "chain_state"
        ];
        
        for cf_name in cf_names {
            if let Some(cf) = db.cf_handle(cf_name) {
                println!("   Clearing column family: {}", cf_name);
                
                // Get all keys in this CF
                let iter = db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
                let mut keys_to_delete = Vec::new();
                
                for item in iter {
                    if let Ok((key, _)) = item {
                        keys_to_delete.push(key.to_vec());
                    }
                }
                
                // Delete all keys
                println!("     Deleting {} entries...", keys_to_delete.len());
                for key in keys_to_delete {
                    db.delete_cf(&cf, &key)?;
                }
            }
        }
        
        println!("✅ All databases cleared!\n");
    }
    
    // Fetch and store network height from RPC early
    println!("🔍 Checking network status...");
    update_network_height(&db).await;
    
    // Check sync status
    println!("🔍 Checking database sync status...");
    match get_sync_status(&db).await? {
        SyncStatus::NeedInitialSync => {
            println!("\n🆕 NO EXISTING INDEX FOUND");
            println!("   Running initial sync from scratch...\n");
            
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
            
            // Run post-sync enrichment (addresses + transaction indexing)
            run_post_sync_enrichment(&db).await?;
            
            // Then switch to live mode
            println!("\n🔄 Switching to live sync mode...");
            run_live_sync(blk_dir, db, state, final_height, broadcaster).await?;
        }
        SyncStatus::Synced { height } => {
            println!("\n✅ EXISTING INDEX FOUND");
            println!("   Database height: {}\n", height);
            
            // Get network height for comparison
            let cf_state = db.cf_handle("chain_state")
                .ok_or("chain_state CF not found")?;
            
            let network_height = match db.get_cf(&cf_state, b"network_height")? {
                Some(bytes) => i32::from_le_bytes(bytes.as_slice().try_into().unwrap_or([0, 0, 0, 0])),
                None => height, // Unknown, assume current
            };
            
            let blocks_behind = network_height - height;
            
            if blocks_behind <= 5 {
                println!("🎉 ALREADY SYNCED! Only {} blocks behind", blocks_behind);
                println!("   Startup will be INSTANT - going straight to RPC monitoring\n");
            } else if blocks_behind <= 100 {
                println!("⚡ NEARLY SYNCED! {} blocks behind", blocks_behind);
                println!("   Startup will be FAST - skipping blk file scan\n");
            } else {
                println!("📥 CATCHING UP: {} blocks behind", blocks_behind);
                println!("   Will process blk files for faster catchup\n");
            }
            
            // Only run enrichment if any phase is incomplete
            // The monitor handles incremental updates for new blocks
            run_post_sync_enrichment(&db).await?;
            
            // Go straight to live mode (it will decide whether to scan blk files)
            run_live_sync(blk_dir, db, state, height, broadcaster).await?;
        }
    }
    
    Ok(())
}

