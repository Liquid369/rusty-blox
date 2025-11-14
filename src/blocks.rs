use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader};
use rocksdb::DB;
use crate::types::{CBlockHeader, AppState, MyError, CTransaction, CTxIn, CTxOut, COutPoint, CScript};
use rustyblox::call_quark_hash;
use rustyblox::chainwork::calculate_work_from_bits;
use sha2::{Sha256, Digest};
use crate::db_utils::batch_put_cf;
use crate::transactions::process_transaction;
use crate::batch_writer::BatchWriter;
use crate::config::get_global_config;

const PREFIX: [u8; 4] = [0x90, 0xc4, 0xfd, 0xe9]; // PIVX network prefix
const BATCH_SIZE: usize = 1000; // Increased from 100 for better throughput
const TX_BATCH_SIZE: usize = 10000; // Increased from 1000 for better throughput

// Helper to read varint from async reader
async fn read_varint<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<u64, std::io::Error> {
    let first = reader.read_u8().await?;
    let value = match first {
        0x00..=0xfc => u64::from(first),
        0xfd => {
            let mut buf = [0u8; 2];
            reader.read_exact(&mut buf).await?;
            u64::from(u16::from_le_bytes(buf))
        }
        0xfe => {
            let mut buf = [0u8; 4];
            reader.read_exact(&mut buf).await?;
            u64::from(u32::from_le_bytes(buf))
        }
        0xff => {
            let mut buf = [0u8; 8];
            reader.read_exact(&mut buf).await?;
            u64::from_le_bytes(buf)
        }
    };
    Ok(value)
}

// Helper to read script
async fn read_script<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Vec<u8>, std::io::Error> {
    let script_length = read_varint(reader).await?;
    let mut script = vec![0u8; script_length as usize];
    reader.read_exact(&mut script).await?;
    Ok(script)
}

// Helper to read outpoint
async fn read_outpoint<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<COutPoint, std::io::Error> {
    let mut hash = [0u8; 32];
    reader.read_exact(&mut hash).await?;
    
    let mut n_buf = [0u8; 4];
    reader.read_exact(&mut n_buf).await?;
    let n = u32::from_le_bytes(n_buf);
    
    // Reverse hash for display
    let reversed_hash: Vec<u8> = hash.iter().rev().cloned().collect();
    let hex_hash = hex::encode(&reversed_hash);
    
    Ok(COutPoint { hash: hex_hash, n })
}

// Parse transaction inputs
async fn read_tx_inputs<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    block_version: u32,
    tx_version: i16,
) -> Result<Vec<CTxIn>, std::io::Error> {
    let input_count = read_varint(reader).await?;
    let mut inputs = Vec::new();
    
    for i in 0..input_count {
        let mut coinbase = None;
        let mut prevout = None;
        let mut script = Vec::new();
        
        // Check if this is a coinbase transaction
        if block_version < 3 && tx_version == 2 {
            // Coinbase transaction
            let mut buffer = [0u8; 26];
            reader.read_exact(&mut buffer).await?;
            coinbase = Some(buffer.to_vec());
        } else {
            // Regular transaction
            prevout = Some(read_outpoint(reader).await?);
            script = read_script(reader).await?;
        }
        
        let mut seq_buf = [0u8; 4];
        reader.read_exact(&mut seq_buf).await?;
        let sequence = u32::from_le_bytes(seq_buf);
        
        inputs.push(CTxIn {
            prevout,
            script_sig: CScript { script },
            sequence,
            index: i,
            coinbase,
        });
    }
    
    Ok(inputs)
}

// Parse transaction outputs
async fn read_tx_outputs<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Vec<CTxOut>, std::io::Error> {
    let output_count = read_varint(reader).await?;
    let mut outputs = Vec::new();
    
    for i in 0..output_count {
        let mut value_buf = [0u8; 8];
        reader.read_exact(&mut value_buf).await?;
        let value = i64::from_le_bytes(value_buf);
        
        let script = read_script(reader).await?;
        let script_length = script.len() as i32;
        
        outputs.push(CTxOut {
            value,
            script_length,
            script_pubkey: CScript { script },
            index: i,
            address: Vec::new(), // Will be populated later with address extraction
        });
    }
    
    Ok(outputs)
}

