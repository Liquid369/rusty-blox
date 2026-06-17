// Block-Related API Endpoints
//
// Endpoints for querying block information.
// Block data is immutable once confirmed, making it ideal for caching.

use axum::{Json, Extension, extract::Path, http::StatusCode};
use rocksdb::DB;
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use std::time::Duration;

use crate::cache::CacheManager;
use crate::chain_state::get_chain_state;
use crate::blocks::parse_block_header_sync;
use super::types::{BlockHash, BlockbookError};
use super::helpers::{not_found, internal_error};

/// nBits (compact target) -> difficulty.
///
/// difficulty = difficulty-1 target / current target. Reproduced locally to
/// avoid cross-module coupling; mirrors `block_detail::bits_to_difficulty`.
/// The old `256^26 / target` form omitted the 0xffff mantissa of the
/// difficulty-1 target, leaving results 65535x too small.
fn bits_to_difficulty(bits: u32) -> f64 {
    let exponent = (bits >> 24) as i32;
    let mantissa = (bits & 0x00ffffff) as f64;
    if mantissa == 0.0 {
        return 0.0;
    }
    (65535.0 * 256_f64.powi(0x1d - 3)) / (mantissa * 256_f64.powi(exponent - 3))
}

/// Resolve a block height from either a decimal height or a 64-char hex block
/// hash (display byte order). Hash lookup uses the chain_metadata 'h' + hash
/// (internal byte order) -> height mapping. Returns None if unresolvable.
fn resolve_block_height(db: &Arc<DB>, param: &str) -> Option<i32> {
    // All-digit param: parse as height directly.
    if let Ok(height) = param.parse::<i32>() {
        return Some(height);
    }

    // 64-char hex: treat as a display-order block hash and resolve to height.
    if param.len() == 64 {
        let hash_bytes = hex::decode(param).ok()?;
        if hash_bytes.len() != 32 {
            return None;
        }
        let cf_metadata = db.cf_handle("chain_metadata")?;

        // 'h' entries are keyed by internal (reversed) byte order.
        let internal_hash: Vec<u8> = hash_bytes.iter().rev().cloned().collect();
        let mut key = vec![b'h'];
        key.extend_from_slice(&internal_hash);
        if let Ok(Some(height_bytes)) = db.get_cf(&cf_metadata, &key) {
            if height_bytes.len() == 4 {
                return Some(i32::from_le_bytes([
                    height_bytes[0],
                    height_bytes[1],
                    height_bytes[2],
                    height_bytes[3],
                ]));
            }
        }

        // Some writers key 'h' entries by display byte order — try that too.
        let mut key_display = vec![b'h'];
        key_display.extend_from_slice(&hash_bytes);
        if let Ok(Some(height_bytes)) = db.get_cf(&cf_metadata, &key_display) {
            if height_bytes.len() == 4 {
                return Some(i32::from_le_bytes([
                    height_bytes[0],
                    height_bytes[1],
                    height_bytes[2],
                    height_bytes[3],
                ]));
            }
        }
    }

    None
}

/// GET /api/v2/block-index/{hashOrHeight}
/// Returns block hash for a given height, or validates a block hash exists.
/// 
/// **CACHED**: 300 second TTL for height lookups (older blocks immutable)
pub async fn block_index_v2(
    Path(param): Path<String>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<BlockHash>, (StatusCode, Json<BlockbookError>)> {
    if let Ok(height) = param.parse::<u32>() {
        // Height lookup - use cache
        let cache_key = format!("block_index:height:{height}");
        let db_clone = Arc::clone(&db);
        
        let result = cache
            .get_or_compute(
                &cache_key,
                Duration::from_secs(300),
                || async move {
                    let height_bytes = height.to_le_bytes().to_vec();
                    
                    match db_clone.cf_handle("chain_metadata") {
                        Some(cf) => {
                            match db_clone.get_cf(&cf, &height_bytes) {
                                Ok(Some(hash_bytes)) => {
                                    Ok(BlockHash {
                                        block_hash: hex::encode(&hash_bytes),
                                    })
                                },
                                Ok(None) => Err(Box::new(std::io::Error::new(
                                    std::io::ErrorKind::NotFound,
                                    format!("Block not found at height {height}")
                                )) as Box<dyn std::error::Error + Send + Sync>),
                                Err(e) => Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
                            }
                        },
                        None => Err(Box::new(std::io::Error::other(
                            "chain_metadata column family not found"
                        )) as Box<dyn std::error::Error + Send + Sync>),
                    }
                }
            )
            .await;
        
        match result {
            Ok(block_hash) => Ok(Json(block_hash)),
            Err(e) => {
                tracing::error!(error = %e, "block endpoint failed");
                Err(internal_error("Internal error"))
            }
        }
    } else if param.len() == 64 {
        // Hash validation - no cache needed (quick lookup)
        let hash_bytes = match hex::decode(&param) {
            Ok(bytes) => bytes,
            Err(_) => return Err((
                StatusCode::BAD_REQUEST,
                Json(BlockbookError::new("Invalid block hash format"))
            )),
        };
        
        if hash_bytes.len() != 32 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(BlockbookError::new("Block hash must be 32 bytes"))
            ));
        }
        
        let reversed_hash: Vec<u8> = hash_bytes.iter().rev().cloned().collect();
        
        match db.cf_handle("blocks") {
            Some(cf) => {
                match db.get_cf(&cf, &reversed_hash) {
                    Ok(Some(_)) => Ok(Json(BlockHash { block_hash: param })),
                    Ok(None) => Err(not_found(format!("Block not found with hash {param}"))),
                    Err(e) => {
                        tracing::error!(error = %e, "block index lookup failed");
                        Err(internal_error("Internal error"))
                    }
                }
            },
            None => Err(internal_error("blocks column family not found")),
        }
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            Json(BlockbookError::new("Parameter must be a block height (number) or block hash (64-char hex)"))
        ))
    }
}

