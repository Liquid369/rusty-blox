use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use rocksdb::DB;
use tokio::sync::Semaphore;
use crate::types::AppState;
use crate::blocks::process_blk_file;
use crate::db_utils::{save_file_as_incomplete, bulk_write_options};
use crate::chain_state::{set_sync_height, get_sync_height};
use crate::chainwork::calculate_all_chainwork;
use crate::sync::validate_canonical_metadata_complete;
use hex;
use std::collections::HashMap;
use crate::config::get_global_config;
use reqwest::Client;
use serde_json::Value;
use tracing::{info, warn, debug, info_span, Instrument};
use crate::metrics;
use crate::telemetry::ProgressCounter;

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
        info!(height = max_height, "Updated sync_height");
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
/// `bulk` selects the per-block write durability mode passed down to
/// `process_blk_file`: `true` on the initial full reindex (WAL disabled — the
/// DB is reconstructible from the `.blk` files), `false` on the live/RPC
/// catch-up path (WAL kept so a crash stays recoverable). It changes durability
/// only, never the bytes written.
pub async fn process_files_parallel(
    entries: Vec<PathBuf>,
    db_arc: Arc<DB>,
    state: AppState,
    max_concurrent: usize,
    bulk: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let _span = info_span!("parallel_processing",
        file_count = %"calculating",
        max_workers = max_concurrent
    ).entered();
    info!("Starting parallel block file processing");
    
    // [F3] CRITICAL: Validate canonical metadata completeness BEFORE parallel processing
    // This prevents the height=0 bug where all transactions get stored with height=0
    // instead of their correct heights due to missing height→hash mappings.
    info!("[F3] Validating canonical chain metadata before parallel processing");
    
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
        
        info!(height_count = height_count, "[F3] Canonical metadata validated - complete and contiguous");
    } else {
        info!("[F3] No canonical metadata found - will assign heights dynamically (normal for first sync)");
    }
    
    info!(workers = max_concurrent, "Starting parallel file processing");
    
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
    info!(
        total_files = total_files,
        first_file = ?blk_files.first().map(|p| p.file_name()),
        last_file = ?blk_files.last().map(|p| p.file_name()),
        "Processing blk files in REVERSE order (newest first)"
    );
    
    // Semaphore to limit concurrent file processing
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    
    // Progress tracking
    let completed = Arc::new(tokio::sync::Mutex::new(0_usize));

    // Running max canonical block height seen across files, flushed to
    // sync_height on a throttle below — replaces the per-file full-CF scan of
    // chain_metadata (~O(files x blocks)). `last_written` is seeded from the
    // stored sync_height so the throttled writes stay monotonic (on the leveldb
    // path sync_height is set before parse and must not move backwards).
    let running_max = Arc::new(AtomicI32::new(-1));
    let last_written = Arc::new(AtomicI32::new(get_sync_height(&db_arc).unwrap_or(0)));
    
    // Process files with controlled concurrency
    let tasks: Vec<_> = blk_files
        .into_iter()
        .map(|file_path| {
            let sem = semaphore.clone();
            let db = db_arc.clone();
            let st = state.clone();
            let completed_clone = completed.clone();
            let running_max = running_max.clone();
            let last_written = last_written.clone();

            let file_name = file_path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            // Attach the per-file span with `.instrument()` (below) instead of
            // `span.entered()`: an entered guard held across the .await points in
            // this task stays on the thread-local span stack while the task is
            // parked, so a sibling file task resumed on the same worker nests
            // UNDER it (observed blk4:blk3:blk2:blk1:blk0), inflating every log
            // line. `.instrument` enters the span only around each poll.
            let file_span = info_span!("process_blk_file", file_name = %file_name);

            async move {
                debug!("Processing blk file");
                
                // Acquire permit - if this fails, semaphore is closed (shutdown).
                // The permit is held for the entire duration of the blocking task
                // below (it is dropped when this scope ends), so the Semaphore(8)
                // still bounds concurrency to `max_concurrent` files at a time.
                let _permit = match sem.acquire().await {
                    Ok(permit) => permit,
                    Err(e) => {
                        warn!(error = %e, "Failed to acquire semaphore permit");
                        return;
                    }
                };

                // LEVER 1: move the CPU-heavy per-file work (Quark/SHA256d block
                // hashing, per-tx txid SHA256d, fixed-width chainwork add) OFF the
                // tokio async worker threads and onto the blocking pool.
                //
                // `process_blk_file` is an async, `!Send` future (it holds the
                // `AsyncCursor` borrow across awaits), so it cannot be `tokio::spawn`-ed
                // onto the multi-threaded runtime. Instead we hand it to
                // `spawn_blocking` and drive it to completion on a dedicated
                // single-threaded (`current_thread`) tokio runtime built *inside*
                // the blocking thread. A current-thread runtime accepts `!Send`
                // futures, so the parse logic is reused byte-for-byte and unchanged;
                // only the CPU now runs on a blocking thread instead of starving the
                // async I/O executor. Everything moved into the closure is
                // `Send + 'static`: the `Arc<DB>`, the cloned `AppState` (two Arcs),
                // the owned `PathBuf`, and the `bool` durability flag. The RocksDB
                // write batching inside `process_blk_file` is untouched.
                let file_for_task = file_path.clone();
                let db_for_task = db.clone();
                let state_for_task = st;
                let join = tokio::task::spawn_blocking(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("failed to build current-thread runtime for blk parsing");
                    rt.block_on(process_blk_file(
                        state_for_task,
                        file_for_task,
                        db_for_task,
                        bulk,
                    ))
                })
                .await;

                match join {
                    Ok(Ok(height_opt)) => {
                        if let Some(h) = height_opt {
                            running_max.fetch_max(h, Ordering::Relaxed);
                        }
                    }
                    Ok(Err(e)) => {
                        warn!(file = %file_path.display(), error = %e, "Failed to process blk file");
                        let _ = save_file_as_incomplete(&db, &file_path).await;
                    }
                    Err(join_err) => {
                        warn!(file = %file_path.display(), error = %join_err, "blk parsing task panicked");
                        let _ = save_file_as_incomplete(&db, &file_path).await;
                    }
                }
                
                // Update progress
                let mut count = completed_clone.lock().await;
                *count += 1;
                let current = *count;
                drop(count);
                
                let progress = (current as f64 / total_files as f64) * 100.0;
                info!(current = current, total = total_files, progress_pct = format!("{:.1}", progress), "File progress");
                
                // Throttled progress refresh from the in-memory running max,
                // replacing the old per-file full-CF scan of chain_metadata
                // (~O(files x blocks)). `cur > last_written` keeps it monotonic;
                // the authoritative refresh after the loop writes the final
                // value. Fire every 16 files and on the last file so chains with
                // fewer than 16 files still surface progress.
                let cur = running_max.load(Ordering::Relaxed);
                if (current % 16 == 0 || current == total_files)
                    && cur > last_written.load(Ordering::Relaxed)
                {
                    if set_sync_height(&db, cur).is_ok() {
                        last_written.store(cur, Ordering::Relaxed);
                    } else {
                        warn!("Failed to update sync_height");
                    }
                }
            }
            .instrument(file_span)
        })
        .collect();
    
    // Execute all tasks concurrently
    futures::future::join_all(tasks).await;
    
    info!("All blk*.dat files processed");
    
    // CRITICAL: Update sync_height to reflect all blocks processed
    // This ensures the next phase (RPC monitoring) knows our true current height
    info!("Updating sync height from all processed blocks");
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
        info!(height_mappings = height_key_count, "Canonical chain metadata exists (from leveldb) - extending with new blocks");
        // CRITICAL: Always resolve to pick up new blocks beyond the leveldb import
        resolve_block_heights(&db_arc, bulk).await?;
        info!("Chain resolution complete (extended with new blocks)");
    } else {
        // FALLBACK: Only resolve if no canonical metadata exists
        info!("No leveldb metadata found - building chain from blk files");
        resolve_block_heights(&db_arc, bulk).await?;
        info!("Chain building complete");
    }
    
    // CRITICAL: Update sync_height AGAIN after chain resolution to pick up newly resolved heights
    info!("Updating sync height after chain resolution");
    update_sync_height_from_metadata(&db_arc).await?;
    
    info!("BLK FILE PROCESSING COMPLETE");
    
    Ok(())
}

