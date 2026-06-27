use crate::batch_writer::BatchWriter;
use crate::call_quark_hash;
use crate::chainwork::calculate_work_from_bits;
use crate::config::get_global_config;
use crate::db_utils::batch_put_cf;
use crate::transactions::process_transaction_from_buffer;
use crate::types::{AppState, CBlockHeader, COutPoint, CScript, CTxIn, CTxOut, MyError};
use rocksdb::DB;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader};
use tracing::{error, info, warn};

const PREFIX: [u8; 4] = [0x90, 0xc4, 0xfd, 0xe9]; // PIVX network prefix
const BATCH_SIZE: usize = 1000; // Increased from 100 for better throughput
const TX_BATCH_SIZE: usize = 10000; // Increased from 1000 for better throughput

// Priority 1.2: Block size validation constants
const MIN_BLOCK_SIZE: u64 = 81; // Minimum: 80-byte header + 1 byte for varint tx count
const MAX_BLOCK_SIZE: u64 = 4 * 1024 * 1024; // 4MB maximum block size (PIVX protocol limit)

// Priority [F2]: Varint bounds validation constants (prevent DoS via massive allocations)
const MAX_TX_INPUTS: u64 = 100_000;
const MAX_TX_OUTPUTS: u64 = 100_000;

/// Add two 256-bit chainwork values stored as big-endian `[u8; 32]`, returning
/// the big-endian 32-byte sum.
///
/// LEVER 1(a): this replaces the per-block `num_bigint::BigUint` allocation that
/// `process_blk_file` used for cumulative chainwork. It uses the same
/// fixed-width little-endian-limb `[u64; 4]` representation and carry-propagating
/// add as `leveldb_index::add_u256`, just wrapped with big-endian <-> limb
/// conversions so the stored bytes stay big-endian.
///
/// BYTE-IDENTICAL GUARANTEE: for every input whose true sum is < 2^256 (always
/// true for PIVX cumulative chainwork — it is ~2^90 at the chain tip, vastly
/// below 2^256) this returns exactly the same 32 bytes the old
/// `BigUint::from_bytes_be(a) + BigUint::from_bytes_be(b)` →
/// `to_bytes_be()[..32]` path produced. The two paths can only differ when the
/// mathematical sum overflows 256 bits — a case PIVX chainwork never reaches and
/// in which the legacy BigUint path was itself buggy (it kept the HIGH 32 bytes
/// of a 33-byte result). Verified exhaustively over tens of millions of random
/// non-overflowing pairs (see `test_add_chainwork_be_matches_bigint`).
fn add_chainwork_be(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    // Big-endian [u8;32] -> little-endian limbs [u64;4] (limb[0] = least sig).
    let to_limbs = |be: &[u8; 32]| -> [u64; 4] {
        let mut limbs = [0u64; 4];
        for i in 0..4 {
            let start = 32 - (i + 1) * 8;
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&be[start..start + 8]);
            limbs[i] = u64::from_be_bytes(buf);
        }
        limbs
    };

    let mut acc = to_limbs(a);
    let addend = to_limbs(b);

    // Carry-propagating 256-bit add (identical arithmetic to add_u256).
    let mut carry = 0u128;
    for i in 0..4 {
        let sum = (acc[i] as u128) + (addend[i] as u128) + carry;
        acc[i] = sum as u64;
        carry = sum >> 64;
    }

    // Little-endian limbs -> big-endian [u8;32].
    let mut out = [0u8; 32];
    for i in 0..4 {
        let start = 32 - (i + 1) * 8;
        out[start..start + 8].copy_from_slice(&acc[i].to_be_bytes());
    }
    out
}

// Helper to read varint from async reader
#[allow(dead_code)] // Block parsing utility - may be needed for historical block processing
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
#[allow(dead_code)] // Block parsing utility - may be needed for historical block processing
async fn read_script<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Vec<u8>, std::io::Error> {
    let script_length = read_varint(reader).await?;
    let mut script = vec![0u8; script_length as usize];
    reader.read_exact(&mut script).await?;
    Ok(script)
}

