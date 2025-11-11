use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader};
use rocksdb::DB;
use crate::types::{CBlockHeader, AppState, MyError, CTransaction, CTxIn, CTxOut, COutPoint, CScript};
use rustyblox::call_quark_hash;
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
        let header_size = get_header_size(ver_as_int);

        // Read the rest of the header (header_size - 4 bytes already read for version)
        header_buffer.clear();
        header_buffer.extend_from_slice(&version_bytes); // Include version in header
        header_buffer.resize(header_size, 0);
        match reader.read_exact(&mut header_buffer[4..]).await {
            Ok(_) => {},
            Err(e) => {
                eprintln!("  Failed to read header (size {}) at block {}: {}", 
                         header_size, block_count, e);
                break;
            }
        }

        let mut block_header = parse_block_header_sync(&header_buffer, header_size)?;
        
        // Determine block height by looking up parent block's height
        let block_height = if block_header.hash_prev_block == [0u8; 32] {
            // Genesis block has height 0
            Some(0)
        } else {
            // Look up parent block's height from chain_metadata CF
            let cf_metadata = db.cf_handle("chain_metadata").ok_or("chain_metadata CF not found")?;
            let mut height_key = vec![b'h'];
            height_key.extend_from_slice(&block_header.hash_prev_block);
            
            match db.get_cf(&cf_metadata, &height_key) {
                Ok(Some(height_bytes)) if height_bytes.len() == 4 => {
                    match height_bytes.as_slice().try_into() {
                        Ok(bytes) => {
                            let parent_height = i32::from_le_bytes(bytes);
                            Some(parent_height + 1)
                        }
                        Err(_) => {
                            eprintln!("  Failed to convert height bytes");
                            None
                        }
                    }
                }
                Ok(Some(_)) => {
                    eprintln!("  Invalid height data for parent block");
                    None
                }
                Ok(None) => {
                    // Parent not indexed yet - block is out of order, will be processed later
                    None
                }
                Err(e) => {
                    eprintln!("  Error looking up parent height: {}", e);
                    None
                }
            }
        };
        
        block_header.block_height = block_height;
        
        // Store block data
        let block_hash_vec = block_header.block_hash.to_vec();
        
        // Store in blocks CF: block_hash -> header_buffer
        batch_items.push((block_hash_vec.clone(), header_buffer.clone()));
        
        // If we have a height, store mappings in chain_metadata CF
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
        
        match process_transaction(&mut reader, block_version, block_hash_slice, db.clone(), &mut tx_batch, fast_sync).await {
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
                eprintln!("Warning: Failed to process transactions for block at height {:?}: {}", 
                    block_header.block_height, e);
                // Skip remaining transaction data to get to next block
                let tx_data_size = (block_size as usize).saturating_sub(header_size);
                if tx_data_size > 0 {
                    let mut skip_buf = vec![0u8; tx_data_size.min(65536)];
                    let mut remaining = tx_data_size;
                    
                    while remaining > 0 {
                        let to_read = remaining.min(65536);
                        skip_buf.truncate(to_read);
                        reader.read_exact(&mut skip_buf).await?;
                        remaining -= to_read;
                    }
                }
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

fn parse_block_header_sync(slice: &[u8], _header_size: usize) -> Result<CBlockHeader, MyError> {
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

    // Calculate block hash - ALWAYS hash only first 80 bytes regardless of version
    let hash_bytes = &slice[..80.min(slice.len())];
    let reversed_hash = match n_version {
        0..=3 => {
            // For v0-v3, use Quark hash on first 80 bytes
            let output_hash = call_quark_hash(hash_bytes);
            output_hash.to_vec()
        }
        _ => {
            // For v4+, use SHA256d on first 80 bytes
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
