// Block-Related API Endpoints
//
// Endpoints for querying block information.
// Block data is immutable once confirmed, making it ideal for caching.

use axum::{extract::Path, http::StatusCode, Extension, Json};
use rocksdb::DB;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use super::helpers::{internal_error, not_found};
use super::types::{BlockHash, BlockbookError};
use crate::blocks::parse_block_header_sync;
use crate::cache::CacheManager;
use crate::chain_state::get_chain_state;

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
            .get_or_compute(&cache_key, Duration::from_secs(300), || async move {
                let height_bytes = height.to_le_bytes().to_vec();

                match db_clone.cf_handle("chain_metadata") {
                    Some(cf) => match db_clone.get_cf(&cf, &height_bytes) {
                        Ok(Some(hash_bytes)) => Ok(BlockHash {
                            block_hash: hex::encode(&hash_bytes),
                        }),
                        Ok(None) => Err(Box::new(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("Block not found at height {height}"),
                        ))
                            as Box<dyn std::error::Error + Send + Sync>),
                        Err(e) => Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
                    },
                    None => Err(Box::new(std::io::Error::other(
                        "chain_metadata column family not found",
                    ))
                        as Box<dyn std::error::Error + Send + Sync>),
                }
            })
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
            Err(_) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(BlockbookError::new("Invalid block hash format")),
                ))
            }
        };

        if hash_bytes.len() != 32 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(BlockbookError::new("Block hash must be 32 bytes")),
            ));
        }

        let reversed_hash: Vec<u8> = hash_bytes.iter().rev().cloned().collect();

        match db.cf_handle("blocks") {
            Some(cf) => match db.get_cf(&cf, &reversed_hash) {
                Ok(Some(_)) => Ok(Json(BlockHash { block_hash: param })),
                Ok(None) => Err(not_found(format!("Block not found with hash {param}"))),
                Err(e) => {
                    tracing::error!(error = %e, "block index lookup failed");
                    Err(internal_error("Internal error"))
                }
            },
            None => Err(internal_error("blocks column family not found")),
        }
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            Json(BlockbookError::new(
                "Parameter must be a block height (number) or block hash (64-char hex)",
            )),
        ))
    }
}

/// Blockbook's fixed page size for block transactions.
const BLOCK_TXS_PER_PAGE: usize = 1000;

#[derive(serde::Deserialize, Debug, Clone)]
pub struct BlockPageQuery {
    pub page: Option<u32>,
}

/// Blockbook v2 block shape: paginated FULL tx objects, camelCase hash links,
/// string nonce/difficulty. Replaces the old custom header+txid-list shape
/// (decision D4: dropped outright, nothing of ours consumed it).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlockbookBlock {
    pub page: u32,
    #[serde(rename = "totalPages")]
    pub total_pages: u32,
    #[serde(rename = "itemsOnPage")]
    pub items_on_page: u32,
    pub hash: String,
    #[serde(rename = "previousBlockHash", skip_serializing_if = "Option::is_none")]
    pub previous_block_hash: Option<String>,
    #[serde(rename = "nextBlockHash", skip_serializing_if = "Option::is_none")]
    pub next_block_hash: Option<String>,
    pub height: i32,
    pub confirmations: i32,
    pub size: usize,
    pub time: u32,
    pub version: u32,
    #[serde(rename = "merkleRoot")]
    pub merkle_root: String,
    pub nonce: String,
    pub bits: String,
    pub difficulty: String,
    #[serde(rename = "txCount")]
    pub tx_count: usize,
    pub txs: Vec<super::types::Transaction>,
}