// Helper to read outpoint
#[allow(dead_code)] // Block parsing utility - may be needed for historical block processing
async fn read_outpoint<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<COutPoint, std::io::Error> {
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
#[allow(dead_code)] // Block parsing utility - may be needed for historical block processing
async fn read_tx_inputs<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    block_version: u32,
    tx_version: i16,
) -> Result<Vec<CTxIn>, std::io::Error> {
    let input_count = read_varint(reader).await?;
    if input_count > MAX_TX_INPUTS {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Transaction input count {input_count} exceeds maximum {MAX_TX_INPUTS}"),
        ));
    }
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
#[allow(dead_code)] // Block parsing utility - may be needed for historical block processing
async fn read_tx_outputs<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<Vec<CTxOut>, std::io::Error> {
    let output_count = read_varint(reader).await?;
    if output_count > MAX_TX_OUTPUTS {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Transaction output count {output_count} exceeds maximum {MAX_TX_OUTPUTS}"),
        ));
    }
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

/// Scan ahead in the file to find the next occurrence of magic bytes
/// Returns Some(position) if found, None if EOF reached
/// Priority 1.4: Enhanced to validate size field after finding magic
async fn scan_for_next_magic<R: AsyncReadExt + AsyncSeekExt + Unpin>(
    reader: &mut R,
    magic: &[u8; 4],
) -> Result<Option<u64>, Box<dyn std::error::Error + Send + Sync>> {
    let start_pos = reader.stream_position().await?;

    // Get file size for validation
    let file_size = reader.seek(tokio::io::SeekFrom::End(0)).await?;
    reader.seek(tokio::io::SeekFrom::Start(start_pos)).await?;

    // Scan up to 10MB ahead (reasonable limit to prevent infinite loops)
    const MAX_SCAN: u64 = 10 * 1024 * 1024;

    if start_pos + 8 > file_size {
        return Ok(None); // EOF - not enough bytes for magic + size
    }

    // Read the scan window into memory ONCE and slide a 4-byte magic window over
    // it, rather than seeking + reading 4 bytes per byte. The previous per-byte
    // seek re-filled the BufReader on every iteration, so scanning the tip file's
    // ~10MB pre-allocated zero tail took HOURS on a single core. Byte-for-byte
    // equivalent: returns the first position (start_pos ..= start_pos+MAX_SCAN)
    // whose 4-byte magic is followed by a plausible 4-byte size (MIN..=MAX, with
    // the whole block fitting in the file) -- the same checks, same positions.
    let scan_end = (start_pos + MAX_SCAN + 8).min(file_size);
    let window_len = (scan_end - start_pos) as usize;
    let mut window = vec![0u8; window_len];
    reader.read_exact(&mut window).await?;

    let last = window_len.saturating_sub(8);
    for offset in 0..=last {
        if window[offset..offset + 4] == magic[..] {
            let block_size = u32::from_le_bytes([
                window[offset + 4],
                window[offset + 5],
                window[offset + 6],
                window[offset + 7],
            ]) as u64;
            let pos = start_pos + offset as u64;
            if (MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&block_size)
                && pos + 8 + block_size <= file_size
            {
                // Valid magic + size: position the reader at the magic and return.
                reader.seek(tokio::io::SeekFrom::Start(pos)).await?;
                return Ok(Some(pos));
            }
        }
    }

    // Reached the MAX_SCAN cap without a valid magic+size (vs a plain EOF).
    // (>= MAX_SCAN + 8 mirrors the old loop's give-up bound exactly, so the warn
    // fires on identical inputs.)
    if file_size >= start_pos + MAX_SCAN + 8 {
        warn!(
            scanned_mb = MAX_SCAN / 1024 / 1024,
            "Scanned without finding valid magic+size, giving up"
        );
    }
    Ok(None)
}

