/// Canonical Chain Builder
/// 
/// Determines the canonical blockchain from blk*.dat files using chainwork,
/// exactly as PIVX Core does in validation.cpp
/// 
/// Algorithm (from PIVX Core):
/// 1. Load all blocks into index (mapBlockIndex)
/// 2. Calculate chainwork for each block (nChainWork = parent.nChainWork + GetBlockProof)
/// 3. Find tip with highest chainwork (FindMostWorkChain)
/// 4. Walk backwards from that tip (this is the canonical chain)

use std::collections::HashMap;
use std::path::PathBuf;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use sha2::{Sha256, Digest};
use crate::chainwork::calculate_work_from_bits;
use super::call_quark_hash;
use num_bigint::BigUint;

/// Block index entry
#[derive(Clone, Debug)]
pub struct BlockIndexEntry {
    pub hash: Vec<u8>,
    pub prev_hash: Vec<u8>,
    pub n_bits: u32,
    pub chainwork: Option<[u8; 32]>,  // Cumulative proof-of-work
    pub height: Option<i32>,
}

/// Calculate block hash based on version
fn calculate_block_hash(header_bytes: &[u8]) -> Vec<u8> {
    if header_bytes.len() < 4 {
        return vec![0u8; 32];
    }
    
    let n_version = i32::from_le_bytes([header_bytes[0], header_bytes[1], header_bytes[2], header_bytes[3]]);
    
    let hash_size = match n_version {
        0..=3 => 80,
        4..=6 => 112,
        7 => 80,
        _ => 112,
    };
    
    let hash_bytes = &header_bytes[..hash_size.min(header_bytes.len())];
    
    match n_version {
        0..=3 => {
            // For v0-v3, use Quark hash on first 80 bytes
            call_quark_hash(hash_bytes).to_vec()
        }
        _ => {
            // For v4+, use SHA256d
            let first_hash = Sha256::digest(hash_bytes);
            Sha256::digest(&first_hash).to_vec()
        }
    }
}

/// Scan all blk files and build block index
pub fn scan_all_blocks(blk_dir: &str) -> Result<HashMap<Vec<u8>, BlockIndexEntry>, Box<dyn std::error::Error>> {
    println!("\n📂 Phase 1: Scanning all blk*.dat files...");
    
    let mut block_index: HashMap<Vec<u8>, BlockIndexEntry> = HashMap::new();
    let mut total_blocks = 0;
    
    // Scan blk files (0 to 141)
    for file_num in 0..=141 {
        let blk_path = PathBuf::from(blk_dir).join(format!("blk{:05}.dat", file_num));
        
        if !blk_path.exists() {
            break;
        }
        
        let mut file = File::open(&blk_path)?;
        let file_size = file.metadata()?.len();
        let mut pos = 0u64;
        let mut blocks_in_file = 0;
        
        while pos < file_size {
            file.seek(SeekFrom::Start(pos))?;
            
            // Read magic bytes (4) and size (4)
            let mut header = [0u8; 8];
            if file.read_exact(&mut header).is_err() {
                break;
            }
            
            let block_size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
            
            if block_size == 0 || block_size > 10_000_000 {
                break;
            }
            
            // Read block data
            let mut block_data = vec![0u8; block_size];
            if file.read_exact(&mut block_data).is_err() {
                break;
            }
            
            // Need at least 80 bytes for header
            if block_data.len() >= 80 {
                // Calculate hash
                let hash = calculate_block_hash(&block_data);
                
                // Extract prev_hash (bytes 4-36)
                let prev_hash = block_data[4..36].to_vec();
                
                // Extract nBits (bytes 72-76)
                let n_bits = u32::from_le_bytes([
                    block_data[72],
                    block_data[73],
                    block_data[74],
                    block_data[75],
                ]);
                
                block_index.insert(hash.clone(), BlockIndexEntry {
                    hash,
                    prev_hash,
                    n_bits,
                    chainwork: None,  // Will calculate in next phase
                    height: None,     // Will calculate in next phase
                });
                
                blocks_in_file += 1;
                total_blocks += 1;
            }
            
            pos += 8u64 + block_size as u64;
        }
        
        if file_num % 10 == 0 || blocks_in_file > 0 {
            println!("  blk{:05}.dat: {} blocks", file_num, blocks_in_file);
        }
    }
    
    println!("✅ Scanned {} total blocks (includes orphans/forks)", total_blocks);
    
    // Debug: find blocks with all-zero prev_hash (potential genesis blocks)
    let mut genesis_candidates = Vec::new();
    for (hash, entry) in block_index.iter() {
        if entry.prev_hash.iter().all(|&b| b == 0) {
            genesis_candidates.push(hash.clone());
        }
    }
    println!("  Found {} blocks with zero prev_hash (genesis candidates)", genesis_candidates.len());
    for (i, hash) in genesis_candidates.iter().take(5).enumerate() {
        println!("    Candidate {}: {}", i + 1, hex::encode(hash));
    }
    
    Ok(block_index)
}

