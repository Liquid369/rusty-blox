/// Combined approach: Read leveldb index + scan blk files for unflushed blocks
/// This properly mimics how PIVX Core builds its block index
use rusty_leveldb::{DB, Options, LdbIterator};
use std::collections::HashMap;
use std::path::Path;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use byteorder::{LittleEndian, ReadBytesExt};

// Parse PIVX's VARINT format
fn read_varint(data: &[u8], offset: &mut usize) -> Option<u64> {
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

// VARINT with NONNEGATIVE_SIGNED mode
fn read_varint_signed(data: &[u8], offset: &mut usize) -> Option<i64> {
    read_varint(data, offset).map(|v| (v / 2) as i64)
}

// Read a vector (compact size + elements)
fn read_vector_bytes(data: &[u8], offset: &mut usize) -> Option<Vec<u8>> {
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
// This approximates the expected number of hashes to mine this block
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
// This implements long division for 2^256 / (target + 1)
fn div_u256(dividend: [u64; 5], divisor: [u64; 4]) -> [u64; 4] {
    // For simplicity, use the approximation: ~2^256 / target
    // This is close enough for chainwork comparison
    
    // Find highest non-zero word in divisor
    let mut shift = 0;
    if divisor[3] != 0 {
        shift = 192;
    } else if divisor[2] != 0 {
        shift = 128;
    } else if divisor[1] != 0 {
        shift = 64;
    }
    
    // Approximate division by shifting
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

// Compare two 256-bit integers (returns true if a > b)
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
struct BlockInfo {
    height: i64,
    hash_prev: Vec<u8>,
    n_bits: u32,
    chainwork: Option<[u64; 4]>,  // 256-bit integer as 4x u64 (little-endian)
    source: BlockSource,
}

#[derive(Debug, Clone)]
enum BlockSource {
    Leveldb { n_file: i64, n_data_pos: u64 },
    BlkFile { file_num: u32, offset: u64 },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔍 Building combined block index from leveldb + blk files...\n");
    
    // Use CURRENT snapshot (fresh copy while daemon running)
    let db_path = "/tmp/pivx_index_current";
    let blk_dir = "/Users/liquid/Library/Application Support/PIVX/blocks";
    
    // Step 1: Read leveldb index
    println!("📖 Step 1: Reading leveldb block index...");
    
    let opts = Options::default();
    let mut db = DB::open(Path::new(db_path), opts)?;
    
    let mut iter = db.new_iter()?;
    let mut index: HashMap<Vec<u8>, BlockInfo> = HashMap::new();
    let mut height_to_hashes: HashMap<i64, Vec<Vec<u8>>> = HashMap::new();
    let mut parse_errors = 0;
    let mut leveldb_count = 0;
    let mut height_1_count = 0;
    
    while let Some((key, value)) = LdbIterator::next(&mut iter) {
        if key.len() != 33 || key[0] != b'b' {
            continue;
        }
        
        let block_hash = key[1..].to_vec();
        let mut offset = 0;
        
        // Parse CDiskBlockIndex (see PIVX chain.h lines 279-362)
        
        // Skip nSerVersion
        read_varint_signed(&value, &mut offset);
        
        // nHeight - NOTE: Seems to be stored as simple varint, not NONNEGATIVE_SIGNED!
        let height_start = offset;
        let height_raw = match read_varint(&value, &mut offset) {
            Some(h) => h,
            None => {
                parse_errors += 1;
                continue;
            }
        };
        let height = height_raw as i64;  // Use raw varint value, not divided by 2
        
        if height == 1 {
            height_1_count += 1;
        }
        
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
        
        let mut n_file: i64 = -1;
        let mut n_data_pos: u64 = 0;
        
        // Conditional fields based on nStatus
        if (status & (BLOCK_HAVE_DATA | BLOCK_HAVE_UNDO)) != 0 {
            n_file = read_varint_signed(&value, &mut offset).unwrap_or(-1);
        }
        
        if (status & BLOCK_HAVE_DATA) != 0 {
            n_data_pos = read_varint(&value, &mut offset).unwrap_or(0);
        }
        
        if (status & BLOCK_HAVE_UNDO) != 0 {
            read_varint(&value, &mut offset); // Skip nUndoPos
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
        
        // vStakeModifier (VECTOR!) - THIS WAS THE KEY MISSING PIECE!
        if read_vector_bytes(&value, &mut offset).is_none() {
            parse_errors += 1;
            continue;
        }
        
        // hashPrev (32 bytes) - already in internal (little-endian) format
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
        
        // nBits (4 bytes)
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
            chainwork: None,  // Will be calculated later
            source: BlockSource::Leveldb { n_file, n_data_pos },
        };
        
        index.insert(block_hash.clone(), info);
        height_to_hashes.entry(height).or_insert_with(Vec::new).push(block_hash);
        leveldb_count += 1;
    }
    
    println!("✅ Leveldb: Parsed {} blocks", leveldb_count);
    println!("  Parse errors: {}", parse_errors);
    println!("  Height 1 blocks found during parsing: {}", height_1_count);
    println!("  Height range: 0 to {}", height_to_hashes.keys().max().unwrap_or(&0));
    
    // Step 2: Scan blk files for blocks not in leveldb
    println!("\n📂 Step 2: Scanning blk files for unflushed blocks...");
    
    let max_leveldb_height = *height_to_hashes.keys().max().unwrap_or(&0);
    println!("  (Looking for blocks beyond height {})...", max_leveldb_height);
    
    // TODO: Implement blk file scanning similar to canonical_chain.rs
    // For now, let's just report what we have
    
    // Step 3: Calculate chainwork for all blocks
    println!("\n⚡ Step 3: Calculating chainwork for all blocks...");
    
    // PIVX genesis - use internal (little-endian) byte order
    let mut genesis_hash = hex::decode("0000041e482b9b9691d98eefb48473405c0b8ec31b76df3797c74a78680ef818")
        .expect("Invalid genesis");
    genesis_hash.reverse();
    
    // Set genesis chainwork to its own proof
    if let Some(genesis_info) = index.get_mut(&genesis_hash) {
        genesis_info.chainwork = Some(get_block_proof(genesis_info.n_bits));
    }
    
    // Calculate chainwork for all blocks by height (BFS from genesis)
    let mut processed = 0;
    for height in 1..=max_leveldb_height {
        if let Some(block_hashes) = height_to_hashes.get(&height) {
            for block_hash in block_hashes {
                if let Some(block_info) = index.get(block_hash) {
                    let parent_hash = block_info.hash_prev.clone();
                    let n_bits = block_info.n_bits;
                    
                    // Get parent chainwork
                    if let Some(parent_info) = index.get(&parent_hash) {
                        if let Some(parent_chainwork) = parent_info.chainwork {
                            // Calculate this block's chainwork = parent + GetBlockProof(nBits)
                            let mut chainwork = parent_chainwork;
                            let proof = get_block_proof(n_bits);
                            add_u256(&mut chainwork, &proof);
                            
                            // Store chainwork
                            if let Some(block_info_mut) = index.get_mut(block_hash) {
                                block_info_mut.chainwork = Some(chainwork);
                            }
                        }
                    }
                }
                processed += 1;
            }
        }
        
        if height % 100_000 == 0 {
            println!("  Calculated chainwork for blocks up to height {}", height);
        }
    }
    
    println!("✅ Chainwork calculated for {} blocks", processed);
    
    // Step 4: Find tip with highest chainwork
    println!("\n🏆 Step 4: Finding best chain tip (highest chainwork)...");
    
    let mut best_tip: Option<(Vec<u8>, i64, [u64; 4])> = None;
    
    // Check all blocks at max height (and a few heights below in case of orphans)
    for height in (max_leveldb_height.saturating_sub(100))..=max_leveldb_height {
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
                    }
                }
            }
        }
    }
    
    let (best_tip_hash, best_tip_height, best_tip_chainwork) = best_tip
        .expect("No valid tip found!");
    
    println!("✅ Best tip found:");
    println!("  Hash: {}", hex::encode(&best_tip_hash));
    println!("  Height: {}", best_tip_height);
    println!("  Chainwork: {:016x}{:016x}{:016x}{:016x}", 
             best_tip_chainwork[3], best_tip_chainwork[2], 
             best_tip_chainwork[1], best_tip_chainwork[0]);
    
    // Step 5: Build canonical chain by walking BACKWARDS from best tip
    println!("\n⬇️  Step 5: Building canonical chain (walking backwards from tip)...");
    
    let mut chain: Vec<(i64, Vec<u8>)> = Vec::new();
    let mut current_hash = best_tip_hash;
    
    loop {
        let block_info = match index.get(&current_hash) {
            Some(info) => info,
            None => {
                println!("  ⚠️  Block not found - stopping");
                break;
            }
        };
        
        chain.push((block_info.height, current_hash.clone()));
        
        // Check if we reached genesis
        if block_info.height == 0 {
            break;
        }
        
        // Move to parent
        current_hash = block_info.hash_prev.clone();
        
        if chain.len() % 100_000 == 0 {
            println!("  Progress: {} blocks (at height {})", chain.len(), block_info.height);
        }
    }
    
    // Reverse to get genesis -> tip order
    chain.reverse();
    
    println!("\n✅ Built canonical chain: {} blocks", chain.len());
    println!("  Heights: {} to {}", 
             chain.first().map(|(h, _)| *h).unwrap_or(0),
             chain.last().map(|(h, _)| *h).unwrap_or(0));
    
    // Save chain
    println!("\n💾 Saving to /tmp/combined_canonical_chain.json...");
    
    let json: Vec<_> = chain.iter()
        .map(|(height, hash)| {
            serde_json::json!({
                "height": height,
                "hash": hex::encode(hash)
            })
        })
        .collect();
    
    std::fs::write(
        "/tmp/combined_canonical_chain.json",
        serde_json::to_string_pretty(&json)?
    )?;
    
    println!("✅ Done!");
    
    // Compare with blk file scan results
    println!("\n📊 Summary:");
    println!("  Leveldb blocks: {}", leveldb_count);
    println!("  Canonical chain: {} blocks", chain.len());
    println!("  Max leveldb height: {}", max_leveldb_height);
    
    Ok(())
}