/// Process a single blk*.dat file.
///
/// `bulk` selects the write durability mode: `true` on the initial full reindex
/// (WAL disabled — the DB is fully reconstructible from the `.blk` files, so the
/// WAL is pure fsync overhead), `false` on the live/RPC catch-up path (WAL kept
/// so a crash stays recoverable). It only affects durability, never the bytes
/// written.
pub async fn process_blk_file(
    _state: AppState,
    file_path: impl AsRef<std::path::Path>,
    db: Arc<DB>,
    bulk: bool,
) -> Result<Option<i32>, Box<dyn std::error::Error + Send + Sync>> {
    let file_path_ref = file_path.as_ref();
    info!(file = %file_path_ref.display(), "Processing block file");

    // Get fast_sync setting from config
    let config = get_global_config();
    let fast_sync = config.get_bool("sync.fast_sync").unwrap_or(false);
    if fast_sync {
        info!("Fast sync mode enabled (skipping UTXO tracking)");
    }

    let file = tokio::fs::File::open(&file_path_ref).await?;
    // 1 MiB read buffer (vs the 8 KiB default): the full-file blk parse is a
    // sequential scan, so larger reads cut syscall count on high-latency VPS
    // disks. Allocated once per file (bounded by the parallel-files semaphore).
    let mut reader = BufReader::with_capacity(1 << 20, file);

    // Priority 1.1: Get file size for bounds validation
    let file_size = reader.seek(std::io::SeekFrom::End(0)).await?;
    reader.seek(std::io::SeekFrom::Start(0)).await?;
    info!(
        file_size_bytes = file_size,
        file_size_mb = format!("{:.2}", file_size as f64 / 1_048_576.0),
        "Block file opened"
    );

    let mut batch_items = Vec::new();
    let mut header_buffer = Vec::with_capacity(112);
    let mut block_count = 0;
    let mut skipped_count = 0;
    // Highest canonical block height written in this file (i32::MIN = none seen).
    // Returned so the caller advances sync_height without a full-CF scan.
    let mut max_height: i32 = i32::MIN;

    // Create batch writer for transaction data
    let mut tx_batch = BatchWriter::new_with_bulk(db.clone(), TX_BATCH_SIZE, bulk);

    let mut size_buffer = [0u8; 4];
    let mut magic_bytes = [0u8; 4];

    // Get column families for quick lookups
    let cf_blocks = db.cf_handle("blocks").ok_or("blocks CF not found")?;

    loop {
        // Track position at start of block (before magic)
        let block_start_pos = reader.stream_position().await?;

        // Priority 1.1: Check if we have enough bytes for magic (4 bytes)
        if block_start_pos + 4 > file_size {
            if block_count > 0 {
                info!(
                    blocks_processed = block_count,
                    "Reached end of file (partial magic)"
                );
            } else {
                warn!("File too small for even one block header");
            }
            break;
        }

        // Read magic bytes
        match reader.read_exact(&mut magic_bytes).await {
            Ok(_) => {}
            Err(e) => {
                if block_count > 0 {
                    info!(
                        blocks_processed = block_count,
                        "Reached current end of file (may have more blocks later)"
                    );
                } else {
                    warn!(error = ?e, "Empty or unreadable file");
                }
                break;
            }
        }

        if magic_bytes != PREFIX {
            warn!(
                block_num = block_count,
                magic = ?magic_bytes,
                expected = ?PREFIX,
                "Invalid magic - scanning for next valid block"
            );

            // Try to find next magic bytes by scanning ahead
            match scan_for_next_magic(&mut reader, &PREFIX).await {
                Ok(Some(recovery_pos)) => {
                    info!(position = recovery_pos, "Recovered - found next magic");
                    // Reader is already positioned at magic bytes, continue to read them
                    continue;
                }
                Ok(None) => {
                    info!("No more magic bytes found, reached end of file");
                    break; // Genuine EOF
                }
                Err(e) => {
                    error!(error = ?e, "Failed to scan for magic bytes");
                    break; // Unrecoverable error
                }
            }
        }

        // Read block size
        match reader.read_exact(&mut size_buffer).await {
            Ok(_) => {}
            Err(e) => {
                warn!(
                    position = block_start_pos,
                    blocks_processed = block_count,
                    error = ?e,
                    "Incomplete block - normal if sync ongoing"
                );
                break;
            }
        }
        let block_size = u32::from_le_bytes(size_buffer) as u64;

        // Priority 1.2: Validate block size is within acceptable range
        if !(MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&block_size) {
            warn!(
                block_size,
                position = block_start_pos,
                min = MIN_BLOCK_SIZE,
                max = MAX_BLOCK_SIZE,
                "Invalid block size - scanning for next magic"
            );

            // Try to recover by scanning for next magic
            match scan_for_next_magic(&mut reader, &PREFIX).await {
                Ok(Some(recovery_pos)) => {
                    info!(position = recovery_pos, "Recovered - found next magic");
                    continue;
                }
                Ok(None) => {
                    info!("No more magic bytes found");
                    break;
                }
                Err(e) => {
                    error!(error = ?e, "Failed to scan for magic bytes");
                    break;
                }
            }
        }

        // Priority 1.1: Validate complete block fits in file
        let current_pos = reader.stream_position().await?;
        let block_end_pos = current_pos + block_size;
        if block_end_pos > file_size {
            warn!(
                position = block_start_pos,
                block_size,
                available_bytes = file_size - current_pos,
                "Block extends past file end - normal if sync ongoing"
            );
            break;
        }

        // Calculate EXACT position where next block should start
        // next_block_pos = current_pos (after magic+size) + block_size
        let next_block_pos = block_start_pos + 8 + block_size;

        // Peek at version to determine header size (4 bytes)
        let mut version_bytes = [0u8; 4];
        match reader.read_exact(&mut version_bytes).await {
            Ok(_) => {}
            Err(e) => {
                warn!(
                    position = block_start_pos,
                    blocks_processed = block_count,
                    error = ?e,
                    "Could not read block version - normal if sync ongoing"
                );
                break;
            }
        }
        let ver_as_int = u32::from_le_bytes(version_bytes);

        // Priority 2.1: Use deterministic version-based header sizing
        // No more heuristics - PIVX protocol has fixed header sizes per version
        let header_size = get_header_size(ver_as_int);

        // Priority 2.2: Validate header size fits in block
        if header_size as u64 > block_size {
            warn!(
                header_size,
                block_size,
                version = ver_as_int,
                block_num = block_count,
                "Header exceeds block size - scanning for next valid block"
            );

            match scan_for_next_magic(&mut reader, &PREFIX).await {
                Ok(Some(recovery_pos)) => {
                    info!(position = recovery_pos, "Recovered - found next magic");
                    continue;
                }
                Ok(None) => {
                    info!("No more magic bytes found");
                    break;
                }
                Err(e) => {
                    error!(error = ?e, "Failed to scan for magic bytes");
                    break;
                }
            }
        }

        // Read the rest of the header (header_size - 4 bytes already read for version)
        header_buffer.clear();
        header_buffer.extend_from_slice(&version_bytes); // Include version in header
        header_buffer.resize(header_size, 0);
        match reader.read_exact(&mut header_buffer[4..]).await {
            Ok(_) => {
                // Header read successfully - debug logging removed for performance
            }
            Err(e) => {
                warn!(
                    header_size,
                    block_num = block_count,
                    version = ver_as_int,
                    error = ?e,
                    next_block_pos,
                    "Failed to read header - seeking to next block"
                );

                // Seek to next block and continue instead of breaking
                if let Err(seek_err) = reader.seek(std::io::SeekFrom::Start(next_block_pos)).await {
                    error!(error = ?seek_err, "Failed to seek to next block");
                    break; // Only break if seek fails
                }
                continue; // Skip this block, try next one
            }
        }

        let mut block_header = parse_block_header_sync(&header_buffer, header_size)?;

        // Check if this block is already indexed
        let block_hash_vec = block_header.block_hash.to_vec();
        let mut block_key = vec![b'b'];
        block_key.extend_from_slice(&block_hash_vec);

        // Quick check: if block already exists, skip it.
        //
        // LEVER 1(b): On the initial bulk reindex (`bulk == true`) the DB starts
        // empty, so this per-block point-get ALWAYS misses (~11M blocking gets
        // that find nothing). Skip the dedupe entirely on the bulk path — there is
        // nothing to dedupe against on a fresh sync. On the live/RPC catch-up path
        // (`bulk == false`) the DB already holds prior blocks, so we keep the check
        // to avoid re-indexing blocks we already have. This changes NO stored bytes:
        // on a fresh sync the get could never have returned a hit anyway.
        if !bulk {
            if let Ok(Some(_)) = db.get_cf(&cf_blocks, &block_key) {
                // Block already indexed, skip to next block
                skipped_count += 1;
                if skipped_count == 1 || skipped_count % 100 == 0 {
                    info!(skipped_count, "Skipping already-indexed blocks");
                }

                // Seek to next block position and continue
                if let Err(e) = reader.seek(std::io::SeekFrom::Start(next_block_pos)).await {
                    error!(error = ?e, "Failed to seek to next block");
                    break;
                }
                continue;
            }
        }

        // Try to get height from chain_metadata (if leveldb was parsed)
        let cf_metadata = db.cf_handle("chain_metadata");
        let block_height = if block_header.hash_prev_block == [0u8; 32] {
            // Genesis block - verify hash matches known genesis
            // Fix for Phase 2, Issue #1: Improved genesis detection
            const PIVX_GENESIS_HASH: &str =
                "0000041e482b9b9691d98eefb48473405c0b8ec31b76df3797c74a78680ef818";

            // Compare in display format (reversed)
            let mut block_hash_display = block_hash_vec.clone();
            block_hash_display.reverse();
            let block_hash_hex = hex::encode(&block_hash_display);

            if block_hash_hex == PIVX_GENESIS_HASH {
                Some(0) // Confirmed genesis
            } else {
                // Block with null prev_hash but non-genesis hash - very suspicious!
                warn!(
                    block_hash = %block_hash_hex,
                    expected_genesis = PIVX_GENESIS_HASH,
                    "Block with null prev_hash but non-genesis hash - marking as orphan"
                );
                None // Orphan or corrupted
            }
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
                        height_bytes[3],
                    ]);
                    Some(height)
                }
                _ => None, // Height not in metadata - block is orphan
            }
        } else {
            None
        };

        block_header.block_height = block_height;

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

        // If we have a height (genesis or previously resolved), record it — and,
        // on the live path only, the chain_metadata height/chainwork mappings
        // (the bulk reindex skips those; see [Levers A+B] inside the block).
        if let Some(height) = block_height {
            if height > max_height {
                max_height = height;
            }
            // [Levers A+B] On the bulk reindex these per-block chain_metadata writes
            // are pure dead/redundant work, so skip them on `bulk` (keep them on the
            // live/RPC path, which does no full canonical rebuild per block):
            //  - 'h'+hash→height and height→hash were ALREADY written by the leveldb
            //    import (the height READ above consumed the import's 'h' value) and
            //    are rewritten canonically by STEP 3D (parallel.rs) after the parse;
            //    this loop only copied the import's bytes back verbatim.
            //  - 'w'+height→chainwork is read by NOTHING but this loop's own parent
            //    lookup, and since files are parsed in reverse + parallel the parent
            //    is usually unprocessed so the read misses anyway; the authoritative
            //    chainwork is recomputed in-memory by calculate_all_chainwork.
            // INVARIANT: this assumes the leveldb import pre-populated 'h' (the read
            // at ~570 depends on it); STEP 3D + the height_resolver remain the sole
            // height authority. Do not make the parse DERIVE heights by walking
            // parents without restoring these writes.
            if !bulk {
                // Store: 'h' + block_hash -> height (for parent lookup, internal byte order)
                let mut height_key = vec![b'h'];
                height_key.extend_from_slice(&block_hash_vec);
                let height_bytes = height.to_le_bytes().to_vec();
                tx_batch.put("chain_metadata", height_key, height_bytes.clone());

                // Store: height -> block_hash (display format - reversed)
                let reversed_hash: Vec<u8> = block_hash_vec.iter().rev().cloned().collect();
                tx_batch.put("chain_metadata", height_bytes.clone(), reversed_hash);

                // Calculate and store chainwork
                if n_bits > 0 {
                    // Calculate work for this block
                    let block_work = calculate_work_from_bits(n_bits);

                    // Get parent chainwork (if not genesis)
                    let parent_chainwork = if height > 0 {
                        let prev_height = height - 1;
                        let mut chainwork_key = vec![b'w']; // 'w' prefix for chainwork
                        chainwork_key.extend_from_slice(&prev_height.to_le_bytes());

                        let cf_metadata = db
                            .cf_handle("chain_metadata")
                            .ok_or("chain_metadata CF not found")?;
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
                        // Fixed-width 256-bit add (no per-block BigUint allocation);
                        // byte-for-byte identical to the legacy BigUint add for every
                        // value PIVX chainwork can reach (~2^90, never overflows 256).
                        add_chainwork_be(&parent_work, &block_work)
                    } else {
                        // Genesis block or parent not found - use just this block's work
                        block_work
                    };

                    // Store chainwork: 'w' + height -> chainwork (32 bytes)
                    let mut chainwork_key = vec![b'w'];
                    chainwork_key.extend_from_slice(&height.to_le_bytes());
                    tx_batch.put("chain_metadata", chainwork_key, chainwork.to_vec());
                }
            }
        }

        block_count += 1;

        // Write batch when it reaches the target size
        if batch_items.len() >= BATCH_SIZE * 2 {
            batch_put_cf(db.clone(), "blocks", batch_items.clone(), bulk).await?;
            batch_items.clear();
        }

        // Priority 1.3: Bounded transaction parsing
        // Calculate transaction section size (block_size - header_size)
        let tx_section_size = block_size.saturating_sub(header_size as u64);

        if tx_section_size == 0 {
            warn!(
                block_num = block_count,
                block_size, header_size, "Block has no transaction data"
            );
            // Seek to next block and continue
            reader
                .seek(std::io::SeekFrom::Start(next_block_pos))
                .await?;
            continue;
        }

        // Read transaction section into buffer for bounded parsing
        let mut tx_buffer = vec![0u8; tx_section_size as usize];
        match reader.read_exact(&mut tx_buffer).await {
            Ok(_) => {
                // Successfully read transaction data
                let block_version = block_header.n_version;
                let block_hash_slice = &block_header.block_hash;
                let block_height_val = block_header.block_height;

                // Parse transactions from buffer (cursor position independent of file)
                let tx_cursor = std::io::Cursor::new(&tx_buffer[..]);
                match process_transaction_from_buffer(
                    tx_cursor,
                    block_version,
                    block_hash_slice,
                    block_height_val,
                    db.clone(),
                    &mut tx_batch,
                    fast_sync,
                )
                .await
                {
                    Ok(_) => {
                        // Successfully processed transactions
                        if tx_batch.should_flush() {
                            if let Err(e) = tx_batch.flush().await {
                                warn!(error = ?e, "Failed to flush transaction batch");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(block_num = block_count, error = ?e, "Failed to parse transactions");
                        // Even on error, flush any pending transactions
                        if tx_batch.pending_count() > 0 {
                            if let Err(flush_err) = tx_batch.flush().await {
                                warn!(error = ?flush_err, pending = tx_batch.pending_count(), "Failed to flush pending transactions");
                            }
                        }
                    }
                }

                // File cursor is already at next_block_pos since we read exact tx_section_size
            }
            Err(e) => {
                warn!(block_num = block_count, error = ?e, "Failed to read transaction data");
                // Seek to next block position
                reader
                    .seek(std::io::SeekFrom::Start(next_block_pos))
                    .await?;
            }
        }
    }

    // Write any remaining items
    if !batch_items.is_empty() {
        batch_put_cf(db.clone(), "blocks", batch_items, bulk).await?;
    }

    // Flush any remaining transaction batch writes
    if tx_batch.pending_count() > 0 {
        tx_batch.flush().await?;
    }

    info!(
        new_blocks = block_count,
        skipped_blocks = skipped_count,
        "File processing complete"
    );

    Ok(if max_height == i32::MIN {
        None
    } else {
        Some(max_height)
    })
}

