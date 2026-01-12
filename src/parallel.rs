use std::path::PathBuf;
use std::sync::Arc;
use rocksdb::DB;
use tokio::sync::Semaphore;
use crate::types::AppState;
use crate::blocks::process_blk_file;
use crate::db_utils::save_file_as_incomplete;
use crate::chain_state::set_sync_height;
use crate::chainwork::calculate_all_chainwork;
use crate::sync::validate_canonical_metadata_complete;
use hex;
use std::collections::HashMap;
use crate::config::get_global_config;
use reqwest::Client;
use serde_json::Value;

/// Update sync_height by finding the highest block in chain_metadata
/// This allows incremental progress updates as files are processed
/// Optimized: reads in reverse to find max height quickly
async fn update_sync_height_from_metadata(db: &Arc<DB>) -> Result<(), Box<dyn std::error::Error>> {
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    // Iterate FORWARD through ALL entries to find the max height
    // We need to check ALL 4-byte keys since they're mixed with 33-byte 'h' + hash keys
    let mut max_height: i32 = -1;
    
    let iter = db.iterator_cf(&cf_metadata, rocksdb::IteratorMode::Start);
    
    for item in iter {
        if let Ok((key, _value)) = item {
            // Only check 4-byte keys (height → hash mappings)
            // Skip 33-byte keys ('h' + hash → height mappings)
            if key.len() == 4 {
                let height = i32::from_le_bytes([key[0], key[1], key[2], key[3]]);
                if height > max_height {
                    max_height = height;
                }
            }
        }
    }
    
    // Only update if we found a valid height and it's higher than current
    if max_height >= 0 {
        set_sync_height(db, max_height)?;
        println!("  Updated sync_height to: {}", max_height);
    }
    
    Ok(())
}


/// Find the maximum chainwork among all descendants of a block
/// This is used to pick the "best" fork - the one leading to the most work
/// Uses iterative DFS with memoization to avoid stack overflow
#[allow(dead_code)]
fn find_max_descendant_chainwork(
    start_hash: &[u8],
    children_map: &HashMap<Vec<u8>, Vec<(Vec<u8>, Vec<u8>)>>,
    chainwork_map: &HashMap<Vec<u8>, [u8; 32]>,
) -> [u8; 32] {
    use std::collections::VecDeque;
    
    let zero_work = [0u8; 32];
    let mut max_work = chainwork_map.get(start_hash).copied().unwrap_or(zero_work);
    let mut queue: VecDeque<Vec<u8>> = VecDeque::new();
    queue.push_back(start_hash.to_vec());
    
    let mut visited = std::collections::HashSet::new();
    
    while let Some(current_hash) = queue.pop_front() {
        if !visited.insert(current_hash.clone()) {
            continue; // Already processed
        }
        
        let current_work = chainwork_map.get(&current_hash).copied().unwrap_or(zero_work);
        if current_work > max_work {
            max_work = current_work;
        }
        
        // Add all children to queue
        if let Some(children) = children_map.get(&current_hash) {
            for (child_hash, _) in children {
                queue.push_back(child_hash.clone());
            }
        }
    }
    
    max_work
}