/// GET /api/v2/block/{heightOrHash}?page=N
/// Blockbook-shaped block with paginated full transactions.
///
/// Accepts EITHER a decimal block height OR a 64-char hex block hash
/// (display byte order); the hash is resolved to a height via chain_metadata.
///
/// **CACHED**: 60-300s TTL (recent blocks 60s, older blocks 300s)
pub async fn block_v2(
    Path(param): Path<String>,
    axum::extract::Query(q): axum::extract::Query<BlockPageQuery>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<BlockbookBlock>, (StatusCode, Json<BlockbookError>)> {
    let height = match resolve_block_height(&db, &param) {
        Some(h) => h,
        None => return Err(not_found("Block not found")),
    };
    let page = q.page.unwrap_or(1).max(1);
    let cache_key = format!("block:bb:{height}:{page}");
    let db_clone = Arc::clone(&db);

    // Determine TTL based on block age
    let chain_state = get_chain_state(&db).ok();
    let current_height = chain_state.map(|s| s.height).unwrap_or(0);
    let ttl = if height > current_height - 10 {
        Duration::from_secs(60) // Recent blocks: 60s
    } else {
        Duration::from_secs(300) // Older blocks: 300s (immutable)
    };

    let result = cache
        .get_or_compute(&cache_key, ttl, || async move {
            compute_blockbook_block(&db_clone, height, page).await
        })
        .await;

    match result {
        Ok(block) => Ok(Json(block)),
        Err(e) => {
            // Distinguish a genuinely-absent block (404) from a storage/IO failure
            // (500). Mapping every error to 404 told clients an existing block
            // vanished on transient IO; the bare StatusCode also dropped the JSON
            // error body every other endpoint returns.
            if e.downcast_ref::<rocksdb::Error>().is_some() {
                Err(internal_error("Internal error loading block; please retry"))
            } else {
                Err(not_found("Block not found"))
            }
        }
    }
}

async fn compute_blockbook_block(
    db: &Arc<DB>,
    height: i32,
    page: u32,
) -> Result<BlockbookBlock, Box<dyn std::error::Error + Send + Sync>> {
    let db_clone = Arc::clone(db);

    // Blocking phase: header, canonical txid list, neighbor hashes.
    #[allow(clippy::type_complexity)]
    let (header_parts, tx_ids, prev_hash, next_hash, header_len): (
        (u32, String, u32, u32, u32, f64, String),
        Vec<String>,
        Option<String>,
        Option<String>,
        usize,
    ) = tokio::task::spawn_blocking(move || {
        let height_bytes = height.to_le_bytes();

        // Get block hash from chain_metadata
        let cf_metadata = db_clone
            .cf_handle("chain_metadata")
            .ok_or("chain_metadata CF not found")?;
        let block_hash = db_clone
            .get_cf(&cf_metadata, height_bytes)?
            .ok_or("Block not found")?;

        // Get block header from blocks CF
        let cf_blocks = db_clone.cf_handle("blocks").ok_or("blocks CF not found")?;
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
                header
                    .hash_prev_block
                    .iter()
                    .rev()
                    .cloned()
                    .collect::<Vec<u8>>(),
            ))
        } else {
            None
        };

        // Next block hash from the forward map (None at the tip).
        let nextblockhash = db_clone
            .get_cf(&cf_metadata, (height + 1).to_le_bytes())
            .ok()
            .flatten()
            .map(hex::encode);

        // Whole-block serialized size from EVERY tx record's length (one batched
        // read), so it is identical on every page of a paginated block.
        let mut size_keys: Vec<(&rocksdb::ColumnFamily, Vec<u8>)> =
            Vec::with_capacity(tx_ids.len() * 2);
        for txid_hex in &tx_ids {
            if let Ok(display) = hex::decode(txid_hex) {
                let internal: Vec<u8> = display.iter().rev().cloned().collect();
                let mut ik = vec![b't'];
                ik.extend_from_slice(&internal);
                let mut dk = vec![b't'];
                dk.extend_from_slice(&display);
                size_keys.push((cf_transactions, ik));
                size_keys.push((cf_transactions, dk));
            }
        }
        let mut txs_bytes = 0usize;
        for pair in db_clone.multi_get_cf(size_keys).chunks(2) {
            let len = pair
                .iter()
                .filter_map(|r| r.as_ref().ok().and_then(|v| v.as_ref()))
                .map(|v| v.len())
                .find(|l| *l > 8)
                .unwrap_or(8);
            txs_bytes += len - 8;
        }

        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((
            (
                header.n_version,
                hex::encode(
                    header
                        .hash_merkle_root
                        .iter()
                        .rev()
                        .cloned()
                        .collect::<Vec<u8>>(),
                ),
                header.n_time,
                header.n_nonce,
                header.n_bits,
                difficulty,
                hex::encode(block_hash),
            ),
            tx_ids,
            previousblockhash,
            nextblockhash,
            header_bytes.len() + txs_bytes,
        ))
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)??;

    let (version, merkle_root, time, nonce, n_bits, difficulty, hash) = header_parts;
    let tx_count = tx_ids.len();
    let total_pages = (tx_count.max(1)).div_ceil(BLOCK_TXS_PER_PAGE) as u32;
    let page = page.min(total_pages.max(1));
    let start = (page as usize - 1) * BLOCK_TXS_PER_PAGE;
    let end = (start + BLOCK_TXS_PER_PAGE).min(tx_count);
    let page_txids: Vec<String> = tx_ids[start.min(tx_count)..end].to_vec();

    // Full Blockbook tx objects via the SAME builder /tx uses (sat-string
    // money, live confirmations, sapling + special-type extras).
    let txs = crate::api::transactions::fetch_transactions_batch(db, &page_txids).await;

    // header + all tx record bytes, page-independent (var-int framing between
    // records is the only omission, same approximation /block-detail uses).
    let size = header_len;

    let current_height = get_chain_state(db).map(|s| s.height).unwrap_or(height);
    let confirmations = (current_height - height + 1).max(0);

    Ok(BlockbookBlock {
        page,
        total_pages,
        items_on_page: BLOCK_TXS_PER_PAGE as u32,
        hash,
        previous_block_hash: prev_hash,
        next_block_hash: next_hash,
        height,
        confirmations,
        size,
        time,
        version,
        merkle_root,
        nonce: nonce.to_string(),
        bits: format!("{n_bits:08x}"),
        difficulty: format!("{difficulty}"),
        tx_count,
        txs,
    })
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
        .get_or_compute(&cache_key, Duration::from_secs(60), || async move {
            compute_block_stats(&db_clone, count).await
        })
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
        let chain_state =
            get_chain_state(&db_clone).map_err(|e| format!("Failed to get chain state: {e}"))?;
        let tip_height = chain_state.height as u32;

        let mut stats = Vec::new();
        let start_height = tip_height.saturating_sub(count);

        let cf_metadata = db_clone
            .cf_handle("chain_metadata")
            .ok_or("chain_metadata CF not found")?;

        let cf_blocks = db_clone.cf_handle("blocks").ok_or("blocks CF not found")?;

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