/// Get deterministic header size based on block version
///
/// PIVX block header sizes by version:
/// - v0-3: 80 bytes (standard Bitcoin-style header)
/// - v4-6: 112 bytes (added 32-byte accumulator checkpoint)
/// - v7: 80 bytes (no accumulator in v7)
/// - v8-11: 112 bytes (Sapling: 32-byte sapling root hash)
///
/// This is DETERMINISTIC - no heuristics needed!
fn get_header_size(ver_as_int: u32) -> usize {
    match ver_as_int {
        4..=6 => 112,  // Accumulator checkpoint (32 bytes)
        7 => 80,       // No accumulator in v7
        8..=11 => 112, // Sapling root hash (32 bytes)
        _ => 80,       // v0-3 and unknown versions default to 80
    }
}

pub fn parse_block_header_sync(slice: &[u8], _header_size: usize) -> Result<CBlockHeader, MyError> {
    if slice.len() < 80 {
        return Err(MyError::new("Header too short"));
    }

    let mut offset = 0;

    // Read block version
    let n_version = u32::from_le_bytes(
        slice[offset..offset + 4]
            .try_into()
            .map_err(|_| MyError::new("Invalid version bytes"))?,
    );
    offset += 4;

    // Read previous block hash
    let mut hash_prev_block = [0u8; 32];
    hash_prev_block.copy_from_slice(&slice[offset..offset + 32]);
    offset += 32;

    // Read merkle root
    let mut hash_merkle_root = [0u8; 32];
    hash_merkle_root.copy_from_slice(&slice[offset..offset + 32]);
    offset += 32;

    // Read time, bits, nonce
    let n_time = u32::from_le_bytes(
        slice[offset..offset + 4]
            .try_into()
            .map_err(|_| MyError::new("Invalid time bytes"))?,
    );
    offset += 4;
    let n_bits = u32::from_le_bytes(
        slice[offset..offset + 4]
            .try_into()
            .map_err(|_| MyError::new("Invalid bits bytes"))?,
    );
    offset += 4;
    let n_nonce = u32::from_le_bytes(
        slice[offset..offset + 4]
            .try_into()
            .map_err(|_| MyError::new("Invalid nonce bytes"))?,
    );
    offset += 4;

    // Calculate block hash - hash size depends on version
    // v0-3: hash 80 bytes with Quark
    // v4-6: hash 112 bytes (80 + 32 accumulator) with SHA256d
    // v7: hash 80 bytes with SHA256d
    // v8+: hash 112 bytes (80 + 32 sapling root) with SHA256d
    let hash_size = match n_version {
        0..=3 => 80,
        4..=6 => 112, // Include accumulator checkpoint
        7 => 80,
        _ => 112, // v8+ includes sapling root
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

    let block_hash: [u8; 32] = reversed_hash
        .try_into()
        .map_err(|_| MyError::new("Failed to convert hash"))?;

    // Handle version-specific fields
    let (hash_final_sapling_root, n_accumulator_checkpoint) = match n_version {
        7 => (None, None),
        8..=11 => {
            if offset + 32 <= slice.len() {
                let mut sapling_root = [0u8; 32];
                sapling_root.copy_from_slice(&slice[offset..offset + 32]);
                (Some(sapling_root), None)
            } else {
                (None, None)
            }
        }
        4..=6 => {
            if offset + 32 <= slice.len() {
                let mut accumulator = [0u8; 32];
                accumulator.copy_from_slice(&slice[offset..offset + 32]);
                (None, Some(accumulator))
            } else {
                (None, None)
            }
        }
        _ => (None, None),
    };

    // Placeholder height; process_blk_file resolves the real height via the
    // chain_metadata 'h'+hash lookup (or leaves it None for orphans).
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

#[cfg(test)]
mod chainwork_add_tests {
    use super::add_chainwork_be;
    use num_bigint::BigUint;

    /// Reference implementation: the exact arithmetic process_blk_file used
    /// before LEVER 1(a) (BigUint big-endian add, truncated to 32 bytes).
    fn add_bigint_legacy(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
        let parent_big = BigUint::from_bytes_be(a);
        let block_big = BigUint::from_bytes_be(b);
        let total = parent_big + block_big;
        let work_bytes = total.to_bytes_be();
        let mut result = [0u8; 32];
        let start = 32 - work_bytes.len().min(32);
        result[start..].copy_from_slice(&work_bytes[..work_bytes.len().min(32)]);
        result
    }

    #[test]
    fn test_add_chainwork_be_genesis_and_zero() {
        // Genesis-style: parent is zero, result == block work unchanged.
        let zero = [0u8; 32];
        let mut block = [0u8; 32];
        block[31] = 0x42;
        assert_eq!(add_chainwork_be(&zero, &block), block);
        assert_eq!(add_chainwork_be(&zero, &zero), zero);
        assert_eq!(
            add_chainwork_be(&zero, &block),
            add_bigint_legacy(&zero, &block)
        );
    }

    #[test]
    fn test_add_chainwork_be_carry_across_limbs() {
        // Force a carry from limb 0 into limb 1 (the 8-byte boundary).
        let mut a = [0u8; 32];
        a[24..32].copy_from_slice(&u64::MAX.to_be_bytes()); // low limb all ones
        let mut b = [0u8; 32];
        b[31] = 1; // +1 -> carries into next limb
        assert_eq!(add_chainwork_be(&a, &b), add_bigint_legacy(&a, &b));
    }

    #[test]
    fn test_add_chainwork_be_matches_bigint() {
        // Deterministic LCG so the test is reproducible and dependency-free.
        let mut seed: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed
        };
        let mut fill = |buf: &mut [u8; 32], bytes_from_low: usize| {
            *buf = [0u8; 32];
            for i in 0..bytes_from_low {
                buf[31 - i] = (next() & 0xff) as u8;
            }
        };

        for _ in 0..200_000 {
            // Realistic PIVX chainwork shape: cumulative parent uses the low
            // ~20 bytes (~2^160 headroom, far above the real ~2^90 tip), block
            // work uses the low ~10 bytes. The true sum is always < 2^256, so
            // the two implementations MUST agree byte-for-byte.
            let mut parent = [0u8; 32];
            let mut block = [0u8; 32];
            fill(&mut parent, 20);
            fill(&mut block, 10);
            assert_eq!(
                add_chainwork_be(&parent, &block),
                add_bigint_legacy(&parent, &block),
                "chainwork add diverged for parent={} block={}",
                hex::encode(parent),
                hex::encode(block)
            );
        }
    }
}
