/// Pattern A Block Indexer - Direct offset-based block reading
///
/// This module implements the authoritative Pattern A approach:
/// 1. Read canonical chain from PIVX Core's block index (LevelDB)
/// 2. For each canonical block, read directly from blkNNNNN.dat at the exact offset
/// 3. Parse and index the block using the same transaction processing logic
///
/// This runs in parallel with the old scanner initially for validation.
/// Once validated, the old scanner can be removed.

use std::sync::Arc;
use std::path::PathBuf;
use rocksdb::DB;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader, SeekFrom};
use crate::types::AppState;
use crate::batch_writer::BatchWriter;
use crate::transactions::process_transaction;
use crate::config::get_global_config;

const PREFIX: [u8; 4] = [0x90, 0xc4, 0xfd, 0xe9]; // PIVX network magic
const TX_BATCH_SIZE: usize = 10000;

/// Read and index a single canonical block using file number + offset from LevelDB
async fn index_block_by_offset(
    blk_dir: &PathBuf,
    file_num: u32,
    data_pos: u64,
    block_hash: &[u8], // internal format
    height: i32,
    db: Arc<DB>,
    state: AppState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Construct blk file path
    let filename = format!("blk{:05}.dat", file_num);
    let mut path = blk_dir.clone();
    path.push(filename);

    // Open file and seek to offset
    let file = File::open(&path).await
        .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(data_pos)).await?;

    // Read and validate magic bytes
    let mut magic_bytes = [0u8; 4];
    reader.read_exact(&mut magic_bytes).await?;
    if magic_bytes != PREFIX {
        return Err(format!("Invalid magic at {}:{} - got {:02x?}", 
            path.display(), data_pos, magic_bytes).into());
    }

    // Read block size
    let mut size_buf = [0u8; 4];
    reader.read_exact(&mut size_buf).await?;
    let block_size = u32::from_le_bytes(size_buf) as u64;

    // Read header version to determine header size
    let mut version_bytes = [0u8; 4];
    reader.read_exact(&mut version_bytes).await?;
    let block_version = u32::from_le_bytes(version_bytes);
    
    let header_size = get_header_size(block_version);

    // Read full header (including the version we already read)
    let mut header_buffer = vec![0u8; header_size];
    header_buffer[0..4].copy_from_slice(&version_bytes);
    reader.read_exact(&mut header_buffer[4..]).await?;

    // Parse header to get block hash and verify it matches expected
    let parsed_header = parse_block_header_sync(&header_buffer, header_size)?;
    
    // Verify block hash matches what we expect from LevelDB
    if &parsed_header.block_hash != block_hash {
        return Err(format!("Block hash mismatch at {}:{} - expected {}, got {}", 
            path.display(), data_pos,
            hex::encode(block_hash),
            hex::encode(&parsed_header.block_hash)).into());
    }

    // Store block header
    let cf_blocks = db.cf_handle("blocks")
        .ok_or("blocks CF not found")?;
    let mut block_key = vec![b'b'];
    block_key.extend_from_slice(block_hash);
    db.put_cf(&cf_blocks, &block_key, &header_buffer)?;

    // Store height mappings in chain_metadata
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    // height -> display_hash (reversed)
    let height_bytes = height.to_le_bytes();
    let mut display_hash = block_hash.to_vec();
    display_hash.reverse();
    db.put_cf(&cf_metadata, height_bytes, &display_hash)?;
    
    // 'h' + internal_hash -> height
    let mut h_key = vec![b'h'];
    h_key.extend_from_slice(block_hash);
    db.put_cf(&cf_metadata, &h_key, height_bytes)?;

    // Process transactions
    let fast_sync = get_global_config().get_bool("sync.fast_sync").unwrap_or(false);
    let mut tx_batch = BatchWriter::new(db.clone(), TX_BATCH_SIZE);

    // Process transactions using existing transaction parser
    match process_transaction(
        &mut reader,
        block_version,
        block_hash,
        Some(height),
        db.clone(),
        &mut tx_batch,
        fast_sync,
    ).await {
        Ok(_) => {
            // Flush remaining transactions
            tx_batch.flush().await?;
        }
        Err(e) => {
            eprintln!("⚠️  Error processing transactions for block {} at height {}: {}", 
                hex::encode(block_hash), height, e);
            // Flush what we have and continue
            tx_batch.flush().await?;
        }
    }

    Ok(())
}

// Helper to determine header size based on block version
fn get_header_size(version: u32) -> usize {
    match version {
        1..=3 => 80,
        4..=6 => 112,  // May vary, but we handle dynamic detection in main code
        7..=10 => 112,
        11.. => 112,
        _ => 80,
    }
}

// Helper to parse block header (sync version)
fn parse_block_header_sync(
    header_bytes: &[u8],
    _header_size: usize,
) -> Result<ParsedBlockHeader, Box<dyn std::error::Error + Send + Sync>> {
    use sha2::{Sha256, Digest};
    
    // Hash the header to get block hash
    let first_hash = Sha256::digest(header_bytes);
    let second_hash = Sha256::digest(&first_hash);
    let block_hash: Vec<u8> = second_hash.to_vec(); // Internal format (little-endian)
    
    // Extract hash_prev_block (bytes 4-36, internal format)
    let mut hash_prev_block = [0u8; 32];
    if header_bytes.len() >= 36 {
        hash_prev_block.copy_from_slice(&header_bytes[4..36]);
    }
    
    Ok(ParsedBlockHeader {
        block_hash,
        hash_prev_block,
    })
}