/// Resolve block heights by following the blockchain from genesis
/// Optimized O(n) version using hash map for instant lookups
/// Now with RPC validation at checkpoints to ensure we follow the canonical chain
///
/// `bulk` selects the write durability for the height/hash metadata written
/// here: `true` disables the WAL on the initial reindex path (reconstructible
/// DB), `false` keeps it for the live/RPC catch-up path. Bytes are identical.
async fn resolve_block_heights(db: &Arc<DB>, bulk: bool) -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::HashMap;
    
    let cf_blocks = db.cf_handle("blocks")
        .ok_or("blocks CF not found")?;
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    info!("Building hash map (loading all blocks into memory)");

    // [A] Single scan of the blocks CF building only the fields the chain-walk
    // and chainwork BFS actually read — NO full-header copies are retained.
    //
    // Two maps replace the previous three full-chain hash maps
    // (children_map-with-headers, blocks_map, and calculate_all_chainwork's
    // internal parent_map/bits_map):
    //
    //   children_map: prev_hash -> Vec<block_hash>
    //     Built over blocks with header len >= 68 (identical predicate to before).
    //     The previous code stored (block_hash, header_bytes) here but only ever
    //     read block_hash (tip discovery, line below discards the header with `_`),
    //     so dropping the header bytes is output-identical. `total_blocks` is the
    //     same count as before.
    //
    //   block_meta: block_hash -> (prev_hash[32], n_bits)
    //     Built over blocks with header len >= 80 — exactly the set
    //     calculate_all_chainwork used to keep (its parent_map/bits_map applied a
    //     `< 80` skip). prev_hash (bytes 4..36) and n_bits (bytes 72..76) are the
    //     ONLY header fields either the chainwork BFS or the genesis chain-walk
    //     consult, so this is byte-for-byte equivalent to passing full headers.
    //
    // Real PIVX block headers are always >= 80 bytes, so the previous
    // blocks_map (which the walk indexed with a `len >= 36` guard) and this
    // >= 80 block_meta cover the same blocks. A block referenced as a parent but
    // shorter than 80 bytes is absent from block_meta and the walk breaks with a
    // chain-gap error — exactly as the old code's "header too short" / "not found
    // in blocks_map" branches did, and in both cases NO height metadata is
    // written (the function returns Err before the assignment loop), so the
    // stored output is unchanged.
    // Block hashes are fixed 32-byte keys; key these maps by [u8; 32] (inline,
    // Copy) instead of Vec<u8> to drop a separate heap allocation + 24-byte
    // header per hash across ~5.5M blocks (large RSS + allocator-churn win).
    let mut children_map: HashMap<[u8; 32], Vec<[u8; 32]>> = HashMap::new();
    let mut block_meta: HashMap<[u8; 32], ([u8; 32], u32)> = HashMap::new();
    let iter = db.iterator_cf(&cf_blocks, rocksdb::IteratorMode::Start);
    let mut total_blocks = 0;

    for item in iter {
        if let Ok((hash, header_bytes)) = item {
            // A header's key is always the 32-byte block hash. A stray non-32-byte
            // key cannot be a header hash, so skipping it is output-identical (it
            // could never be on the canonical chain). Only such keys carry >= 68
            // byte values, so total_blocks is unaffected.
            let hash32: [u8; 32] = match <[u8; 32]>::try_from(&hash[..]) {
                Ok(h) => h,
                Err(_) => continue,
            };
            if header_bytes.len() >= 68 {
                let prev_hash: [u8; 32] = header_bytes[4..36].try_into().unwrap();
                children_map.entry(prev_hash)
                    .or_default()
                    .push(hash32);

                total_blocks += 1;
            }
            if header_bytes.len() >= 80 {
                let prev_hash: [u8; 32] = header_bytes[4..36].try_into().unwrap();
                let n_bits = u32::from_le_bytes([
                    header_bytes[72], header_bytes[73], header_bytes[74], header_bytes[75],
                ]);
                block_meta.insert(hash32, (prev_hash, n_bits));
            }
        }
    }

    info!(total_blocks = total_blocks, "Loaded blocks into memory");

    info!("Calculating accumulated chainwork (Bitcoin consensus)");
    let chainwork_map = calculate_all_chainwork(db, &block_meta)?;
    info!(blocks = chainwork_map.len(), "Chainwork calculated");
    
    info!("Finding best chain tip");
    
    // STEP 3A: Find blocks with no children (potential tips)
    let mut all_blocks: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
    let mut has_children: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
    
    for (parent_hash, children) in &children_map {
        all_blocks.insert(parent_hash.clone());
        for child_hash in children {
            all_blocks.insert(child_hash.clone());
            has_children.insert(parent_hash.clone());
        }
    }
    
    let potential_tips: Vec<[u8; 32]> = all_blocks
        .difference(&has_children)
        .cloned()
        .collect();
    
    debug!(tips = potential_tips.len(), "Found potential chain tips");

    // [M2] OPTIMIZATION: Use chainwork-only selection instead of RPC validating every tip
    // Old approach: RPC validate 1000+ tips (100+ seconds)
    // New approach: Use chainwork to find best tip, only RPC validate the chosen one (1 call)
    
    debug!("Selecting best tip by chainwork");
    
    // Find tip with highest chainwork
    let mut best_tip: Option<([u8; 32], [u8; 32])> = None;
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
    
    info!(tip = &tip_hex[..16], chainwork = %hex::encode(best_chainwork), "Selected best tip by chainwork");
    
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
            
            debug!("Validating chosen tip with RPC");
            
            let body = serde_json::json!({
                "jsonrpc": "1.0",
                "id": "rbx",
                "method": "getblock",
                "params": [tip_hex, 1]
            });
            
            // Use async reqwest client to avoid runtime nesting issues
            let client = reqwest::Client::new();
            let rpc_result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                client
                    .post(&url)
                    .basic_auth(&user, Some(&pass))
                    .json(&body)
                    .send()
            ).await;
            
            match rpc_result {
                Ok(Ok(resp)) if resp.status().is_success() => {
                    if let Ok(text) = resp.text().await {
                        if let Ok(json_val) = serde_json::from_str::<Value>(&text) {
                            if let Some(height) = json_val.get("result").and_then(|r| r.get("height")).and_then(|h| h.as_i64()) {
                                info!(height = height, "RPC confirms tip");
                                Some(height as i32)
                            } else {
                                warn!("RPC returned block but no height field");
                                None
                            }
                        } else {
                            warn!("RPC returned invalid JSON");
                            None
                        }
                    } else {
                        warn!("RPC returned non-text response");
                        None
                    }
                }
                Ok(Ok(resp)) => {
                    warn!(status = %resp.status(), "RPC returned error status");
                    None
                }
                Ok(Err(e)) => {
                    warn!(error = %e, "RPC error");
                    None
                }
                Ok(Err(e)) => {
                    warn!(error = %e, "RPC task panic");
                    None
                }
                Err(_) => {
                    warn!("RPC timeout (>10s)");
                    None
                }
            }
        }
        _ => {
            debug!("RPC not configured - trusting chainwork selection");
            None
        }
    };
    
    let highest_height_opt = rpc_validation;

    // [M2] Old O(n²) loop removed - we now select best tip via chainwork only
    // This eliminates 1000+ RPC calls (saving 100+ seconds on typical sync)
    
    // Display result
    if let Some(h) = highest_height_opt {
        info!(height = h, hash = %tip_hex, "Selected canonical chain tip");
    } else {
        info!(hash = %tip_hex, "Selected canonical chain tip (chainwork-based, height unknown)");
    }
    
    // STEP 3C: Walk backwards from the HIGHEST tip to genesis
    let mut highest_height: i32 = 0;
    let mut have_highest_height = false;
    if let Some(h) = highest_height_opt {
        highest_height = h;
        have_highest_height = true;
        info!(height = highest_height, "Walking backwards from highest tip to genesis");
    } else {
        info!("Walking backwards from highest tip to genesis (height unknown)");
    }
    
    let mut chain_path: Vec<[u8; 32]> = Vec::new();
    let mut current_hash = highest_tip;
    let genesis_parent = [0u8; 32];
    let mut steps = 0;
    
    loop {
        chain_path.push(current_hash.clone());
        steps += 1;

        // Look up this block's prev_hash from the compact block_meta map.
        // block_meta stores prev_hash (header bytes 4..36) directly, so this is
        // identical to the previous `header[4..36]` read. block_meta covers every
        // block with a >= 80-byte header (all real PIVX blocks); a parent that is
        // absent here is the same condition the old code hit as "header too
        // short" / "not found in blocks_map" — both break and yield a chain-gap
        // error with NO metadata written.
        if let Some((prev_hash, _n_bits)) = block_meta.get(&current_hash) {
            let prev_hash = prev_hash.clone();

            // Check if we reached genesis
            if prev_hash == genesis_parent {
                info!("Reached genesis block");
                chain_path.push(prev_hash);
                break;
            }

            // Move to parent block
            current_hash = prev_hash;
        } else {
            let display_hash: Vec<u8> = current_hash.iter().rev().cloned().collect();
            warn!(
                step = steps,
                missing_hash = %hex::encode(&display_hash),
                chain_breaks_at = steps,
                "Block not found in block_meta"
            );
            break;
        }
    }
    
    let reached_genesis = chain_path.last().map(|h| h == &genesis_parent).unwrap_or(false);
    
    debug!(chain_length = chain_path.len(), "Chain path built");
    
    if !reached_genesis {
        warn!("Chain did NOT reach genesis - checking for gap");
        
        // STEP 3C2: The chain broke - now walk backwards from the MISSING block to see if it reaches genesis
        let missing_hash_display: Vec<u8> = current_hash.iter().rev().cloned().collect();
        warn!(missing_block = %hex::encode(&missing_hash_display), "Gap detected - recommend full resync");
        
        // [M2] Note: Gap filling via RPC removed as part of optimization
        return Err(format!("Chain gap detected at block {}", hex::encode(&missing_hash_display)).into());
    } else {
        info!("Chain successfully reached genesis");
    }
    
    // If we couldn't reach genesis and don't have an RPC-supplied tip height,
    // we cannot reliably assign heights — abort with a clear error so the
    // operator can supply RPC access or rebuild metadata with leveldb tools.
    if !reached_genesis && !have_highest_height {
        return Err("Chain did not reach genesis and tip height is unknown (RPC required to continue)".into());
    }

    // STEP 3D: Reverse and assign heights
    info!("Assigning heights to canonical chain");
    chain_path.reverse();
    
    let start_idx = if reached_genesis && chain_path[0] == genesis_parent { 1 } else { 0 };

    // On the initial reindex these per-block metadata writes are pure WAL fsync
    // overhead; disable the WAL on the bulk path (kept on for live catch-up).
    let wo = bulk_write_options();
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
        if bulk {
            db.put_cf_opt(&cf_metadata, &h_key, height.to_le_bytes(), &wo)?;
        } else {
            db.put_cf(&cf_metadata, &h_key, height.to_le_bytes())?;
        }

        // Store height -> hash mapping (in DISPLAY format)
        let display_hash: Vec<u8> = block_hash.iter().rev().cloned().collect();
        if bulk {
            db.put_cf_opt(&cf_metadata, height.to_le_bytes(), &display_hash, &wo)?;
        } else {
            db.put_cf(&cf_metadata, height.to_le_bytes(), &display_hash)?;
        }
    }
    
    let chain_height = if reached_genesis {
        (chain_path.len() - start_idx - 1) as i32
    } else {
        highest_height
    };

    info!(
        chain_blocks = chain_path.len() - start_idx,
        start_height = if reached_genesis { 0 } else { highest_height - (chain_path.len() - start_idx - 1) as i32 },
        tip_height = chain_height,
        "Canonical chain established"
    );
    
    // Calculate statistics
    let orphaned_count = total_blocks - (chain_path.len() - start_idx);
    info!(
        total_blocks = total_blocks,
        canonical_chain = chain_path.len() - start_idx,
        tip_height = chain_height,
        orphaned = orphaned_count,
        "Chain statistics"
    );
    
    Ok(())
}