/// Calculate chainwork for all blocks (Phase 2)
/// This implements PIVX's LoadBlockIndexDB chainwork calculation
pub fn calculate_chainwork_for_all(
    block_index: &mut HashMap<Vec<u8>, BlockIndexEntry>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n⚙️  Phase 2: Calculating chainwork (cumulative PoW)...");
    
    // PIVX genesis hash in display format (big-endian/reversed)
    let genesis_hash_display = "0000041e482b9b9691d98eefb48473405c0b8ec31b76df3797c74a78680ef818";
    // Convert to internal format (little-endian)
    let mut genesis_hash_bytes = hex::decode(genesis_hash_display)?;
    genesis_hash_bytes.reverse(); // Reverse to get internal format
    
    println!("  Looking for genesis: {}", genesis_hash_display);
    println!("  Internal format: {}", hex::encode(&genesis_hash_bytes));
    
    // Build parent→children mapping for forward traversal
    let mut children_map: HashMap<Vec<u8>, Vec<Vec<u8>>> = HashMap::new();
    for (hash, entry) in block_index.iter() {
        children_map
            .entry(entry.prev_hash.clone())
            .or_insert_with(Vec::new)
            .push(hash.clone());
    }
    
    // BFS from genesis to calculate chainwork
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(genesis_hash_bytes.clone());
    
    // Genesis has no parent, chainwork = its own proof
    if let Some(genesis_entry) = block_index.get_mut(&genesis_hash_bytes) {
        let genesis_work = calculate_work_from_bits(genesis_entry.n_bits);
        genesis_entry.chainwork = Some(genesis_work);
        genesis_entry.height = Some(0);
        println!("  Found genesis block");
    } else {
        return Err("Genesis block not found in blk files!".into());
    }
    
    let mut processed = 1;
    
    while let Some(current_hash) = queue.pop_front() {
        let current_entry = match block_index.get(&current_hash) {
            Some(e) => e.clone(),
            None => {
                eprintln!("Warning: Block not found: {}", hex::encode(&current_hash));
                continue;
            }
        };
        
        let current_chainwork = match current_entry.chainwork {
            Some(work) => work,
            None => {
                eprintln!("Warning: Block missing chainwork: {}", hex::encode(&current_hash));
                continue;
            }
        };
        
        let current_height = match current_entry.height {
            Some(h) => h,
            None => {
                eprintln!("Warning: Block missing height: {}", hex::encode(&current_hash));
                continue;
            }
        };
        
        // Process all children
        if let Some(children) = children_map.get(&current_hash) {
            for child_hash in children {
                let child_entry = block_index.get_mut(child_hash).unwrap();
                
                // Calculate child's chainwork = parent chainwork + child's proof
                let child_proof = calculate_work_from_bits(child_entry.n_bits);
                
                // Add using BigUint for arbitrary precision
                let parent_big = BigUint::from_bytes_be(&current_chainwork);
                let proof_big = BigUint::from_bytes_be(&child_proof);
                let total = parent_big + proof_big;
                
                let work_bytes = total.to_bytes_be();
                let mut chainwork = [0u8; 32];
                let start = 32 - work_bytes.len().min(32);
                chainwork[start..].copy_from_slice(&work_bytes[..work_bytes.len().min(32)]);
                
                child_entry.chainwork = Some(chainwork);
                child_entry.height = Some(current_height + 1);
                
                queue.push_back(child_hash.clone());
                processed += 1;
                
                if processed % 100_000 == 0 {
                    println!("    Processed {} blocks...", processed);
                }
            }
        }
    }
    
    println!("✅ Calculated chainwork for {} blocks", processed);
    
    Ok(())
}