/// Process multiple block files in parallel with controlled concurrency
/// 
/// Architecture:
/// - Uses tokio tasks with semaphore to limit concurrent processing
/// - Each file is processed on the tokio runtime
/// - Database writes are batched within each file processor
pub async fn process_files_parallel(
    entries: Vec<PathBuf>,
    db_arc: Arc<DB>,
    state: AppState,
    max_concurrent: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    
    // [F3] CRITICAL: Validate canonical metadata completeness BEFORE parallel processing
    // This prevents the height=0 bug where all transactions get stored with height=0
    // instead of their correct heights due to missing height→hash mappings.
    println!("\n🔍 [F3] Validating canonical chain metadata before parallel processing...");
    
    let cf_metadata = db_arc.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    // Count height→hash mappings (4-byte keys) to determine expected chain length
    let mut height_count = 0;
    let iter = db_arc.iterator_cf(&cf_metadata, rocksdb::IteratorMode::Start);
    for item in iter {
        if let Ok((key, _)) = item {
            if key.len() == 4 {
                height_count += 1;
            }
        }
    }
    
    if height_count > 0 {
        // Metadata exists - validate it's complete
        let metadata_complete = validate_canonical_metadata_complete(&db_arc, height_count).await?;
        
        if !metadata_complete {
            return Err(format!(
                "FATAL [F3]: Canonical metadata incomplete ({} height mappings found).\n\
                 This would cause ALL transactions to get height=0 instead of correct heights.\n\
                 Check leveldb import logs for errors.\n\
                 Recommendation: Delete database and resync from scratch.",
                height_count
            ).into());
        }
        
        println!("✅ [F3] Canonical metadata validated: {} heights complete and contiguous", height_count);
    } else {
        println!("⚠️  [F3] No canonical metadata found - processing will assign heights dynamically");
        println!("    This is normal for first sync without leveldb import");
    }
    
    println!("Starting parallel file processing with {} workers", max_concurrent);
    
    // Filter for .dat files
    let mut blk_files: Vec<_> = entries
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("blk") && n.ends_with(".dat"))
                .unwrap_or(false)
        })
        .collect();
    
    // CRITICAL: Sort blk files in REVERSE order (newest first)
    // This ensures we process blk00141.dat, blk00140.dat, etc.
    // So we find the highest blocks first and work backwards to genesis
    blk_files.sort_by(|a, b| b.cmp(a));
    
    let total_files = blk_files.len();
    println!("Found {} block files to process (processing in REVERSE order - newest first)", total_files);
    println!("First file: {:?}", blk_files.first().map(|p| p.file_name()));
    println!("Last file: {:?}", blk_files.last().map(|p| p.file_name()));
    
    // Semaphore to limit concurrent file processing
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    
    // Progress tracking
    let completed = Arc::new(tokio::sync::Mutex::new(0_usize));
    
    // Process files with controlled concurrency
    let tasks: Vec<_> = blk_files
        .into_iter()
        .map(|file_path| {
            let sem = semaphore.clone();
            let db = db_arc.clone();
            let st = state.clone();
            let completed_clone = completed.clone();
            
            async move {
                // Acquire permit - if this fails, semaphore is closed (shutdown)
                let _permit = match sem.acquire().await {
                    Ok(permit) => permit,
                    Err(e) => {
                        eprintln!("Failed to acquire semaphore permit: {}", e);
                        return;
                    }
                };
                
                // Process file (this is async but not Send, so we run it directly)
                if let Err(e) = process_blk_file(st, file_path.clone(), db.clone()).await {
                    eprintln!("Failed to process {}: {}", file_path.display(), e);
                    let _ = save_file_as_incomplete(&db, &file_path).await;
                }
                
                // Update progress
                let mut count = completed_clone.lock().await;
                *count += 1;
                let current = *count;
                drop(count);
                
                let progress = (current as f64 / total_files as f64) * 100.0;
                println!("\n📊 File Progress: {}/{} ({:.1}%) complete", current, total_files, progress);
                
                // Update sync_height incrementally to show progress
                if let Err(e) = update_sync_height_from_metadata(&db).await {
                    eprintln!("Warning: Failed to update sync_height: {}", e);
                }
            }
        })
        .collect();
    
    // Execute all tasks concurrently
    futures::future::join_all(tasks).await;
    
    println!("\n✅ All blk*.dat files processed!");
    
    // CRITICAL: Update sync_height to reflect all blocks processed
    // This ensures the next phase (RPC monitoring) knows our true current height
    println!("\n🔄 Updating sync height from all processed blocks...");
    update_sync_height_from_metadata(&db_arc).await?;
    
    // Check if canonical chain metadata already exists (from leveldb phase)
    let cf_metadata = db_arc.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    // Check if canonical metadata appears complete: count how many height->hash 4-byte keys exist.
    // If there's only the genesis mapping (or none) we consider the metadata incomplete and
    // fall back to resolving block heights from the processed blk files. This avoids the
    // situation where a partial leveldb import left only height 0 mapping and the explorer
    // never assigns real heights (sync_height remains 0).
    let mut height_key_count: usize = 0;
    let iter = db_arc.iterator_cf(&cf_metadata, rocksdb::IteratorMode::Start);
    for item in iter {
        if let Ok((key, _)) = item {
            if key.len() == 4 {
                height_key_count += 1;
            }
        }
    }

    let has_canonical_metadata = height_key_count > 1; // more than just genesis

    if has_canonical_metadata {
        println!("\n✅ Canonical chain metadata already exists (from leveldb)");
        println!("   Using pre-built canonical chain ({} height mappings found)", height_key_count);
        println!("   Resolving any NEW blocks from blk files...");
        // CRITICAL: Always resolve to pick up new blocks beyond the leveldb import
        resolve_block_heights(&db_arc).await?;
        println!("✅ Chain resolution complete (extended chain with new blocks)!");
    } else {
        // FALLBACK: Only resolve if no canonical metadata exists
        println!("\n🔗 Phase 2: Resolving block heights (building chain)...");
        println!("   (No leveldb metadata found, building chain from blk files)");
        resolve_block_heights(&db_arc).await?;
        println!("✅ Chain building complete!");
    }
    
    // CRITICAL: Update sync_height AGAIN after chain resolution to pick up newly resolved heights
    println!("\n🔄 Updating sync height after chain resolution...");
    update_sync_height_from_metadata(&db_arc).await?;
    
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║        📦 BLK FILE PROCESSING COMPLETE 📦          ║");
    println!("╚════════════════════════════════════════════════════╝");
    
    Ok(())
}