struct ParsedBlockHeader {
    block_hash: Vec<u8>,
    hash_prev_block: [u8; 32],
}

/// Run offset-based indexing for all canonical blocks
///
/// This function:
/// 1. Reads the canonical chain from chain_metadata (already stored by sync.rs)
/// 2. For each block, reads the 'o' mapping (file + offset)
/// 3. Indexes the block by reading directly from blkNNNNN.dat at that offset
/// 4. Runs in parallel with configurable concurrency
pub async fn index_canonical_blocks_by_offset(
    blk_dir: PathBuf,
    db: Arc<DB>,
    state: AppState,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║    PATTERN A: OFFSET-BASED BLOCK INDEXING         ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    // Find highest height in canonical chain
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    let sync_height = match db.get_cf(&cf_state, b"sync_height")? {
        Some(bytes) => i32::from_le_bytes(bytes.as_slice().try_into()?),
        None => {
            println!("⚠️  No sync_height found - canonical chain not loaded yet");
            return Ok(());
        }
    };
    
    println!("📊 Canonical chain height: {}", sync_height);
    println!("📁 Block directory: {}", blk_dir.display());
    
    // Count how many blocks have offset mappings
    let mut blocks_with_offsets = 0;
    let mut blocks_without_offsets = 0;
    
    for height in 0..=sync_height {
        let height_bytes = height.to_le_bytes();
        
        // Get block hash for this height
        let display_hash = match db.get_cf(&cf_metadata, height_bytes)? {
            Some(h) => h,
            None => {
                eprintln!("⚠️  No block hash for height {}", height);
                continue;
            }
        };
        
        // Convert to internal format
        let mut internal_hash = display_hash.clone();
        internal_hash.reverse();
        
        // Check if we have offset mapping
        let mut o_key = vec![b'o'];
        o_key.extend_from_slice(&internal_hash);
        
        if db.get_cf(&cf_metadata, &o_key)?.is_some() {
            blocks_with_offsets += 1;
        } else {
            blocks_without_offsets += 1;
        }
    }
    
    println!("📊 Offset mapping coverage:");
    println!("   {} blocks WITH offsets (can use Pattern A)", blocks_with_offsets);
    println!("   {} blocks WITHOUT offsets (need fallback)", blocks_without_offsets);
    
    if blocks_with_offsets == 0 {
        println!("\n⚠️  No offset mappings found!");
        println!("   Run sync again to generate offset mappings from LevelDB.");
        return Ok(());
    }
    
    // Process blocks with offsets
    println!("\n🚀 Starting offset-based indexing for {} blocks...\n", blocks_with_offsets);
    
    let config = get_global_config();
    let max_concurrent = config.get_int("sync.parallel_files").unwrap_or(4) as usize;
    
    use futures::stream::{self, StreamExt};
    
    let mut tasks = Vec::new();
    for height in 0..=sync_height {
        let height_bytes = height.to_le_bytes();
        
        // Get block hash
        let display_hash = match db.get_cf(&cf_metadata, height_bytes)? {
            Some(h) => h,
            None => continue,
        };
        
        let mut internal_hash = display_hash.clone();
        internal_hash.reverse();
        
        // Get offset mapping
        let mut o_key = vec![b'o'];
        o_key.extend_from_slice(&internal_hash);
        
        let offset_data = match db.get_cf(&cf_metadata, &o_key)? {
            Some(d) => d,
            None => continue, // Skip blocks without offsets
        };
        
        if offset_data.len() != 12 {
            eprintln!("⚠️  Invalid offset data for height {}", height);
            continue;
        }
        
        // Parse file number and data position
        let file_num = u32::from_le_bytes([
            offset_data[0], offset_data[1], offset_data[2], offset_data[3]
        ]);
        let data_pos = u64::from_le_bytes([
            offset_data[4], offset_data[5], offset_data[6], offset_data[7],
            offset_data[8], offset_data[9], offset_data[10], offset_data[11]
        ]);
        
        tasks.push((height, internal_hash, file_num, data_pos));
    }
    
    println!("📦 Processing {} blocks with {} workers...\n", tasks.len(), max_concurrent);
    
    let total_tasks = tasks.len();
    let processed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    
    // Process in parallel
    stream::iter(tasks)
        .map(|(height, internal_hash, file_num, data_pos)| {
            let blk_dir = blk_dir.clone();
            let db = db.clone();
            let state = state.clone();
            let processed = processed.clone();
            
            async move {
                let result = index_block_by_offset(
                    &blk_dir,
                    file_num,
                    data_pos,
                    &internal_hash,
                    height,
                    db,
                    state,
                ).await;
                
                let count = processed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                
                if count % 10000 == 0 {
                    println!("  ✅ Processed {}/{} blocks ({:.1}%)", 
                        count, total_tasks, (count as f64 / total_tasks as f64) * 100.0);
                }
                
                if let Err(e) = result {
                    eprintln!("❌ Failed to index block at height {}: {}", height, e);
                }
            }
        })
        .buffer_unordered(max_concurrent)
        .collect::<Vec<_>>()
        .await;
    
    let final_count = processed.load(std::sync::atomic::Ordering::Relaxed);
    
    println!("\n✅ Pattern A indexing complete!");
    println!("   Successfully processed {} blocks", final_count);
    
    Ok(())
}