pub async fn process_blk_file(_state: AppState, file_path: impl AsRef<std::path::Path>, db: Arc<DB>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let file_path_ref = file_path.as_ref();
    println!("Processing file: {}", file_path_ref.display());
    
    // Get fast_sync setting from config
    let config = get_global_config();
    let fast_sync = config.get_bool("sync.fast_sync").unwrap_or(false);
    if fast_sync {
        println!("  Fast sync mode enabled (skipping UTXO tracking)");
    }

    let file = tokio::fs::File::open(&file_path_ref).await?;
    let mut reader = BufReader::new(file);

    let mut batch_items = Vec::new();
    let mut header_buffer = Vec::with_capacity(112);
    let mut block_count = 0;
    
    // Create batch writer for transaction data
    let mut tx_batch = BatchWriter::new(db.clone(), TX_BATCH_SIZE);
    
    let mut size_buffer = [0u8; 4];
    let mut magic_bytes = [0u8; 4];

    loop {
        // Track position at start of block (after magic+size)
        let block_start_pos = reader.stream_position().await?;
        
        // Read magic bytes
        match reader.read_exact(&mut magic_bytes).await {
            Ok(_) => {},
            Err(e) => {
                if block_count > 0 {
                    println!("  EOF reached after {} blocks: {}", block_count, e);
                }
                break;
            }
        }

        if magic_bytes != PREFIX {
            eprintln!("  Invalid magic at block {}: {:02x?}, expected {:02x?}", 
                     block_count, magic_bytes, PREFIX);
            break;  // Invalid block, end of file
        }

        // Read block size
        match reader.read_exact(&mut size_buffer).await {
            Ok(_) => {},
            Err(e) => {
                eprintln!("  Failed to read size at block {}: {}", block_count, e);
                break;
            }
        }
        let block_size = u32::from_le_bytes(size_buffer) as u64;
        
        // Calculate EXACT position where next block should start
        // next_block_pos = current_pos (after magic+size) + block_size
        let next_block_pos = block_start_pos + 8 + block_size;

        // Peek at version to determine header size (4 bytes)
        let mut version_bytes = [0u8; 4];
        match reader.read_exact(&mut version_bytes).await {
            Ok(_) => {},
            Err(e) => {
                eprintln!("  Failed to read version at block {}: {}", block_count, e);
                break;
            }
        }
        let ver_as_int = u32::from_le_bytes(version_bytes);
        
        // For version 4-6, we need to DETECT the actual header size dynamically
        // Some v4 blocks have 80 bytes, some have 112 bytes
        let header_size = if ver_as_int >= 4 && ver_as_int <= 6 {
            // Save current position
            let current_pos = reader.stream_position().await?;
            
            // Skip to byte 80 (we've read 4 bytes for version, need to skip 76 more)
            let mut skip_buffer = vec![0u8; 76];
            let detected_size = match reader.read_exact(&mut skip_buffer).await {
                Ok(_) => {
                    // Now check the next 32 bytes
                    let mut potential_accumulator = [0u8; 32];
                    match reader.read_exact(&mut potential_accumulator).await {
                        Ok(_) => {
                            // Improved heuristic to differentiate accumulator checkpoint from transaction varint
                            // Transaction count varint patterns:
                            // - 1 tx: 0x01 followed by tx data (coinbase usually)
                            // - 2 txs: 0x02 followed by tx data
                            // - N txs (N < 253): 0xNN followed by tx data
                            // - For varint, next bytes would be tx version (0x01000000 or 0x02000000)
                            
                            let first_byte = potential_accumulator[0];
                            
                            // Check if it looks like a varint followed by transaction version
                            let looks_like_tx_count = if first_byte < 0xfd {
                                // Next 4 bytes should be tx version (little endian)
                                // Common tx versions: 1, 2, 3, 4
                                let potential_tx_version = u32::from_le_bytes([
                                    potential_accumulator[1],
                                    potential_accumulator[2],
                                    potential_accumulator[3],
                                    potential_accumulator[4]
                                ]);
                                // Tx version is typically 1-4, definitely < 100
                                potential_tx_version > 0 && potential_tx_version < 100
                            } else {
                                false
                            };
                            
                            // Accumulator checkpoint is a 32-byte hash
                            // Could be all zeros or a valid hash (mixed bytes)
                            let all_zeros = potential_accumulator.iter().all(|&b| b == 0);
                            
                            if looks_like_tx_count {
                                80  // It's transaction data, header is 80 bytes
                            } else if all_zeros || first_byte >= 0xfd {
                                112 // It's accumulator checkpoint, header is 112 bytes
                            } else {
                                // Fallback: check byte distribution
                                // Real hashes have mixed bytes, tx data has patterns
                                let unique_bytes = potential_accumulator.iter().collect::<std::collections::HashSet<_>>().len();
                                if unique_bytes > 10 {
                                    112 // Looks like hash data
                                } else {
                                    80  // Looks like structured data (tx count + version)
                                }
                            }
                        },
                        Err(_) => 80 // Can't read 32 more bytes, must be 80
                    }
                },
                Err(_) => 80 // Can't skip to position 80, must be shorter
            };
            
            // Seek back to where we started (after version bytes)
            use tokio::io::AsyncSeekExt;
            reader.seek(std::io::SeekFrom::Start(current_pos)).await?;
            
            detected_size
        } else {
            get_header_size(ver_as_int)
        };

        // Debug logging for version 4+ blocks  
        if ver_as_int >= 4 {
            if block_count < 5 || (block_count >= 863785 && block_count <= 863790) {
                println!("  Block {}: Version {} detected, detected header_size = {} bytes", 
                         block_count, ver_as_int, header_size);
            }
        }

        // Read the rest of the header (header_size - 4 bytes already read for version)
        header_buffer.clear();
        header_buffer.extend_from_slice(&version_bytes); // Include version in header
        header_buffer.resize(header_size, 0);
        match reader.read_exact(&mut header_buffer[4..]).await {
            Ok(_) => {
                if ver_as_int >= 4 && (block_count < 5 || (block_count >= 863785 && block_count <= 863790)) {
                    println!("    ✅ Successfully read {} bytes for header", header_size);
                }
            },
            Err(e) => {
                eprintln!("  Failed to read header (size {}) at block {}: {}", 
                         header_size, block_count, e);
                eprintln!("    Version: {}, expected {} bytes", ver_as_int, header_size);
                break;
            }
        }

        let mut block_header = parse_block_header_sync(&header_buffer, header_size)?;
        
        // Try to get height from chain_metadata (if leveldb was parsed)
        let cf_metadata = db.cf_handle("chain_metadata");
        let block_height = if block_header.hash_prev_block == [0u8; 32] {
            // Genesis block has height 0
            Some(0)
        } else if let Some(cf) = cf_metadata {
            // Try to look up height from chain_metadata using 'h' + block_hash
            let mut height_key = vec![b'h'];
            height_key.extend_from_slice(&block_header.block_hash);
            
            match db.get_cf(&cf, &height_key) {
                Ok(Some(height_bytes)) if height_bytes.len() == 4 => {
                    // Found height in metadata
                    let height = i32::from_le_bytes([
                        height_bytes[0],
                        height_bytes[1], 
                        height_bytes[2],
                        height_bytes[3]
                    ]);
                    Some(height)
                }
                _ => None  // Height not in metadata yet
            }
        } else {
            None
        };
        
        block_header.block_height = block_height;
        
        // Store block data
        let block_hash_vec = block_header.block_hash.to_vec();
        
        // Extract nBits for chainwork calculation (bytes 72-76 in header)
        let n_bits = if header_buffer.len() >= 76 {
            u32::from_le_bytes([
                header_buffer[72],
                header_buffer[73],
                header_buffer[74],
                header_buffer[75],
            ])
        } else {
            0
        };
        
        // Store in blocks CF: block_hash -> header_buffer  
        // ALL blocks are stored, even if height is unknown
        batch_items.push((block_hash_vec.clone(), header_buffer.clone()));
        
        // If we have a height (genesis or previously resolved), store mappings
        if let Some(height) = block_height {
            let cf_metadata = db.cf_handle("chain_metadata").ok_or("chain_metadata CF not found")?;
            
            // Store: 'h' + block_hash -> height (for parent lookup, uses internal byte order)
            let mut height_key = vec![b'h'];
            height_key.extend_from_slice(&block_hash_vec);
            let height_bytes = height.to_le_bytes().to_vec();
            db.put_cf(&cf_metadata, &height_key, &height_bytes)
                .map_err(|e| format!("Failed to store height: {}", e))?;
            
            // Store: height -> block_hash (for height-based queries, use display format - reversed)
            let reversed_hash: Vec<u8> = block_hash_vec.iter().rev().cloned().collect();
            db.put_cf(&cf_metadata, &height_bytes, &reversed_hash)
                .map_err(|e| format!("Failed to store height mapping: {}", e))?;
            
            // Calculate and store chainwork
            if n_bits > 0 {
                // Calculate work for this block
                let block_work = calculate_work_from_bits(n_bits);
                
                // Get parent chainwork (if not genesis)
                let parent_chainwork = if height > 0 {
                    let prev_height = height - 1;
                    let mut chainwork_key = vec![b'w']; // 'w' prefix for chainwork
                    chainwork_key.extend_from_slice(&prev_height.to_le_bytes());
                    
                    match db.get_cf(&cf_metadata, &chainwork_key)? {
                        Some(parent_work_bytes) => {
                            if parent_work_bytes.len() == 32 {
                                let mut parent_work = [0u8; 32];
                                parent_work.copy_from_slice(&parent_work_bytes);
                                Some(parent_work)
                            } else {
                                None
                            }
                        }
                        None => None,
                    }
                } else {
                    None // Genesis has no parent
                };
                
                // Calculate cumulative chainwork
                let chainwork = if let Some(parent_work) = parent_chainwork {
                    // Add parent chainwork + this block's work
                    use num_bigint::BigUint;
                    let parent_big = BigUint::from_bytes_be(&parent_work);
                    let block_big = BigUint::from_bytes_be(&block_work);
                    let total = parent_big + block_big;
                    
                    let work_bytes = total.to_bytes_be();
                    let mut result = [0u8; 32];
                    let start = 32 - work_bytes.len().min(32);
                    result[start..].copy_from_slice(&work_bytes[..work_bytes.len().min(32)]);
                    result
                } else {
                    // Genesis block or parent not found - use just this block's work
                    block_work
                };
                
                // Store chainwork: 'w' + height -> chainwork (32 bytes)
                let mut chainwork_key = vec![b'w'];
                chainwork_key.extend_from_slice(&height.to_le_bytes());
                db.put_cf(&cf_metadata, &chainwork_key, &chainwork)
                    .map_err(|e| format!("Failed to store chainwork: {}", e))?;
            }
        }
        
        block_count += 1;
        
        // Debug: print every 100 blocks
        if block_count % 100 == 0 {
            println!("  Processed {} blocks so far", block_count);
        }
        
        // Write batch when it reaches the target size
        if batch_items.len() >= BATCH_SIZE * 2 {
            batch_put_cf(db.clone(), "blocks", batch_items.clone()).await?;
            println!("  Wrote {} blocks to database", BATCH_SIZE);
            batch_items.clear();
        }
        
        // Process transactions
        let block_version = block_header.n_version;
        let block_hash_slice = &block_header.block_hash;
        let block_height_val = block_header.block_height;
        
        // Process transactions - errors are non-fatal, we'll seek to correct position anyway
        match process_transaction(&mut reader, block_version, block_hash_slice, block_height_val, db.clone(), &mut tx_batch, fast_sync).await {
            Ok(_) => {
                // Successfully processed transactions
                // Flush batch if needed
                if tx_batch.should_flush() {
                    if let Err(e) = tx_batch.flush().await {
                        eprintln!("Warning: Failed to flush transaction batch: {}", e);
                    }
                }
            },
            Err(e) => {
                eprintln!("Warning: Failed to process transactions for block {}: {}", block_count, e);
            }
        }
        
        // CRITICAL: Always seek to the EXACT position where next block starts
        // This ensures we never skip blocks regardless of transaction parsing issues
        let current_pos = reader.stream_position().await?;
        if current_pos != next_block_pos {
            // Seek to exact position
            use tokio::io::AsyncSeekExt;
            reader.seek(std::io::SeekFrom::Start(next_block_pos)).await?;
            
            if block_count % 1000 == 0 && current_pos != next_block_pos {
                eprintln!("  Block {}: position adjusted from {} to {} (diff: {} bytes)", 
                         block_count, current_pos, next_block_pos, 
                         next_block_pos as i64 - current_pos as i64);
            }
        }
    }
    
    // Write any remaining items
    if !batch_items.is_empty() {
        let remaining_count = batch_items.len();
        batch_put_cf(db.clone(), "blocks", batch_items).await?;
        println!("Wrote {} remaining blocks to database", remaining_count);
    }
    
    // Flush any remaining transaction batch writes
    if tx_batch.pending_count() > 0 {
        tx_batch.flush().await?;
        println!("Flushed {} pending transaction operations", tx_batch.pending_count());
    }

    println!("File complete: {} total blocks processed", block_count);
    Ok(())
}

