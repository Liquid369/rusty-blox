use std::path::PathBuf;
use std::sync::Arc;
use rocksdb::DB;
use tokio::sync::Semaphore;
use crate::types::AppState;
use crate::blocks::process_blk_file;
use crate::db_utils::save_file_as_incomplete;
use rustyblox::chainwork::calculate_all_chainwork;
use hex;
use std::collections::HashMap;

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
    
    println!("Found {} block files to process (processing in REVERSE order - newest first)", blk_files.len());
    println!("First file: {:?}", blk_files.first().map(|p| p.file_name()));
    println!("Last file: {:?}", blk_files.last().map(|p| p.file_name()));
    
    // Semaphore to limit concurrent file processing
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    
    // Process files with controlled concurrency
    let tasks: Vec<_> = blk_files
        .into_iter()
        .map(|file_path| {
            let sem = semaphore.clone();
            let db = db_arc.clone();
            let st = state.clone();
            
            async move {
                // Acquire permit
                let _permit = sem.acquire().await.unwrap();
                
                // Process file (this is async but not Send, so we run it directly)
                if let Err(e) = process_blk_file(st, file_path.clone(), db.clone()).await {
                    eprintln!("Failed to process {}: {}", file_path.display(), e);
                    let _ = save_file_as_incomplete(&db, &file_path).await;
                }
            }
        })
        .collect();
    
    // Execute all tasks concurrently
    futures::future::join_all(tasks).await;
    
    println!("\n✅ All blk*.dat files processed!");
    
    // Check if canonical chain metadata already exists (from leveldb phase)
    let cf_metadata = db_arc.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    // Check if genesis block exists in metadata (height 0)
    let has_canonical_metadata = db_arc.get_cf(&cf_metadata, &0i32.to_le_bytes())?.is_some();
    
    if has_canonical_metadata {
        println!("\n✅ Canonical chain metadata already exists (from leveldb)");
        println!("   Skipping chain resolution - using pre-built canonical chain");
    } else {
        // FALLBACK: Only resolve if no canonical metadata exists
        println!("\n🔗 Phase 2: Resolving block heights (building chain)...");
        println!("   (No leveldb metadata found, building chain from blk files)");
        resolve_block_heights(&db_arc).await?;
        println!("✅ Chain building complete!");
    }
    
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
                    .or_insert_with(Vec::new)
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
    let block_2678400_hash = hex::decode("bde2ea24bba50fb80a9c98b67e76a4407d0251aa61ac13bbcfa7feccab57bce7")
        .expect("Failed to decode hash");
    if let Some(children) = children_map.get(&block_2678400_hash) {
        println!("  DEBUG: Block 2,678,400 has {} children in map (BEFORE chainwork calc)", children.len());
        for (i, (child_hash, _)) in children.iter().enumerate() {
            let display_hash: Vec<u8> = child_hash.iter().rev().cloned().collect();
            println!("    Child {}: {}", i + 1, hex::encode(&display_hash));
        }
    } else {
        println!("  DEBUG: Block 2,678,400 NOT FOUND in children_map!");
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
    
    let chainwork_map = calculate_all_chainwork(&db, &blocks_map)?;
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
    
    // STEP 3B: Check each tip with RPC to get its HEIGHT
    // Then work backwards from the HIGHEST one
    println!("  Checking tips with RPC to find the highest one...");
    
    let mut tips_with_heights: Vec<(Vec<u8>, i32)> = Vec::new();
    
    for (idx, tip_hash) in potential_tips.iter().enumerate() {
        if idx % 1000 == 0 && idx > 0 {
            println!("    Checked {} tips with RPC...", idx);
        }
        
        // Convert to display format for RPC
        let tip_display: Vec<u8> = tip_hash.iter().rev().cloned().collect();
        let tip_hex = hex::encode(&tip_display);
        
        // Query RPC for this block's height
        let rpc_result = std::process::Command::new("/Users/liquid/Downloads/pivx-5.6.1/bin/pivx-cli")
            .args(&["getblock", &tip_hex, "1"])
            .output();
        
        if let Ok(output) = rpc_result {
            if output.status.success() {
                let json_str = String::from_utf8_lossy(&output.stdout);
                // Parse height from JSON (simple string search)
                if let Some(height_start) = json_str.find("\"height\":") {
                    let height_substr = &json_str[height_start + 9..];
                    if let Some(comma_pos) = height_substr.find(',') {
                        let height_str = height_substr[..comma_pos].trim();
                        if let Ok(height) = height_str.parse::<i32>() {
                            tips_with_heights.push((tip_hash.clone(), height));
                            
                            if tips_with_heights.len() <= 10 {
                                println!("    Tip at height {}: {}", height, &tip_hex[..16]);
                            }
                        }
                    }
                }
            }
        }
        
        // Stop after checking 5000 tips - should be enough to find the highest
        if idx >= 5000 {
            println!("    Checked 5000 tips, stopping RPC queries");
            break;
        }
    }
    
    println!("  Found {} tips with known heights via RPC", tips_with_heights.len());
    
    if tips_with_heights.is_empty() {
        return Err("No tips found with valid heights from RPC!".into());
    }
    
    // Sort by height (descending) to find the highest
    tips_with_heights.sort_by(|a, b| b.1.cmp(&a.1));
    
    let (highest_tip, highest_height) = &tips_with_heights[0];
    let tip_display: Vec<u8> = highest_tip.iter().rev().cloned().collect();
    
    println!("\n  📍 Found HIGHEST tip in database:");
    println!("     Height: {}", highest_height);
    println!("     Hash: {}", hex::encode(&tip_display));
    
    // Show top 10 tips
    println!("\n  Top 10 highest tips:");
    for (idx, (tip, height)) in tips_with_heights.iter().take(10).enumerate() {
        let tip_disp: Vec<u8> = tip.iter().rev().cloned().collect();
        println!("    #{}: Height {} - {}", idx + 1, height, hex::encode(&tip_disp));
    }
    
    // STEP 3C: Walk backwards from the HIGHEST tip to genesis
    println!("\n  Walking backwards from highest tip (height {}) to genesis...", highest_height);
    
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
        
        // Try to find this block's parent and continue walking backwards
        println!("\n  Attempting second backwards walk from the gap point...");
        
        // Query RPC for the missing block to get its parent
        let rpc_result = std::process::Command::new("/Users/liquid/Downloads/pivx-5.6.1/bin/pivx-cli")
            .args(&["getblock", &hex::encode(&missing_hash_display), "1"])
            .output();
        
        if let Ok(output) = rpc_result {
            if output.status.success() {
                let json_str = String::from_utf8_lossy(&output.stdout);
                
                // Extract the previousblockhash
                if let Some(prev_start) = json_str.find("\"previousblockhash\":") {
                    let prev_substr = &json_str[prev_start + 22..];
                    if let Some(quote_end) = prev_substr.find('"') {
                        let prev_hash_str = &prev_substr[..quote_end];
                        
                        if let Ok(prev_hash_bytes) = hex::decode(prev_hash_str) {
                            // Convert from display format to internal format
                            let prev_hash_internal: Vec<u8> = prev_hash_bytes.iter().rev().cloned().collect();
                            
                            println!("  Found missing block's parent via RPC: {}", prev_hash_str);
                            println!("  Continuing backwards walk from parent...");
                            
                            // Now walk backwards from this parent
                            let mut second_chain_path: Vec<Vec<u8>> = Vec::new();
                            let mut current_hash_2 = prev_hash_internal.clone();
                            let mut second_reached_genesis = false;
                            
                            for step2 in 0..10_000_000 {
                                if step2 % 100000 == 0 && step2 > 0 {
                                    println!("    Second walk: traced back {} blocks...", step2);
                                }
                                
                                second_chain_path.push(current_hash_2.clone());
                                
                                // Check if this block exists in our database
                                if let Some(header) = blocks_map.get(&current_hash_2) {
                                    if header.len() >= 36 {
                                        let prev_hash_2 = header[4..36].to_vec();
                                        
                                        // Check if we reached genesis
                                        if prev_hash_2 == genesis_parent {
                                            println!("  ✅ Second walk reached genesis block!");
                                            second_chain_path.push(prev_hash_2);
                                            second_reached_genesis = true;
                                            break;
                                        }
                                        
                                        current_hash_2 = prev_hash_2;
                                    } else {
                                        break;
                                    }
                                } else {
                                    let display_2: Vec<u8> = current_hash_2.iter().rev().cloned().collect();
                                    println!("  ⚠️  Second walk: block not found at step {}", step2);
                                    println!("     Missing hash: {}", hex::encode(&display_2));
                                    break;
                                }
                            }
                            
                            println!("  Second chain path length: {} blocks", second_chain_path.len());
                            
                            if second_reached_genesis {
                                println!("  🎉 SUCCESS! We can reach genesis from the high chain!");
                                println!("  Total connected blocks: {} (high chain) + {} (low chain) = {}", 
                                         chain_path.len(), 
                                         second_chain_path.len(),
                                         chain_path.len() + second_chain_path.len());
                                
                                // Merge the two chains
                                // Second chain is in reverse order (tip -> genesis), so reverse it first
                                second_chain_path.reverse();
                                
                                // Remove genesis parent marker if present
                                let second_start_idx = if second_chain_path[0] == genesis_parent { 1 } else { 0 };
                                
                                // Combine: [genesis...gap] + [missing_block] + [gap+1...tip]
                                let mut full_chain = second_chain_path[second_start_idx..].to_vec();
                                full_chain.push(current_hash.clone()); // Add the missing block
                                full_chain.extend(chain_path.iter().rev().cloned()); // Add high chain (reverse it first)
                                
                                // Replace chain_path with the full chain
                                chain_path = full_chain.iter().rev().cloned().collect();
                                
                                println!("  Combined chain length: {} blocks", chain_path.len());
                            } else {
                                println!("  ⚠️  Second walk also failed to reach genesis");
                                println!("  There's still a gap that needs to be filled with RPC");
                            }
                        }
                    }
                }
            }
        }
    } else {
        println!("  ✅ Chain successfully reached genesis!");
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
            highest_height - (chain_path.len() - start_idx - 1 - idx) as i32
        };
        
        // Store hash -> height mapping
        let mut h_key = vec![b'h'];
        h_key.extend_from_slice(block_hash);
        db.put_cf(&cf_metadata, &h_key, &height.to_le_bytes())?;
        
        // Store height -> hash mapping (in DISPLAY format)
        let display_hash: Vec<u8> = block_hash.iter().rev().cloned().collect();
        db.put_cf(&cf_metadata, &height.to_le_bytes(), &display_hash)?;
        
        if height % 100000 == 0 || height < 5 || height >= highest_height - 5 {
            println!("    Height {}: {}", height, hex::encode(&display_hash));
        }
    }
    
    let chain_height = if reached_genesis {
        (chain_path.len() - start_idx - 1) as i32
    } else {
        *highest_height
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