/// GET /api/v2/block/{heightOrHash}
/// Returns full block details with all transactions.
///
/// Accepts EITHER a decimal block height OR a 64-char hex block hash
/// (display byte order); the hash is resolved to a height via chain_metadata.
///
/// **CACHED**: 60-300s TTL (recent blocks 60s, older blocks 300s)
pub async fn block_v2(
    Path(param): Path<String>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<crate::types::Block>, StatusCode> {
    let height = match resolve_block_height(&db, &param) {
        Some(h) => h,
        None => return Err(StatusCode::NOT_FOUND),
    };
    let cache_key = format!("block:height:{height}");
    let db_clone = Arc::clone(&db);
    
    // Determine TTL based on block age
    let chain_state = get_chain_state(&db).ok();
    let current_height = chain_state.map(|s| s.height).unwrap_or(0);
    let ttl = if height > current_height - 10 {
        Duration::from_secs(60)  // Recent blocks: 60s
    } else {
        Duration::from_secs(300) // Older blocks: 300s (immutable)
    };
    
    let result = cache
        .get_or_compute(
            &cache_key,
            ttl,
            || async move {
                compute_block_details(&db_clone, height).await
            }
        )
        .await;
    
    match result {
        Ok(block) => Ok(Json(block)),
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

async fn compute_block_details(
    db: &Arc<DB>,
    height: i32,
) -> Result<crate::types::Block, Box<dyn std::error::Error + Send + Sync>> {
    let db_clone = Arc::clone(db);
    
    tokio::task::spawn_blocking(move || {
        let height_bytes = height.to_le_bytes();
        
        // Get block hash from chain_metadata
        let cf_metadata = db_clone
            .cf_handle("chain_metadata")
            .ok_or("chain_metadata CF not found")?;
        let block_hash = db_clone
            .get_cf(&cf_metadata, height_bytes)?
            .ok_or("Block not found")?;
        
        // Get block header from blocks CF
        let cf_blocks = db_clone
            .cf_handle("blocks")
            .ok_or("blocks CF not found")?;
        let internal_hash: Vec<u8> = block_hash.iter().rev().cloned().collect();
        let header_bytes = db_clone
            .get_cf(&cf_blocks, &internal_hash)?
            .ok_or("Block header not found")?;
        
        // Parse block header
        let header = parse_block_header_sync(&header_bytes, header_bytes.len())?;
        
        // Get transaction IDs for this block
        let cf_transactions = db_clone
            .cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        let mut tx_ids = Vec::new();
        
        let mut block_tx_prefix = vec![b'B'];
        block_tx_prefix.extend_from_slice(&height_bytes);
        
        let iter = db_clone.prefix_iterator_cf(&cf_transactions, &block_tx_prefix);
        for item in iter {
            if let Ok((key, value)) = item {
                if key.len() >= 5 && &key[0..5] == block_tx_prefix.as_slice() {
                    if let Ok(txid_str) = String::from_utf8(value.to_vec()) {
                        // Stored txids are in internal (little-endian) byte order;
                        // emit canonical display-order txids (reversed) so API
                        // consumers match the node and /block-detail.
                        let display_txid = match hex::decode(&txid_str) {
                            Ok(bytes) => {
                                hex::encode(bytes.iter().rev().cloned().collect::<Vec<u8>>())
                            }
                            Err(_) => txid_str,
                        };
                        tx_ids.push(display_txid);
                    }
                } else {
                    break;
                }
            }
        }

        // Calculate difficulty from nBits (difficulty-1 target / current target).
        let difficulty = bits_to_difficulty(header.n_bits);
        
        // Get previous block hash (if not genesis)
        let previousblockhash = if header.hash_prev_block != [0u8; 32] {
            Some(hex::encode(
                header.hash_prev_block.iter().rev().cloned().collect::<Vec<u8>>()
            ))
        } else {
            None
        };
        
        Ok(crate::types::Block {
            hash: hex::encode(block_hash),
            height: height as u32,
            version: header.n_version,
            merkleroot: hex::encode(
                header.hash_merkle_root.iter().rev().cloned().collect::<Vec<u8>>()
            ),
            time: header.n_time,
            nonce: header.n_nonce,
            bits: format!("{:08x}", header.n_bits),
            difficulty,
            tx: tx_ids,
            previousblockhash,
        })
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlockStats {
    pub height: u32,
    pub hash: String,
    pub time: u64,
    pub tx_count: usize,
    pub size: usize,
    pub difficulty: f64,
}

/// GET /api/v2/block-stats/{count}
/// Returns statistics for the last N blocks.
/// 
/// **CACHED**: 60 second TTL
pub async fn block_stats_v2(
    Path(count): Path<u32>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<Vec<BlockStats>>, (StatusCode, Json<BlockbookError>)> {
    // DoS guard: each block costs several DB reads plus a per-block tx prefix scan.
    // Without a cap, a single request could walk the entire chain.
    let count = count.min(1_000);
    let cache_key = format!("block_stats:{count}");
    let db_clone = Arc::clone(&db);
    
    let result = cache
        .get_or_compute(
            &cache_key,
            Duration::from_secs(60),
            || async move {
                compute_block_stats(&db_clone, count).await
            }
        )
        .await;
    
    match result {
        Ok(stats) => Ok(Json(stats)),
        Err(e) => {
            tracing::error!(error = %e, "block stats failed");
            Err(internal_error("Internal error"))
        }
    }
}

async fn compute_block_stats(
    db: &Arc<DB>,
    count: u32,
) -> Result<Vec<BlockStats>, Box<dyn std::error::Error + Send + Sync>> {
    let db_clone = Arc::clone(db);
    
    tokio::task::spawn_blocking(move || {
        let chain_state = get_chain_state(&db_clone)
            .map_err(|e| format!("Failed to get chain state: {e}"))?;
        let tip_height = chain_state.height as u32;
        
        let mut stats = Vec::new();
        let start_height = tip_height.saturating_sub(count);
        
        let cf_metadata = db_clone
            .cf_handle("chain_metadata")
            .ok_or("chain_metadata CF not found")?;
        
        let cf_blocks = db_clone
            .cf_handle("blocks")
            .ok_or("blocks CF not found")?;
        
        let cf_transactions = db_clone
            .cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        
        for height in (start_height..=tip_height).rev() {
            let height_bytes = (height as i32).to_le_bytes();
            
            // Get block hash from chain_metadata
            let block_hash = match db_clone.get_cf(&cf_metadata, height_bytes) {
                Ok(Some(hash)) => hash,
                _ => continue,
            };
            
            let block_hash_hex = hex::encode(&block_hash);
            
            // Get block header from blocks CF (reverse the hash for internal storage)
            let internal_hash: Vec<u8> = block_hash.iter().rev().cloned().collect();
            let header_bytes = match db_clone.get_cf(&cf_blocks, &internal_hash) {
                Ok(Some(bytes)) => bytes,
                _ => continue,
            };
            
            // Parse the block header
            if let Ok(header) = parse_block_header_sync(&header_bytes, header_bytes.len()) {
                // Count transactions in the block
                let mut block_tx_prefix = vec![b'B'];
                block_tx_prefix.extend_from_slice(&height_bytes);
                
                let tx_count = db_clone
                    .prefix_iterator_cf(&cf_transactions, &block_tx_prefix)
                    .take_while(|item| {
                        if let Ok((key, _)) = item {
                            key.len() >= 5 && &key[0..5] == block_tx_prefix.as_slice()
                        } else {
                            false
                        }
                    })
                    .count();
                
                let size = header_bytes.len();
                
                // Calculate difficulty from nBits
                let difficulty = if header.n_bits != 0 {
                    let compact = header.n_bits;
                    let size = compact >> 24;
                    let word = compact & 0x00ffffff;
                    
                    let target = if size <= 3 {
                        (word >> (8 * (3 - size))) as f64
                    } else {
                        word as f64 * 256f64.powi((size - 3) as i32)
                    };
                    
                    if target > 0.0 {
                        // Max target for difficulty calculation
                        let max_target = 0x00000000ffff_u64 as f64 * 256f64.powi(0x1d - 3);
                        max_target / target
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                
                stats.push(BlockStats {
                    height,
                    hash: block_hash_hex,
                    time: header.n_time as u64,
                    tx_count,
                    size,
                    difficulty,
                });
            }
        }
        
        Ok::<Vec<BlockStats>, String>(stats)
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
    .map_err(|e| e.into())
}
