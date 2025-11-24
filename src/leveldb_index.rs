/// LevelDB Index Reader - Builds canonical chain from PIVX's block index
/// 
/// This module reads PIVX's leveldb block index and constructs the canonical
/// blockchain using the same chainwork-based algorithm as PIVX Core:
/// 1. Parse all CDiskBlockIndex entries from leveldb
/// 2. Calculate chainwork for all blocks (cumulative proof-of-work)
/// 3. Find tip with highest chainwork
/// 4. Walk backwards from that tip to genesis

use rusty_leveldb::{DB, Options, LdbIterator};
use std::collections::HashMap;
use std::path::Path;

// Parse PIVX's VARINT format
pub fn read_varint(data: &[u8], offset: &mut usize) -> Option<u64> {
    if *offset >= data.len() { return None; }
    
    let mut n: u64 = 0;
    
    loop {
        if *offset >= data.len() { return None; }
        
        let ch_data = data[*offset];
        *offset += 1;
        
        if n > (u64::MAX >> 7) {
            return None;
        }
        
        n = (n << 7) | ((ch_data & 0x7F) as u64);
        
        if (ch_data & 0x80) != 0 {
            if n == u64::MAX {
                return None;
            }
            n += 1;
        } else {
            return Some(n);
        }
    }
}

// VARINT with NONNEGATIVE_SIGNED mode (divides by 2)
pub fn read_varint_signed(data: &[u8], offset: &mut usize) -> Option<i64> {
    read_varint(data, offset).map(|v| (v / 2) as i64)
}

// Read a vector (compact size + elements)
pub fn read_vector_bytes(data: &[u8], offset: &mut usize) -> Option<Vec<u8>> {
    let size = read_varint(data, offset)? as usize;
    if *offset + size > data.len() {
        return None;
    }
    let vec = data[*offset..*offset + size].to_vec();
    *offset += size;
    Some(vec)
}

// Convert compact nBits to 256-bit target
fn compact_to_target(n_bits: u32) -> [u64; 4] {
    let size = (n_bits >> 24) as usize;
    let word = n_bits & 0x007fffff;
    
    if size <= 3 {
        let compact = word >> (8 * (3 - size));
        [compact as u64, 0, 0, 0]
    } else {
        let shift = 8 * (size - 3);
        if shift < 64 {
            [(word as u64) << shift, 0, 0, 0]
        } else if shift < 128 {
            [0, (word as u64) << (shift - 64), 0, 0]
        } else if shift < 192 {
            [0, 0, (word as u64) << (shift - 128), 0]
        } else {
            [0, 0, 0, (word as u64) << (shift - 192)]
        }
    }
}

// Calculate block proof from nBits: proof = 2^256 / (target + 1)
fn get_block_proof(n_bits: u32) -> [u64; 4] {
    let target = compact_to_target(n_bits);
    
    // Add 1 to target
    let mut target_plus_one = target;
    let carry = add_u256(&mut target_plus_one, &[1, 0, 0, 0]);
    
    // If target is 0 or target+1 overflows, work is 0
    if carry || (target[0] == 0 && target[1] == 0 && target[2] == 0 && target[3] == 0) {
        return [0, 0, 0, 0];
    }
    
    // Calculate 2^256 / (target + 1)
    div_u256([0, 0, 0, 0, 1], target_plus_one)
}

// Add two 256-bit integers, returns carry
fn add_u256(a: &mut [u64; 4], b: &[u64; 4]) -> bool {
    let mut carry = 0u128;
    for i in 0..4 {
        let sum = (a[i] as u128) + (b[i] as u128) + carry;
        a[i] = sum as u64;
        carry = sum >> 64;
    }
    carry > 0
}

// Divide 320-bit by 256-bit, returns 256-bit quotient
fn div_u256(dividend: [u64; 5], divisor: [u64; 4]) -> [u64; 4] {
    // Simplified division approximation for chainwork comparison
    let mut shift = 0;
    if divisor[3] != 0 {
        shift = 192;
    } else if divisor[2] != 0 {
        shift = 128;
    } else if divisor[1] != 0 {
        shift = 64;
    }
    
    if shift >= 192 {
        let q = (1u64 << 63) / (divisor[3] >> 1).max(1);
        [0, 0, 0, q]
    } else if shift >= 128 {
        let q = (1u64 << 63) / (divisor[2] >> 1).max(1);
        [0, 0, q, 0]
    } else if shift >= 64 {
        let q = (1u64 << 63) / (divisor[1] >> 1).max(1);
        [0, q, 0, 0]
    } else {
        let q = (1u64 << 63) / divisor[0].max(1);
        [q, 0, 0, 0]
    }
}