/// Resolve block heights by following the blockchain from genesis
/// Optimized O(n) version using hash map for instant lookups
/// Now with RPC validation at checkpoints to ensure we follow the canonical chain
async fn resolve_block_heights(db: &Arc<DB>) -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::HashMap;
    
    let cf_blocks = db.cf_handle("blocks")
        .ok_or("blocks CF not found")?;
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    println!("  Step 1: Building hash map (loading all blocks into memory)...");
    
    // Build hash map: prev_hash -> Vec<(block_hash, header_bytes)>
    // Multiple children = fork, we'll need to pick the right one
    let mut children_map: HashMap<Vec<u8>, Vec<(Vec<u8>, Vec<u8>)>> = HashMap::new();
    let iter = db.iterator_cf(&cf_blocks, rocksdb::IteratorMode::Start);
    let mut total_blocks = 0;
    
    for item in iter {
        if let Ok((hash, header_bytes)) = item {
            if header_bytes.len() >= 68 {
                let prev_hash = header_bytes[4..36].to_vec();
                let hash_vec = hash.to_vec();
                let header_vec = header_bytes.to_vec();
                
                children_map.entry(prev_hash)
                    .or_default()
                    .push((hash_vec, header_vec));
                
                total_blocks += 1;
                if total_blocks % 100000 == 0 {
                    println!("    Loaded {} blocks...", total_blocks);
                }
            }
        }
    }
    
    println!("  Loaded {} blocks into memory", total_blocks);
    
    // DEBUG: Check if block 2,678,400 is in children_map
    let block_2678400_hash = match hex::decode("bde2ea24bba50fb80a9c98b67e76a4407d0251aa61ac13bbcfa7feccab57bce7") {
        Ok(hash) => hash,
        Err(e) => {
            eprintln!("  DEBUG: Failed to decode debug hash: {}", e);
            vec![] // Empty vec won't match anything in map
        }
    };
    if !block_2678400_hash.is_empty() {
        if let Some(children) = children_map.get(&block_2678400_hash) {
            println!("  DEBUG: Block 2,678,400 has {} children in map (BEFORE chainwork calc)", children.len());
            for (i, (child_hash, _)) in children.iter().enumerate() {
                let display_hash: Vec<u8> = child_hash.iter().rev().cloned().collect();
                println!("    Child {}: {}", i + 1, hex::encode(&display_hash));
            }
        } else {
            println!("  DEBUG: Block 2,678,400 NOT FOUND in children_map!");
        }
    }
    
    // Store children_map size
    let children_map_size_before = children_map.len();
    let total_children_before: usize = children_map.values().map(|v| v.len()).sum();
    println!("  DEBUG: children_map has {} parent hashes, {} total children (BEFORE)", 
             children_map_size_before, total_children_before);
    
    // Build blocks_map for chainwork calculation (hash -> header_bytes)
    println!("  Step 2: Calculating accumulated chainwork (Bitcoin consensus)...");
    let mut blocks_map: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    let iter2 = db.iterator_cf(&cf_blocks, rocksdb::IteratorMode::Start);
    for item in iter2 {
        if let Ok((hash, header_bytes)) = item {
            blocks_map.insert(hash.to_vec(), header_bytes.to_vec());
        }
    }
    
    let chainwork_map = calculate_all_chainwork(db, &blocks_map)?;
    println!("  ✅ Chainwork calculated for {} blocks", chainwork_map.len());
    
    // DEBUG: Check children_map AFTER chainwork calculation
    println!("  DEBUG: Checking children_map AFTER chainwork calculation...");
    let children_map_size_after = children_map.len();
    let total_children_after: usize = children_map.values().map(|v| v.len()).sum();
    println!("  DEBUG: children_map has {} parent hashes, {} total children (AFTER)", 
             children_map_size_after, total_children_after);
    
    if let Some(children) = children_map.get(&block_2678400_hash) {
        println!("  DEBUG: Block 2,678,400 STILL has {} children in map (AFTER chainwork calc)", children.len());
    } else {
        println!("  DEBUG: Block 2,678,400 NOW MISSING from children_map!");
    }
    
    println!("  Step 3: Finding the best chain tip by checking highest blocks first...");
    
    // Strategy: Start from the HIGHEST blocks in the database and work backwards
    // The daemon stores newest blocks in the last blk files (blk00141.dat)
    // So we should find tips there and work backwards to genesis
    
    println!("  Finding potential tips and verifying with RPC...");
    
    // STEP 3A: Find blocks with no children (potential tips)
    let mut all_blocks: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    let mut has_children: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    
    for (parent_hash, children) in &children_map {
        all_blocks.insert(parent_hash.clone());
        for (child_hash, _) in children {
            all_blocks.insert(child_hash.clone());
            has_children.insert(parent_hash.clone());
        }
    }
    
    let potential_tips: Vec<Vec<u8>> = all_blocks
        .difference(&has_children)
        .cloned()
        .collect();
    
    println!("  Found {} potential chain tips", potential_tips.len());

    // [M2] OPTIMIZATION: Use chainwork-only selection instead of RPC validating every tip
    // Old approach: RPC validate 1000+ tips (100+ seconds)
    // New approach: Use chainwork to find best tip, only RPC validate the chosen one (1 call)
    
    println!("  Step 3B: Selecting best tip by chainwork (avoiding O(n²) RPC calls)...");
    
    // Find tip with highest chainwork
    let mut best_tip: Option<(Vec<u8>, [u8; 32])> = None;
    for tip in &potential_tips {
        if let Some(work) = chainwork_map.get(tip) {
            match &best_tip {
                None => best_tip = Some((tip.clone(), *work)),
                Some((_, best_work)) => {
                    if work > best_work {
                        best_tip = Some((tip.clone(), *work));
                    }
                }
            }
        }
    }
    
    let (highest_tip, best_chainwork) = match best_tip {
        Some((tip, work)) => (tip, work),
        None => return Err("No tips found with chainwork data".into()),
    };
    
    let tip_display: Vec<u8> = highest_tip.iter().rev().cloned().collect();
    let tip_hex = hex::encode(&tip_display);
    
    println!("  ✅ Selected best tip by chainwork: {}", &tip_hex[..16]);
    println!("     Chainwork: {}", hex::encode(best_chainwork));
    
    // Optional: RPC-validate the chosen tip matches node's view
    let config = get_global_config();
    let rpc_validation = match (
        config.get_string("rpc.host"),
        config.get_string("rpc.user"),
        config.get_string("rpc.pass"),
    ) {
        (Ok(host), Ok(user), Ok(pass)) => {
            let url = if host.starts_with("http://") || host.starts_with("https://") {
                host
            } else {
                format!("http://{}", host)
            };
            
            println!("  🔍 Validating chosen tip with RPC (single call)...");
            
            let body = serde_json::json!({
                "jsonrpc": "1.0",
                "id": "rbx",
                "method": "getblock",
                "params": [tip_hex, 1]
            });
            
            // Use spawn_blocking for RPC call
            let rpc_result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                tokio::task::spawn_blocking(move || {
                    reqwest::blocking::Client::new()
                        .post(&url)
                        .basic_auth(&user, Some(&pass))
                        .json(&body)
                        .send()
                })
            ).await;
            
            match rpc_result {
                Ok(Ok(Ok(resp))) if resp.status().is_success() => {
                    if let Ok(text) = resp.text() {
                        if let Ok(json_val) = serde_json::from_str::<Value>(&text) {
                            if let Some(height) = json_val.get("result").and_then(|r| r.get("height")).and_then(|h| h.as_i64()) {
                                println!("  ✅ RPC confirms tip at height {}", height);
                                Some(height as i32)
                            } else {
                                println!("  ⚠️  RPC returned block but no height field");
                                None
                            }
                        } else {
                            println!("  ⚠️  RPC returned invalid JSON");
                            None
                        }
                    } else {
                        println!("  ⚠️  RPC returned non-text response");
                        None
                    }
                }
                Ok(Ok(Ok(resp))) => {
                    println!("  ⚠️  RPC returned error status: {}", resp.status());
                    None
                }
                Ok(Ok(Err(e))) => {
                    println!("  ⚠️  RPC error: {}", e);
                    None
                }
                Ok(Err(e)) => {
                    println!("  ⚠️  RPC task panic: {}", e);
                    None
                }
                Err(_) => {
                    println!("  ⚠️  RPC timeout (>10s)");
                    None
                }
            }
        }
        _ => {
            println!("  ℹ️  RPC not configured - trusting chainwork selection");
            None
        }
    };
    
    let highest_height_opt = rpc_validation;

    // [M2] Old O(n²) loop removed - we now select best tip via chainwork only
    // This eliminates 1000+ RPC calls (saving 100+ seconds on typical sync)
    
    // Display result
    if let Some(h) = highest_height_opt {
        println!("\n  📍 Selected canonical chain tip:");
        println!("     Height: {}", h);
        println!("     Hash: {}", tip_hex);
    } else {
        println!("\n  📍 Selected canonical chain tip (chainwork-based, height unknown):");
        println!("     Hash: {}", tip_hex);
    }
    
    // STEP 3C: Walk backwards from the HIGHEST tip to genesis
    let mut highest_height: i32 = 0;
    let mut have_highest_height = false;
    if let Some(h) = highest_height_opt {
        highest_height = h;
        have_highest_height = true;
        println!("\n  Walking backwards from highest tip (height {}) to genesis...", highest_height);
    } else {
        println!("\n  Walking backwards from highest tip (height unknown via RPC) to genesis...");
    }
    
    let mut chain_path: Vec<Vec<u8>> = Vec::new();
    let mut current_hash = highest_tip.clone();
    let genesis_parent = vec![0u8; 32];
    let mut steps = 0;
    
    loop {
        chain_path.push(current_hash.clone());
        steps += 1;
        
        if steps % 100000 == 0 {
            println!("    Traced back {} blocks...", steps);
        }
        
        // Get the header to find prev_hash
        if let Some(header) = blocks_map.get(&current_hash) {
            if header.len() >= 36 {
                let prev_hash = header[4..36].to_vec();
                
                // Check if we reached genesis
                if prev_hash == genesis_parent {
                    println!("  ✅ Reached genesis block!");
                    chain_path.push(prev_hash);
                    break;
                }
                
                // Move to parent block
                current_hash = prev_hash;
            } else {
                println!("  ⚠️  Block header too short at step {}", steps);
                break;
            }
        } else {
            println!("  ⚠️  Block not found in blocks_map at step {}", steps);
            let display: Vec<u8> = current_hash.iter().rev().cloned().collect();
            println!("     Missing hash: {}", hex::encode(&display));
            println!("     Chain breaks at approximately {} blocks from tip", steps);
            println!("\n  🔍 DEBUG: Checking blocks_map for this hash...");
            println!("     blocks_map size: {} blocks", blocks_map.len());
            println!("     Searching for internal format: {:02x?}", &current_hash[..8]);
            
            // Check if this hash exists in the map with different format
            let mut found_similar = 0;
            for (hash_key, _) in blocks_map.iter().take(5) {
                println!("     Sample key in map: {:02x?}", &hash_key[..8]);
                found_similar += 1;
                if found_similar >= 3 {
                    break;
                }
            }
            break;
        }
    }
    
    let reached_genesis = chain_path.last().map(|h| h == &genesis_parent).unwrap_or(false);
    
    println!("  Chain path length: {} blocks", chain_path.len());
    
    if !reached_genesis {
        println!("  ⚠️  Chain did NOT reach genesis on first pass - checking if gap continues to genesis...");
        
        // STEP 3C2: The chain broke - now walk backwards from the MISSING block to see if it reaches genesis
        let missing_hash_display: Vec<u8> = current_hash.iter().rev().cloned().collect();
        println!("  Missing block: {}", hex::encode(&missing_hash_display));
        
        // [M2] Note: Gap filling via RPC removed as part of optimization
        // The old code attempted RPC lookup here, but this is rarely needed
        // If gaps occur, recommend resync from scratch
        println!("  ⚠️  Gap detected - recommend full resync");
        println!("      (RPC gap filling removed as part of M2 optimization)");
        
        return Err(format!("Chain gap detected at block {}", hex::encode(&missing_hash_display)).into());
    } else {
        println!("  ✅ Chain successfully reached genesis!");
    }
    
    // If we couldn't reach genesis and don't have an RPC-supplied tip height,
    // we cannot reliably assign heights — abort with a clear error so the
    // operator can supply RPC access or rebuild metadata with leveldb tools.
    if !reached_genesis && !have_highest_height {
        return Err("Chain did not reach genesis and tip height is unknown (RPC required to continue)".into());
    }

    // STEP 3D: Reverse and assign heights
    println!("  Assigning heights to canonical chain...");
    chain_path.reverse();
    
    let start_idx = if reached_genesis && chain_path[0] == genesis_parent { 1 } else { 0 };
    
    for (idx, block_hash) in chain_path[start_idx..].iter().enumerate() {
        let height = if reached_genesis {
            idx as i32
        } else {
            // If we didn't reach genesis, calculate height from the known tip height
            // highest_height is guaranteed to be present here (check above)
            highest_height - (chain_path.len() - start_idx - 1 - idx) as i32
        };
        
        // Store hash -> height mapping
        let mut h_key = vec![b'h'];
        h_key.extend_from_slice(block_hash);
        db.put_cf(&cf_metadata, &h_key, height.to_le_bytes())?;
        
        // Store height -> hash mapping (in DISPLAY format)
        let display_hash: Vec<u8> = block_hash.iter().rev().cloned().collect();
        db.put_cf(&cf_metadata, height.to_le_bytes(), &display_hash)?;
        
        if height % 100000 == 0 || height < 5 || (have_highest_height && height >= highest_height - 5) {
            println!("    Height {}: {}", height, hex::encode(&display_hash));
        }
    }
    
    let chain_height = if reached_genesis {
        (chain_path.len() - start_idx - 1) as i32
    } else {
        highest_height
    };

    println!("  ✅ Canonical chain established: {} blocks (height {} to {})", 
             chain_path.len() - start_idx,
             if reached_genesis { 0 } else { highest_height - (chain_path.len() - start_idx - 1) as i32 },
             chain_height);
    
    // Calculate statistics
    let orphaned_count = total_blocks - (chain_path.len() - start_idx);
    println!("\n📊 Chain Statistics:");
    println!("  Total blocks loaded: {}", total_blocks);
    println!("  Canonical chain length: {}", chain_path.len() - start_idx);
    println!("  Chain tip height: {}", chain_height);
    println!("  Orphaned blocks: {}", orphaned_count);
    
    Ok(())
}