fn get_header_size(ver_as_int: u32) -> usize {
    match ver_as_int {
        4 | 5 | 6 | 8 | 9 | 10 | 11 => 112,
        7 => 80,
        _ => 80,
    }
}

pub fn parse_block_header_sync(slice: &[u8], _header_size: usize) -> Result<CBlockHeader, MyError> {
    if slice.len() < 80 {
        return Err(MyError::new("Header too short"));
    }

    let mut offset = 0;

    // Read block version
    let n_version = u32::from_le_bytes(
        slice[offset..offset+4].try_into()
            .map_err(|_| MyError::new("Invalid version bytes"))?
    );
    offset += 4;

    // Read previous block hash
    let mut hash_prev_block = [0u8; 32];
    hash_prev_block.copy_from_slice(&slice[offset..offset+32]);
    offset += 32;

    // Read merkle root
    let mut hash_merkle_root = [0u8; 32];
    hash_merkle_root.copy_from_slice(&slice[offset..offset+32]);
    offset += 32;

    // Read time, bits, nonce
    let n_time = u32::from_le_bytes(
        slice[offset..offset+4].try_into()
            .map_err(|_| MyError::new("Invalid time bytes"))?
    );
    offset += 4;
    let n_bits = u32::from_le_bytes(
        slice[offset..offset+4].try_into()
            .map_err(|_| MyError::new("Invalid bits bytes"))?
    );
    offset += 4;
    let n_nonce = u32::from_le_bytes(
        slice[offset..offset+4].try_into()
            .map_err(|_| MyError::new("Invalid nonce bytes"))?
    );
    offset += 4;

    // Calculate block hash - hash size depends on version
    // v0-3: hash 80 bytes with Quark
    // v4-6: hash 112 bytes (80 + 32 accumulator) with SHA256d
    // v7: hash 80 bytes with SHA256d
    // v8+: hash 112 bytes (80 + 32 sapling root) with SHA256d
    let hash_size = match n_version {
        0..=3 => 80,
        4..=6 => 112,  // Include accumulator checkpoint
        7 => 80,
        _ => 112,  // v8+ includes sapling root
    };
    
    let hash_bytes = &slice[..hash_size.min(slice.len())];
    let reversed_hash = match n_version {
        0..=3 => {
            // For v0-v3, use Quark hash on first 80 bytes
            let output_hash = call_quark_hash(hash_bytes);
            output_hash.to_vec()
        }
        _ => {
            // For v4+, use SHA256d on full header (80 or 112 bytes depending on version)
            let first_hash = Sha256::digest(hash_bytes);
            let block_hash = Sha256::digest(&first_hash);
            block_hash.to_vec()
        }
    };

    let block_hash: [u8; 32] = reversed_hash.try_into()
        .map_err(|_| MyError::new("Failed to convert hash"))?;

    // Handle version-specific fields
    let (hash_final_sapling_root, n_accumulator_checkpoint) = match n_version {
        7 => (None, None),
        8..=11 => {
            if offset + 32 <= slice.len() {
                let mut sapling_root = [0u8; 32];
                sapling_root.copy_from_slice(&slice[offset..offset+32]);
                (Some(sapling_root), None)
            } else {
                (None, None)
            }
        }
        4..=6 => {
            if offset + 32 <= slice.len() {
                let mut accumulator = [0u8; 32];
                accumulator.copy_from_slice(&slice[offset..offset+32]);
                (None, Some(accumulator))
            } else {
                (None, None)
            }
        }
        _ => (None, None),
    };

    // Height will be assigned sequentially by process_blk_file
    let block_height = Some(0);

    Ok(CBlockHeader {
        n_version,
        block_hash,
        block_height,
        hash_prev_block,
        hash_merkle_root,
        n_time,
        n_bits,
        n_nonce,
        n_accumulator_checkpoint,
        hash_final_sapling_root,
    })
}