/// Find the canonical chain tip (Phase 3)
/// This implements PIVX's FindMostWorkChain - finds tip with highest chainwork
pub fn find_best_tip(
    block_index: &HashMap<Vec<u8>, BlockIndexEntry>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    println!("\n🔍 Phase 3: Finding canonical chain tip (highest chainwork)...");
    
    // Build set of all prev_hashes (blocks that have children)
    let mut has_children = std::collections::HashSet::new();
    for entry in block_index.values() {
        has_children.insert(entry.prev_hash.clone());
    }
    
    // Find all tips (blocks with no children)
    let mut tips = Vec::new();
    for (hash, entry) in block_index.iter() {
        if !has_children.contains(hash) && entry.chainwork.is_some() {
            tips.push((hash.clone(), entry.chainwork.unwrap(), entry.height.unwrap()));
        }
    }
    
    println!("  Found {} tip(s) (potential chain ends)", tips.len());
    
    if tips.is_empty() {
        return Err("No tips found!".into());
    }
    
    // Find tip with highest chainwork (this is FindMostWorkChain logic)
    tips.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by chainwork descending
    
    // Show top 10 tips
    for (i, (hash, work, height)) in tips.iter().take(10).enumerate() {
        println!("  Tip {}: height={}, chainwork={}, hash={}",
            i + 1,
            height,
            hex::encode(&work[28..32]), // Last 4 bytes for display
            hex::encode(&hash[..8])
        );
    }
    
    let best_tip = tips[0].0.clone();
    let best_height = tips[0].2;
    
    println!("\n✅ Best tip: height={}, hash={}", best_height, hex::encode(&best_tip[..16]));
    
    Ok(best_tip)
}

/// Build canonical chain by walking backwards from tip (Phase 4)
pub fn build_canonical_chain(
    tip_hash: &[u8],
    block_index: &HashMap<Vec<u8>, BlockIndexEntry>,
) -> Result<Vec<(i32, Vec<u8>)>, Box<dyn std::error::Error>> {
    println!("\n⬅️  Phase 4: Walking backwards from tip to build canonical chain...");
    
    let mut chain = Vec::new();
    let mut current_hash = tip_hash.to_vec();
    
    // Genesis hash in internal format (little-endian)
    let mut genesis_hash_bytes = hex::decode("0000041e482b9b9691d98eefb48473405c0b8ec31b76df3797c74a78680ef818")?;
    genesis_hash_bytes.reverse();
    
    loop {
        let entry = block_index.get(&current_hash)
            .ok_or_else(|| format!("Block not found: {}", hex::encode(&current_hash)))?;
        
        let height = entry.height.ok_or("Block missing height")?;
        chain.push((height, current_hash.clone()));
        
        // Check if we reached genesis
        if current_hash == genesis_hash_bytes || entry.prev_hash.iter().all(|&b| b == 0) {
            break;
        }
        
        current_hash = entry.prev_hash.clone();
        
        if chain.len() % 100_000 == 0 {
            println!("    Walked {} blocks...", chain.len());
        }
    }
    
    // Reverse to get genesis → tip order
    chain.reverse();
    
    println!("✅ Canonical chain: {} blocks (height 0 to {})", chain.len(), chain.len() - 1);
    
    Ok(chain)
}

/// Main entry point - build canonical chain from blk files
pub fn build_canonical_chain_from_blk_files(
    blk_dir: &str,
) -> Result<Vec<(i32, Vec<u8>)>, Box<dyn std::error::Error>> {
    println!("\n🚀 Building Canonical Chain (PIVX Core Algorithm)");
    println!("====================================================\n");
    
    // Phase 1: Scan all blocks
    let mut block_index = scan_all_blocks(blk_dir)?;
    
    // Phase 2: Calculate chainwork
    calculate_chainwork_for_all(&mut block_index)?;
    
    // Phase 3: Find best tip
    let best_tip = find_best_tip(&block_index)?;
    
    // Phase 4: Build canonical chain
    let canonical_chain = build_canonical_chain(&best_tip, &block_index)?;
    
    println!("\n✅ SUCCESS! Canonical chain built with {} blocks", canonical_chain.len());
    
    Ok(canonical_chain)
}