// Compare two 256-bit integers
fn cmp_u256(a: &[u64; 4], b: &[u64; 4]) -> std::cmp::Ordering {
    for i in (0..4).rev() {
        if a[i] > b[i] {
            return std::cmp::Ordering::Greater;
        } else if a[i] < b[i] {
            return std::cmp::Ordering::Less;
        }
    }
    std::cmp::Ordering::Equal
}

#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub height: i64,
    pub hash_prev: Vec<u8>,
    pub n_bits: u32,
    pub chainwork: Option<[u64; 4]>,
    // File number & data position (if present in CDiskBlockIndex)
    pub file: Option<u64>,
    pub data_pos: Option<u64>,
}

/// Build canonical blockchain from leveldb block index
/// Returns: Vec<(height, block_hash, Option<file>, Option<data_pos>)> in genesis -> tip order
pub fn build_canonical_chain_from_leveldb(
    leveldb_path: &str,
) -> Result<Vec<(i64, Vec<u8>, Option<u64>, Option<u64>)>, Box<dyn std::error::Error>> {
    
    println!("📖 Reading PIVX leveldb block index from: {}", leveldb_path);
    
    let opts = Options::default();
    let mut db = DB::open(Path::new(leveldb_path), opts)?;
    
    let mut iter = db.new_iter()?;
    let mut index: HashMap<Vec<u8>, BlockInfo> = HashMap::new();
    let mut height_to_hashes: HashMap<i64, Vec<Vec<u8>>> = HashMap::new();
    let mut parse_errors = 0;
    let mut leveldb_count = 0;
    
    // Statistics for offset data
    let mut blocks_with_data_flag = 0;
    let mut blocks_with_file_and_pos = 0;
    
    // Step 1: Parse all blocks from leveldb
    while let Some((key, value)) = LdbIterator::next(&mut iter) {
        if key.len() != 33 || key[0] != b'b' {
            continue;
        }
        
        let block_hash = key[1..].to_vec();
        let mut offset = 0;
        
        // Parse CDiskBlockIndex (PIVX chain.h)
        
        // nSerVersion (NONNEGATIVE_SIGNED)
        read_varint_signed(&value, &mut offset);
        
        // nHeight (regular VARINT, not signed!)
        let height = match read_varint(&value, &mut offset) {
            Some(h) => h as i64,
            None => {
                parse_errors += 1;
                continue;
            }
        };
        
        // nStatus
        let status = match read_varint(&value, &mut offset) {
            Some(s) => s,
            None => {
                parse_errors += 1;
                continue;
            }
        };
        
        // nTx
        read_varint(&value, &mut offset);
        
        const BLOCK_HAVE_DATA: u64 = 8;
        const BLOCK_HAVE_UNDO: u64 = 16;
        
        // Conditional fields based on nStatus
        // Capture file number and data position when present
        let mut n_file: Option<u64> = None;
        let mut n_data_pos: Option<u64> = None;
        
        let has_data = (status & BLOCK_HAVE_DATA) != 0;
        let has_undo = (status & BLOCK_HAVE_UNDO) != 0;
        
        if has_data {
            blocks_with_data_flag += 1;
        }
        
        // Debug: log first few blocks to verify parsing
        if leveldb_count < 3 {
            println!("  Block #{}: height={}, nStatus=0x{:x} (HAVE_DATA={}, HAVE_UNDO={})", 
                     leveldb_count, height, status, has_data, has_undo);
        }
        
        if (status & (BLOCK_HAVE_DATA | BLOCK_HAVE_UNDO)) != 0 {
            if let Some(f) = read_varint_signed(&value, &mut offset) {
                if f >= 0 {
                    n_file = Some(f as u64);
                    if leveldb_count < 3 {
                        println!("    → nFile={}", f);
                    }
                }
            }
        }

        if (status & BLOCK_HAVE_DATA) != 0 {
            if let Some(pos) = read_varint(&value, &mut offset) {
                n_data_pos = Some(pos);
                if leveldb_count < 3 {
                    println!("    → nDataPos={}", pos);
                }
            }
        }
        
        // Track statistics
        if n_file.is_some() && n_data_pos.is_some() {
            blocks_with_file_and_pos += 1;
        }
        
        if (status & BLOCK_HAVE_UNDO) != 0 {
            read_varint(&value, &mut offset); // nUndoPos
        }
        
        // Block header data
        
        // nFlags (4 bytes)
        if offset + 4 > value.len() {
            parse_errors += 1;
            continue;
        }
        offset += 4;
        
        // nVersion (4 bytes)
        if offset + 4 > value.len() {
            parse_errors += 1;
            continue;
        }
        offset += 4;
        
        // vStakeModifier (VECTOR!)
        if read_vector_bytes(&value, &mut offset).is_none() {
            parse_errors += 1;
            continue;
        }
        
        // hashPrev (32 bytes) - internal (little-endian) format
        if offset + 32 > value.len() {
            parse_errors += 1;
            continue;
        }
        let hash_prev = value[offset..offset + 32].to_vec();
        offset += 32;
        
        // hashMerkleRoot (32 bytes) - skip
        if offset + 32 > value.len() {
            parse_errors += 1;
            continue;
        }
        offset += 32;
        
        // nTime (4 bytes) - skip
        if offset + 4 > value.len() {
            parse_errors += 1;
            continue;
        }
        offset += 4;
        
        // nBits (4 bytes) - NEEDED FOR CHAINWORK!
        if offset + 4 > value.len() {
            parse_errors += 1;
            continue;
        }
        let n_bits = u32::from_le_bytes([
            value[offset], value[offset+1], value[offset+2], value[offset+3]
        ]);
        
        let info = BlockInfo {
            height,
            hash_prev,
            n_bits,
            chainwork: None,
            file: n_file,
            data_pos: n_data_pos,
        };
        
        index.insert(block_hash.clone(), info);
        height_to_hashes.entry(height).or_default().push(block_hash);
        leveldb_count += 1;
    }
    
    let max_height = *height_to_hashes.keys().max().unwrap_or(&0);
    
    println!("✅ Parsed {} blocks from leveldb", leveldb_count);
    println!("  Parse errors: {}", parse_errors);
    println!("  Height range: 0 to {}", max_height);
    println!("  Blocks with BLOCK_HAVE_DATA flag: {} ({:.2}%)", 
             blocks_with_data_flag,
             (blocks_with_data_flag as f64 / leveldb_count as f64) * 100.0);
    println!("  Blocks with file AND position: {} ({:.2}%)", 
             blocks_with_file_and_pos,
             (blocks_with_file_and_pos as f64 / leveldb_count as f64) * 100.0);
    
    if blocks_with_file_and_pos < leveldb_count / 2 {
        println!("\n⚠️  WARNING: Less than 50% of blocks have offset data!");
        println!("   This will limit Pattern A offset-based reading.");
        println!("   Consider running: pivxd -reindex");
    }
    
    // Step 2: Calculate chainwork for all blocks
    println!("⚡ Calculating chainwork for all blocks...");
    
    // PIVX genesis hash (internal/little-endian byte order)
    let mut genesis_hash = hex::decode("0000041e482b9b9691d98eefb48473405c0b8ec31b76df3797c74a78680ef818")
        .expect("Invalid genesis");
    genesis_hash.reverse();
    
    // Set genesis chainwork
    if let Some(genesis_info) = index.get_mut(&genesis_hash) {
        genesis_info.chainwork = Some(get_block_proof(genesis_info.n_bits));
    }
    
    // Calculate chainwork for all blocks by height (BFS from genesis)
    let mut processed = 0;
    for height in 1..=max_height {
        if let Some(block_hashes) = height_to_hashes.get(&height) {
            for block_hash in block_hashes {
                if let Some(block_info) = index.get(block_hash) {
                    let parent_hash = block_info.hash_prev.clone();
                    let n_bits = block_info.n_bits;
                    
                    if let Some(parent_info) = index.get(&parent_hash) {
                        if let Some(parent_chainwork) = parent_info.chainwork {
                            // chainwork = parent.chainwork + GetBlockProof(nBits)
                            let mut chainwork = parent_chainwork;
                            let proof = get_block_proof(n_bits);
                            add_u256(&mut chainwork, &proof);
                            
                            if let Some(block_info_mut) = index.get_mut(block_hash) {
                                block_info_mut.chainwork = Some(chainwork);
                            }
                        }
                    }
                }
                processed += 1;
            }
        }
        
        if height % 500_000 == 0 {
            println!("  Calculated chainwork for blocks up to height {}", height);
        }
    }
    
    println!("✅ Chainwork calculated for {} blocks", processed);
    
    // Step 3: Find tip with highest chainwork
    println!("🏆 Finding best chain tip (highest chainwork)...");
    
    let mut best_tip: Option<(Vec<u8>, i64, [u64; 4])> = None;
    let mut blocks_without_chainwork = 0;
    
    // Check all blocks at max height and nearby (in case of orphans)
    for height in (max_height.saturating_sub(100))..=max_height {
        if let Some(block_hashes) = height_to_hashes.get(&height) {
            for block_hash in block_hashes {
                if let Some(block_info) = index.get(block_hash) {
                    if let Some(chainwork) = block_info.chainwork {
                        match &best_tip {
                            None => {
                                best_tip = Some((block_hash.clone(), block_info.height, chainwork));
                            }
                            Some((_, _, best_chainwork)) => {
                                if cmp_u256(&chainwork, best_chainwork) == std::cmp::Ordering::Greater {
                                    best_tip = Some((block_hash.clone(), block_info.height, chainwork));
                                }
                            }
                        }
                    } else {
                        blocks_without_chainwork += 1;
                    }
                }
            }
        }
    }
    
    if blocks_without_chainwork > 0 {
        println!("⚠️  {} blocks at tip heights missing chainwork", blocks_without_chainwork);
    }
    
    // Fallback: if no tip found with chainwork, just use max height
    let (best_tip_hash, best_tip_height, _) = if let Some(tip) = best_tip {
        tip
    } else {
        println!("⚠️  No blocks found with valid chainwork, using max height as fallback");
        if let Some(hashes) = height_to_hashes.get(&max_height) {
            if let Some(hash) = hashes.first() {
                (hash.clone(), max_height, [0, 0, 0, 0])
            } else {
                return Err("No blocks found at max height".into());
            }
        } else {
            return Err("No blocks found at max height".into());
        }
    };
    
    println!("✅ Best tip: height {}, hash {}", best_tip_height, hex::encode(&best_tip_hash));
    
    // Step 4: Build canonical chain by walking BACKWARDS from best tip
    println!("⬇️  Building canonical chain (walking backwards from tip)...");
    
    let mut chain: Vec<(i64, Vec<u8>, Option<u64>, Option<u64>)> = Vec::new();
    let mut current_hash = best_tip_hash;
    let mut steps = 0;
    
    loop {
        let block_info = match index.get(&current_hash) {
            Some(info) => info,
            None => {
                println!("  ⚠️  Block not found - stopping at height {}", chain.last().map(|(h, _, _, _)| *h).unwrap_or(-1));
                break;
            }
        };
        
        if steps < 20 {
            let hash_hex: String = current_hash.iter().rev().map(|b| format!("{:02x}", b)).collect();
            println!("  Step {}: height={}, hash={}", steps, block_info.height, &hash_hex[..16]);
        }
        steps += 1;
        
        chain.push((block_info.height, current_hash.clone(), block_info.file, block_info.data_pos));
        
        // Check if we reached genesis
        if block_info.height == 0 {
            break;
        }
        
        // Move to parent
        current_hash = block_info.hash_prev.clone();
        
        if chain.len() % 500_000 == 0 {
            println!("  Progress: {} blocks", chain.len());
        }
    }
    
    // Reverse to get genesis -> tip order
    chain.reverse();

    println!("✅ Built canonical chain: {} blocks (height 0 to {})", 
             chain.len(), 
             chain.last().map(|(h, _, _, _)| *h).unwrap_or(0));

    Ok(chain)
}
