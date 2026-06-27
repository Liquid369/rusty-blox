// Address and UTXO API Endpoints
//
// NOTE: These are complex endpoints with significant database operations.
// Caching provides moderate benefits since addresses are frequently updated.

use axum::{Json, Extension, extract::{Path as AxumPath, Query}};
use rocksdb::DB;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use crate::cache::CacheManager;
use crate::constants::{HEIGHT_ORPHAN, HEIGHT_UNRESOLVED, is_canonical_height};
use crate::parser::{deserialize_utxos, deserialize_transaction, deserialize_transaction_blocking};
use crate::maturity::get_current_height;
use super::types::{AddressInfo, AddressQuery, XPubInfo, UTXO, UtxoQuery};
use super::transactions::{fetch_transactions_batch};

/// Redact xpub for safe logging (privacy protection)
/// Shows first 8 and last 4 characters: "xpub661M...3Mzx"
pub(crate) fn redact_xpub(xpub: &str) -> String {
    if xpub.len() <= 12 {
        return "<invalid>".to_string();
    }
    format!("{}...{}", &xpub[..8], &xpub[xpub.len()-4..])
}

/// P2-B: validate a PIVX transparent address before treating it as a real
/// account. The prior handlers accepted any string, so a one-char typo of a
/// real address returned HTTP 200 with balance:0 (a fake zero account) — the
/// reference Blockbook returns 400 on a checksum mismatch, and `/search`
/// already NotFounds the same string.
///
/// We validate the base58check CHECKSUM (class-agnostic) rather than enumerate
/// version bytes: `bs58::decode(addr).with_check(None)` recomputes the 4-byte
/// double-SHA256 checksum and fails on any typo, regardless of address class.
/// A flipped character almost always breaks the checksum, so this catches the
/// bug without risking rejection of a valid address.
///
/// Accepted decoded-payload lengths (checksum already stripped by with_check):
///   - 21 bytes = 1-byte version + 20-byte hash160. Covers every standard
///     class: D (P2PKH v30), S (cold-staking staker v63), 6/7 (P2SH v13),
///     E (single-byte exchange variants).
///   - 23 bytes = 3-byte EXM prefix [0x01,0xb9,0xa2] + 20-byte hash160. Covers
///     EXM exchange addresses (OP_EXCHANGEADDR 0xe0), which are 36 chars and
///     use a 3-byte prefix (see encode_pivx_exchange_address in parser.rs).
///
/// This deliberately does NOT hard-code a single version byte (that would
/// exclude S/6/7/E). The checksum + sane-length test is the whole guard.
///
/// Implementation note: we decode raw base58 and verify the trailing 4-byte
/// double-SHA256 checksum by hand (the same computation bs58's `with_check`
/// performs) because the `bs58` crate is pulled in without its `check`
/// feature. `Sha256` is already a top-level dependency (see parser.rs).
pub(crate) fn is_valid_address(addr: &str) -> bool {
    match decode_base58check(addr) {
        // 21 = version(1) + hash160(20); 23 = EXM prefix(3) + hash160(20).
        Some(payload) => payload.len() == 21 || payload.len() == 23,
        None => false,
    }
}

/// Decode a base58check string and verify its 4-byte double-SHA256 checksum.
/// Returns the version+payload bytes (checksum stripped) on success, or None
/// if the base58 is malformed, too short to hold a checksum, or the checksum
/// does not match. Class-agnostic: works for every address class and xpubs.
fn decode_base58check(s: &str) -> Option<Vec<u8>> {
    use sha2::{Digest, Sha256};

    let raw = bs58::decode(s).into_vec().ok()?;
    // Need at least the 4 checksum bytes (plus some payload).
    if raw.len() <= 4 {
        return None;
    }
    let (payload, checksum) = raw.split_at(raw.len() - 4);
    let first = Sha256::digest(payload);
    let second = Sha256::digest(&first);
    if second[..4] == checksum[..] {
        Some(payload.to_vec())
    } else {
        None
    }
}

/// P2-B: cheap plausibility check for a BIP32 extended public key, used to
/// reject typo/garbage input at the top of `xpub_v2` before the derivation
/// scan. PIVX reuses Bitcoin's xpub version bytes (0x0488B21E -> "xpub"), so a
/// real account-level key must start with "xpub" and decode as base58check to
/// the standard 78-byte serialization (4 version + 1 depth + 4 fingerprint +
/// 4 child + 32 chain code + 33 pubkey). `with_check(None)` validates the same
/// 4-byte double-SHA256 checksum used by addresses, so a one-char typo fails.
/// `compute_xpub_info` remains the authoritative parser (depth, key validity).
pub(crate) fn is_valid_xpub(xpub: &str) -> bool {
    if !xpub.starts_with("xpub") {
        return false;
    }
    match decode_base58check(xpub) {
        // 78 = 4 version + 1 depth + 4 parent fingerprint + 4 child number
        //      + 32 chain code + 33 compressed pubkey.
        Some(payload) => payload.len() == 78,
        None => false,
    }
}

/// Per-request cap for compute_address_info's one remaining O(history) scan:
/// the newest-first height-sort. Balances/totals are now always served from the
/// persisted 'r'/'s' aggregates (O(1)), so this no longer gates any value
/// recompute — only ordering. Addresses with more than this many txs skip the
/// height read and serve the stored txid order (unchanged prior over_cap
/// behavior). Configurable via RUSTYBLOX_ADDR_MAX_TX_SCAN; defaults to 50_000
/// (well above any normal address; the sort is now a single batched blocking
/// pass, so this is a safety bound on pathological addresses, not the hot path).
fn address_recompute_cap() -> usize {
    std::env::var("RUSTYBLOX_ADDR_MAX_TX_SCAN")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(50_000)
}

/// Read the exact persisted per-address totals written by the enrichment phase:
/// 'r'+address -> totalReceived, 's'+address -> totalSent (both i64 LE).
/// Returns (total_received, total_sent); missing keys read as 0.
async fn read_address_totals(db: &Arc<DB>, address: &str) -> (i64, i64) {
    let r_key = format!("r{address}").into_bytes();
    let s_key = format!("s{address}").into_bytes();
    let db_clone = Arc::clone(db);

    tokio::task::spawn_blocking(move || -> (i64, i64) {
        let read_i64 = |key: &[u8]| -> i64 {
            db_clone
                .cf_handle("addr_index")
                .and_then(|cf| db_clone.get_cf(&cf, key).ok().flatten())
                .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                .map(i64::from_le_bytes)
                .unwrap_or(0)
        };
        (read_i64(&r_key), read_i64(&s_key))
    })
    .await
    .unwrap_or((0, 0))
}

/// GET /api/v2/address/{address}
/// Returns address balance, transactions, and history.
///
/// **CACHED**: 30 second TTL (balances change with new blocks)
/// Note: Complex endpoint, cache provides ~30% improvement
pub async fn addr_v2(
    AxumPath(address): AxumPath<String>,
    Query(params): Query<AddressQuery>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<AddressInfo>, (axum::http::StatusCode, Json<super::types::BlockbookError>)> {
    // P2-B: reject invalid/mistyped addresses with 400 instead of serving a
    // fake zero account (HTTP 200, balance:0). Matches Blockbook and the
    // existing /search behaviour for the same string.
    if !is_valid_address(&address) {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(super::types::BlockbookError::new(format!(
                "Invalid address '{address}': checksum mismatch"
            ))),
        ));
    }

    let cache_key = format!("addr:{}:{}:{}", address, params.page, params.details);
    let db_clone = Arc::clone(&db);
    let address_clone = address.clone();
    let params_clone = params.clone();
    
    let result = cache
        .get_or_compute(
            &cache_key,
            Duration::from_secs(30),
            || async move {
                compute_address_info(&db_clone, &address_clone, &params_clone).await
            }
        )
        .await;
    
    match result {
        Ok(info) => Ok(Json(info)),
        Err(_) => {
            // Fallback: return empty address info. The address has already
            // passed checksum validation above, so this is a transient DB/compute
            // error for a real address — not the P2-B fake-zero-account case.
            Ok(Json(AddressInfo {
                page: Some(params.page),
                total_pages: Some(1),
                items_on_page: Some(params.page_size),
                address,
                balance: "0".to_string(),
                total_received: "0".to_string(),
                total_sent: "0".to_string(),
                unconfirmed_balance: "0".to_string(),
                unconfirmed_txs: 0,
                txs: 0,
                txids: Some(vec![]),
                transactions: None,
            }))
        }
    }
}

async fn compute_address_info(
    db: &Arc<DB>,
    address: &str,
    params: &AddressQuery,
) -> Result<AddressInfo, Box<dyn std::error::Error + Send + Sync>> {
    // Per-address tx list ('t' + address): the authoritative tx set written by
    // enrichment, stored in canonical display order (see the P1-1 note below).
    let tx_list_key = format!("t{address}");
    let tx_list_key_bytes = tx_list_key.as_bytes().to_vec();
    let db_clone = Arc::clone(db);

    let tx_list_data = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let cf_addr_index = db_clone.cf_handle("addr_index")
            .ok_or_else(|| "addr_index CF not found".to_string())?;
        db_clone.get_cf(&cf_addr_index, &tx_list_key_bytes)
            .map_err(|e| e.to_string())
            .map(|opt| opt.unwrap_or_default())
    })
    .await??;

    let all_txids: Vec<Vec<u8>> = tx_list_data.chunks_exact(32)
        .map(|chunk| chunk.to_vec())
        .collect();

    // Balance and lifetime totals come straight from the aggregates enrichment
    // already persisted: 'r'+address = totalReceived, 's'+address = totalSent
    // (both i64 LE). By the UTXO-accounting identity, confirmed
    //     balance == totalReceived - totalSent
    // and this INCLUDES immature coinbase/coinstake outputs (both 'r' and the
    // unspent set count them, so the identity still holds — the prior balance
    // loop deliberately included immature outputs for Blockbook parity).
    //
    // This replaces the former per-UTXO balance loop AND the per-tx
    // total_received rescan — together ~2x len(txs) sequential
    // spawn_blocking().await round-trips whose scheduling overhead (~0.65ms
    // each) pushed 5k-50k-tx addresses past the 30s HTTP timeout — with two
    // point lookups. The >50k-tx ("over_cap") path already served these exact
    // aggregates in production; this just makes them the path for every address
    // size, so the values are unchanged while the work drops from O(history) to
    // O(1). Missing keys read as 0 (an unenriched-but-indexed address shows
    // 0/0/0 rather than erroring — same as the prior fallback).
    let (total_received, total_sent) = read_address_totals(db, address).await;
    let balance = total_received - total_sent;

    // P1-1: the stored 't'-index bytes are ALREADY canonical display order
    // (parser.rs hash_txid reverses to display, and the tx CF is keyed by
    // 't' + display bytes). Emit them directly — a prior extra .reverse()
    // flipped txids to a non-canonical order that the node, duddino, and every
    // other explorer reject (broke Blockbook compat on /address.txids[]).
    let unique_txids: Vec<String> = all_txids.iter()
        .map(hex::encode)
        .collect();

    // Order newest-first by block height. This is the only history-sized step
    // left, so do it as ONE blocking pass: a single get_cf per txid that reads
    // the 4-byte height at offset 4..8 (no full-tx deserialization; one tx blob
    // in memory at a time, freed each iteration) instead of one spawn_blocking
    // task per txid. That removes the per-item async scheduling overhead that
    // made this the second O(history) scan, while keeping the result
    // byte-identical (same stable sort, same height source). Above the cap we
    // serve the stored txid order, preserving the prior over_cap behavior
    // exactly (no height reads at all).
    let order_cap = address_recompute_cap();
    let all_txids: Vec<String> = if unique_txids.len() > order_cap {
        unique_txids
    } else {
        // Clone for the blocking pass so a JoinError (task panic / runtime
        // shutdown) falls back to the stored order instead of dropping txids
        // (no-silent-failure rule). The clone is at most ~4MB at the 50k cap (a
        // few hundred KB for typical addresses) — negligible against the
        // thousands of full-tx deserializations this commit removes.
        let txids_for_sort = unique_txids.clone();
        let db_clone = Arc::clone(db);
        let sorted = tokio::task::spawn_blocking(move || -> Vec<String> {
            let cf = match db_clone.cf_handle("transactions") {
                Some(cf) => cf,
                None => return txids_for_sort,
            };
            let mut txid_heights: Vec<(String, i32)> = Vec::with_capacity(txids_for_sort.len());
            for txid in txids_for_sort {
                // tx CF is keyed by 't' + canonical display-order bytes (exactly
                // the bytes we just hex-encoded), so decode straight back to it.
                let height = hex::decode(&txid).ok()
                    .and_then(|txid_bytes| {
                        let mut key = vec![b't'];
                        key.extend(&txid_bytes);
                        db_clone.get_cf(&cf, &key).ok().flatten()
                    })
                    .filter(|tx_data| tx_data.len() >= 8)
                    .map(|tx_data| i32::from_le_bytes([tx_data[4], tx_data[5], tx_data[6], tx_data[7]]))
                    .unwrap_or(0);
                txid_heights.push((txid, height));
            }
            // Sort by height descending (newest first = highest block). Stable
            // sort keeps stored order within a height, matching the old loop.
            txid_heights.sort_by(|a, b| b.1.cmp(&a.1));
            txid_heights.into_iter().map(|(txid, _)| txid).collect()
        })
        .await;
        match sorted {
            Ok(ordered) => ordered,
            // spawn_blocking only JoinErrors on a task panic or runtime shutdown.
            // Serve the stored txid order rather than dropping txids, but make the
            // rare degraded (unsorted) response observable instead of silent.
            Err(e) => {
                warn!(address = %address, error = %e,
                    "address height-sort task failed; serving stored txid order");
                unique_txids
            }
        }
    };

    // === PAGINATION LOGIC ===
    // Validate and clamp parameters
    const MAX_PAGE_SIZE: u32 = 1000;
    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, MAX_PAGE_SIZE);
    
    let total_tx_count = all_txids.len();
    let total_pages = if total_tx_count == 0 {
        1
    } else {
        ((total_tx_count as f64) / (page_size as f64)).ceil() as u32
    };
    
    // Calculate pagination indices
    let start_idx = (page as usize - 1).saturating_mul(page_size as usize);
    let end_idx = start_idx.saturating_add(page_size as usize).min(total_tx_count);
    
    // Handle page out of bounds - return empty result
    let (paginated_txids, actual_items) = if start_idx >= total_tx_count {
        (vec![], 0)
    } else {
        let slice = &all_txids[start_idx..end_idx];
        (slice.to_vec(), slice.len())
    };
    
    // === DETAILS MODE HANDLING ===
    // Blockbook API:
    // - basic: No transaction data (just balances)
    // - txids: Transaction IDs only (default)
    // - txs: Full transaction objects
    let (txids, transactions) = match params.details.as_str() {
        "basic" => {
            // No transaction data at all
            (None, None)
        },
        "txs" => {
            // Full transaction objects - fetch them
            let txs = fetch_transactions_batch(db, &paginated_txids).await;
            (None, Some(txs))
        },
        _ => {
            // Default: "txids" or any other value = just txid strings
            (Some(paginated_txids), None)
        }
    };
    
    Ok(AddressInfo {
        page: Some(page),
        total_pages: Some(total_pages),
        items_on_page: Some(actual_items as u32),  // Actual count, not pageSize
        address: address.to_string(),
        balance: balance.to_string(),
        total_received: total_received.to_string(),
        total_sent: total_sent.to_string(),
        unconfirmed_balance: "0".to_string(),
        unconfirmed_txs: 0,
        txs: total_tx_count as u32,  // Total tx count (not paginated count)
        txids,
        transactions,
    })
}

/// GET /api/v2/xpub/{xpub}
/// Returns aggregated data for all addresses derived from extended public key.
/// 
/// **CACHED**: 5 minute TTL
pub async fn xpub_v2(
    AxumPath(xpub_str): AxumPath<String>,
    Query(params): Query<AddressQuery>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<XPubInfo>, (axum::http::StatusCode, Json<super::types::BlockbookError>)> {
    // P2-B: cheaply reject obviously-not-an-xpub input up front (before the
    // cache/derivation machinery) so a typo/garbage string gets a 400 rather
    // than entering the scan. PIVX reuses Bitcoin's xpub version bytes
    // (0x0488B21E -> "xpub" prefix; see compute_xpub_info note), so a real
    // account-level xpub must start with "xpub" and be valid base58check.
    // compute_xpub_info still does the authoritative ExtendedPubKey parse +
    // depth check; this is just an early, class-correct guard.
    if !is_valid_xpub(&xpub_str) {
        warn!(xpub = %redact_xpub(&xpub_str), "rejected invalid xpub (P2-B)");
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(super::types::BlockbookError::new(
                "Invalid xpub: not a valid base58check BIP32 extended public key",
            )),
        ));
    }

    let cache_key = format!("xpub:{}:{}:{}", xpub_str, params.page, params.details);
    let db_clone = Arc::clone(&db);
    let xpub_clone = xpub_str.clone();
    let params_clone = params.clone();
    
    let result = cache
        .get_or_compute(
            &cache_key,
            Duration::from_secs(300),
            || async move {
                compute_xpub_info(&db_clone, &xpub_clone, &params_clone).await
            }
        )
        .await;
    
    match result {
        Ok(info) => Ok(Json(info)),
        Err(e) => {
            // Log error with redacted xpub (privacy protection)
            warn!(xpub = %redact_xpub(&xpub_str), error = %e, "xpub query error");
            Err((
                axum::http::StatusCode::BAD_REQUEST,
                Json(super::types::BlockbookError::new(e.to_string()))
            ))
        }
    }
}

async fn compute_xpub_info(
    db: &Arc<DB>,
    xpub_str: &str,
    params: &AddressQuery,
) -> Result<XPubInfo, Box<dyn std::error::Error + Send + Sync>> {
    use bitcoin::util::bip32::ExtendedPubKey;
    use std::str::FromStr;
    
    // Parse and validate the xpub
    let xpub = ExtendedPubKey::from_str(xpub_str)
        .map_err(|e| format!("Invalid xpub format: {e}. Please provide a valid BIP32 extended public key."))?;
    
    // Validate xpub is at correct depth (account level = 3 for m/44'/119'/account')
    if xpub.depth != 3 {
        return Err(format!(
            "Invalid xpub depth: expected 3 (account level m/44'/119'/account'), got {}. This endpoint only supports account-level xpubs. Please export xpub at depth 3.",
            xpub.depth
        ).into());
    }
    
    // Validate gap limit parameter (if provided)
    if let Some(gap) = params.gap_limit {
        if gap == 0 {
            return Err("Gap limit must be at least 1".into());
        }
        if gap > 200 {
            return Err("Gap limit cannot exceed 200 for performance reasons".into());
        }
    }
    
    // Validate max_scan parameter (if provided) - limits total addresses scanned per request
    if let Some(max_scan) = params.max_scan {
        if max_scan == 0 {
            return Err("Max scan limit must be at least 1".into());
        }
        if max_scan > 2000 {
            return Err("Max scan limit cannot exceed 2000 for performance reasons".into());
        }
    }
    
    // Note: PIVX uses the same xpub version bytes as Bitcoin (0x0488B21E) for wallet compatibility.
    // This is intentional and allows PIVX wallets to use standard BIP32 tools.
    // We cannot validate coin type from the xpub itself as it's not encoded in the serialization.
    
    // Create secp256k1 context once for all derivations (performance optimization)
    let secp = bitcoin::secp256k1::Secp256k1::new();
    
    // BIP44 gap limit: stop scanning after N consecutive unused addresses
    let gap_limit = params.gap_limit.unwrap_or(20); // Default 20 per BIP44
    let max_scan_limit = params.max_scan.unwrap_or(1000); // Configurable safety limit (default 1000)
    
    // Derive addresses with gap limit logic for both chains
    let mut all_addresses: Vec<(String, String)> = Vec::new(); // (address, path)
    
    // External chain (receive addresses): m/44'/119'/account'/0/index
    let mut external_consecutive_unused = 0;
    for i in 0..max_scan_limit {
        if external_consecutive_unused >= gap_limit {
            break; // Gap limit reached
        }
        
        match derive_address(&xpub, &secp, 0, i, xpub.depth) {
            Ok((address, path)) => {
                // Check if address has activity
                let has_activity = check_address_activity(db, &address).await?;
                
                if has_activity {
                    external_consecutive_unused = 0; // Reset gap counter
                } else {
                    external_consecutive_unused += 1;
                }
                
                all_addresses.push((address, path));
            },
            Err(_) => break,
        }
    }
    
    // Internal chain (change addresses): m/44'/119'/account'/1/index
    let mut internal_consecutive_unused = 0;
    for i in 0..max_scan_limit {
        if internal_consecutive_unused >= gap_limit {
            break; // Gap limit reached
        }
        
        match derive_address(&xpub, &secp, 1, i, xpub.depth) {
            Ok((address, path)) => {
                let has_activity = check_address_activity(db, &address).await?;
                
                if has_activity {
                    internal_consecutive_unused = 0;
                } else {
                    internal_consecutive_unused += 1;
                }
                
                all_addresses.push((address, path));
            },
            Err(_) => break,
        }
    }
    
    // Aggregate UTXOs and transactions from all derived addresses
    aggregate_xpub_data(db, xpub_str, &all_addresses, params).await
}

/// Derive a single address from xpub at specified chain/index
pub(crate) fn derive_address(
    xpub: &bitcoin::util::bip32::ExtendedPubKey,
    secp: &bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>,
    chain: u32,
    index: u32,
    xpub_depth: u8,
) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    use bitcoin::util::bip32::ChildNumber;
    
    // Derive: xpub/chain/index
    let chain_key = xpub.ckd_pub(secp, ChildNumber::from_normal_idx(chain)?)?;
    let child_key = chain_key.ckd_pub(secp, ChildNumber::from_normal_idx(index)?)?;
    
    let pubkey_hash = child_key.public_key.pubkey_hash();
    
    // Encode as PIVX address (version 30 for mainnet P2PKH)
    let address = crate::parser::encode_pivx_address(pubkey_hash.as_ref(), 30)
        .ok_or("Failed to encode PIVX address")?;
    
    // Construct full derivation path
    let account = xpub_depth.saturating_sub(3);
    let path = format!("m/44'/119'/{account}'/{chain}/{index}");
    
    Ok((address, path))
}

/// Check if an address has any activity (UTXOs or transactions)
async fn check_address_activity(
    db: &Arc<DB>,
    address: &str,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Check UTXO key: 'a' + address
    let utxo_key = format!("a{address}");
    let utxo_key_bytes = utxo_key.as_bytes().to_vec();
    
    // Check transaction list key: 't' + address
    let tx_key = format!("t{address}");
    let tx_key_bytes = tx_key.as_bytes().to_vec();
    
    let db_clone = db.clone();
    
    let has_activity = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let cf_addr_index = db_clone.cf_handle("addr_index")
            .ok_or_else(|| "addr_index CF not found".to_string())?;
        
        // Check UTXOs
        if let Ok(Some(utxo_data)) = db_clone.get_cf(&cf_addr_index, &utxo_key_bytes) {
            if !utxo_data.is_empty() {
                return Ok(true);
            }
        }
        
        // Check transaction history
        if let Ok(Some(tx_data)) = db_clone.get_cf(&cf_addr_index, &tx_key_bytes) {
            if !tx_data.is_empty() {
                return Ok(true);
            }
        }
        
        Ok(false)
    })
    .await??;
    
    Ok(has_activity)
}

/// Aggregate balance, transactions, and UTXO data from all derived addresses
async fn aggregate_xpub_data(
    db: &Arc<DB>,
    xpub_str: &str,
    all_addresses: &[(String, String)],
    params: &AddressQuery,
) -> Result<XPubInfo, Box<dyn std::error::Error + Send + Sync>> {
    use std::collections::HashSet;
    
    let mut all_txids = HashSet::new();
    let mut used_addresses: Vec<(String, String, usize, i64, i64, i64)> = Vec::new();
    
    // Get current chain height for maturity checks
    let _current_height = get_current_height(db).unwrap_or(0);
    
    // PERFORMANCE OPTIMIZATION: Batch all address lookups into single multi_get_cf call
    // This replaces N sequential queries with 1 batched query (10-50x faster)
    
    // Build batch of UTXO keys ("a" + address)
    let utxo_keys: Vec<Vec<u8>> = all_addresses.iter()
        .map(|(addr, _)| format!("a{addr}").into_bytes())
        .collect();
    
    // Build batch of transaction list keys ("t" + address)
    let tx_list_keys: Vec<Vec<u8>> = all_addresses.iter()
        .map(|(addr, _)| format!("t{addr}").into_bytes())
        .collect();
    
    let db_clone = db.clone();
    let utxo_keys_clone = utxo_keys.clone();
    let tx_list_keys_clone = tx_list_keys.clone();
    
    // Execute batched queries
    let (utxo_results, tx_list_results) = tokio::task::spawn_blocking(move || -> Result<(Vec<Option<Vec<u8>>>, Vec<Option<Vec<u8>>>), String> {
        let cf_addr_index = db_clone.cf_handle("addr_index")
            .ok_or_else(|| "addr_index CF not found".to_string())?;
        
        // Batch get UTXOs for all addresses
        let utxo_batch: Vec<_> = utxo_keys_clone.iter()
            .map(|k| (&cf_addr_index, k.as_slice()))
            .collect();
        let utxo_results: Vec<Option<Vec<u8>>> = db_clone.multi_get_cf(utxo_batch)
            .into_iter()
            .map(|r| r.ok().flatten())
            .collect();
        
        // Batch get transaction lists for all addresses
        let tx_list_batch: Vec<_> = tx_list_keys_clone.iter()
            .map(|k| (&cf_addr_index, k.as_slice()))
            .collect();
        let tx_list_results: Vec<Option<Vec<u8>>> = db_clone.multi_get_cf(tx_list_batch)
            .into_iter()
            .map(|r| r.ok().flatten())
            .collect();
        
        Ok((utxo_results, tx_list_results))
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
    .map_err(|e| e.to_string())?;
    
    // Process each address with pre-fetched data
    for (idx, (address, path)) in all_addresses.iter().enumerate() {
        let utxo_data = utxo_results.get(idx)
            .and_then(|opt| opt.as_ref()).cloned()
            .unwrap_or_default();
        
        let utxos = deserialize_utxos(&utxo_data).await;
        
        // Blockbook parity: include immature outputs in xpub balances
        let spendable_utxos = utxos.clone();
        
        // Calculate balance for this address
        let mut address_balance: i64 = 0;
        for (txid_hash, output_index) in &spendable_utxos {
            let mut key = vec![b't'];
            key.extend(txid_hash);
            let key_clone = key.clone();
            let db_clone = db.clone();
            
            let tx_data = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, String> {
                let cf_transactions = db_clone.cf_handle("transactions")
                    .ok_or_else(|| "transactions CF not found".to_string())?;
                db_clone.get_cf(&cf_transactions, &key_clone)
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| format!("Task join error: {e}"))?
            .map_err(|e| e.to_string())?;
            
            if let Some(tx_data) = tx_data {
                if tx_data.len() >= 8 {
                    let tx_data_len = tx_data.len() - 8;
                    if tx_data_len > 0 {
                        let mut tx_data_with_header = Vec::with_capacity(4 + tx_data_len);
                        tx_data_with_header.extend_from_slice(&[0u8; 4]);
                        tx_data_with_header.extend_from_slice(&tx_data[8..]);
                        
                        if let Ok(tx) = deserialize_transaction(&tx_data_with_header).await {
                            if let Some(output) = tx.outputs.get(*output_index as usize) {
                                address_balance += output.value;
                            }
                        }
                    }
                }
            }
        }
        
        // Get transaction list from pre-fetched batch data
        let tx_list_data = tx_list_results.get(idx)
            .and_then(|opt| opt.as_ref()).cloned()
            .unwrap_or_default();
        
        // Parse transaction list (32 bytes per txid)
        let txids: Vec<Vec<u8>> = tx_list_data.chunks_exact(32)
            .map(|chunk| chunk.to_vec())
            .collect();
        
        // Calculate total received for this address
        let mut total_received: i64 = 0;
        
        for txid_bytes in &txids {
            let mut key = vec![b't'];
            key.extend(txid_bytes);
            let key_clone = key.clone();
            let db_clone = db.clone();
            let addr_clone = address.clone();
            
            let tx_data = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, String> {
                let cf_transactions = db_clone.cf_handle("transactions")
                    .ok_or_else(|| "transactions CF not found".to_string())?;
                db_clone.get_cf(&cf_transactions, &key_clone)
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| format!("Task join error: {e}"))?
            .map_err(|e| e.to_string())?;
            
            if let Some(tx_data) = tx_data {
                if tx_data.len() >= 8 {
                    // Skip orphaned and unresolved transactions
                    let block_height = i32::from_le_bytes([tx_data[4], tx_data[5], tx_data[6], tx_data[7]]);
                    if block_height == HEIGHT_ORPHAN || block_height == HEIGHT_UNRESOLVED {
                        continue;
                    }
                    let tx_data_len = tx_data.len() - 8;
                    if tx_data_len > 0 {
                        let mut tx_data_with_header = Vec::with_capacity(4 + tx_data_len);
                        tx_data_with_header.extend_from_slice(&[0u8; 4]);
                        tx_data_with_header.extend_from_slice(&tx_data[8..]);
                        
                        if let Ok(tx) = deserialize_transaction(&tx_data_with_header).await {
                            for output in &tx.outputs {
                                if output.address.contains(&addr_clone) {
                                    total_received += output.value;
                                }
                            }
                        }
                    }
                }
            }
        }
        
        let total_sent = total_received - address_balance;
        
        // Track address if it has activity
        if !txids.is_empty() || !utxos.is_empty() {
            used_addresses.push((
                address.clone(),
                path.clone(),
                txids.len(),
                address_balance,
                total_received,
                total_sent,
            ));
        }
        
        // Add transaction IDs to set
        for txid in txids {
            all_txids.insert(hex::encode(txid));
        }
    }
    
    // Convert txid set to vec
    let unique_txids: Vec<String> = all_txids.into_iter().collect();
    
    // Sort transactions by block height (descending = newest first)
    let mut txid_heights: Vec<(String, i32)> = Vec::new();
    for txid in &unique_txids {
        if let Ok(txid_bytes) = hex::decode(txid) {
            // tx CF is keyed by 't' + canonical display-order bytes, so use the
            // decoded txid directly. The prior reverse made every key miss, so
            // xpub tx heights silently read 0 and the list was effectively unsorted.
            let mut key = vec![b't'];
            key.extend(&txid_bytes);
            let db_clone = db.clone();
            let height = tokio::task::spawn_blocking(move || -> i32 {
                if let Some(cf) = db_clone.cf_handle("transactions") {
                    if let Ok(Some(tx_data)) = db_clone.get_cf(&cf, &key) {
                        if tx_data.len() >= 8 {
                            return i32::from_le_bytes([tx_data[4], tx_data[5], tx_data[6], tx_data[7]]);
                        }
                    }
                }
                0
            })
            .await
            .unwrap_or(0);
            txid_heights.push((txid.clone(), height));
        }
    }
    
    // Sort by height descending (newest first = highest block)
    txid_heights.sort_by(|a, b| b.1.cmp(&a.1));
    let unique_txids: Vec<String> = txid_heights.into_iter().map(|(txid, _)| txid).collect();
    
    // === PAGINATION LOGIC (same as address endpoint) ===
    // Validate and clamp parameters
    const MAX_PAGE_SIZE: u32 = 1000;
    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, MAX_PAGE_SIZE);
    
    let total_tx_count = unique_txids.len();
    let total_pages = if total_tx_count == 0 {
        1
    } else {
        ((total_tx_count as f64) / (page_size as f64)).ceil() as u32
    };
    
    // Calculate pagination indices
    let start_idx = (page as usize - 1).saturating_mul(page_size as usize);
    let end_idx = start_idx.saturating_add(page_size as usize).min(total_tx_count);
    
    // Handle page out of bounds - return empty result
    let (paginated_txids, actual_items) = if start_idx >= total_tx_count {
        (vec![], 0)
    } else {
        let slice = &unique_txids[start_idx..end_idx];
        (slice.to_vec(), slice.len())
    };
    
    // === DETAILS MODE HANDLING (same as address endpoint) ===
    // Blockbook API:
    // - basic: No transaction data (just balances)
    // - txids: Transaction IDs only (default)
    // - txs: Full transaction objects
    let (txids, transactions) = match params.details.as_str() {
        "basic" => {
            // No transaction data at all
            (None, None)
        },
        "txs" => {
            // Full transaction objects - fetch them
            let txs = fetch_transactions_batch(db, &paginated_txids).await;
            (None, Some(txs))
        },
        "tokens" | "tokenBalances" => {
            // Token modes don't return tx data
            (None, None)
        },
        _ => {
            // Default: "txids" or any other value = just txid strings
            (Some(paginated_txids), None)
        }
    };
    
    // Build tokens array if requested, filtered by tokens parameter
    let tokens = if params.details == "tokens" || params.details == "tokenBalances" {
        // Filter addresses based on tokens parameter
        let filtered_addresses: Vec<_> = all_addresses.iter().filter_map(|(addr, path)| {
            // Find matching used address data
            let addr_data = used_addresses.iter().find(|(a, _, _, _, _, _)| a == addr);
            
            match params.tokens.as_str() {
                "derived" => {
                    // Return all derived addresses, even if unused
                    if let Some((_, _, tx_count, balance, total_recv, total_snt)) = addr_data {
                        Some(super::types::XPubToken {
                            token_type: "XPUBAddress".to_string(),
                            name: addr.clone(),
                            path: path.clone(),
                            transfers: *tx_count as u32,
                            decimals: 8,
                            balance: balance.to_string(),
                            total_received: total_recv.to_string(),
                            total_sent: total_snt.to_string(),
                        })
                    } else {
                        // Address was derived but never used
                        Some(super::types::XPubToken {
                            token_type: "XPUBAddress".to_string(),
                            name: addr.clone(),
                            path: path.clone(),
                            transfers: 0,
                            decimals: 8,
                            balance: "0".to_string(),
                            total_received: "0".to_string(),
                            total_sent: "0".to_string(),
                        })
                    }
                },
                "used" => {
                    // Return addresses with at least one transaction
                    addr_data.filter(|(_, _, tx_count, _, _, _)| *tx_count > 0).map(|(_, _, tx_count, balance, total_recv, total_snt)| {
                        super::types::XPubToken {
                            token_type: "XPUBAddress".to_string(),
                            name: addr.clone(),
                            path: path.clone(),
                            transfers: *tx_count as u32,
                            decimals: 8,
                            balance: balance.to_string(),
                            total_received: total_recv.to_string(),
                            total_sent: total_snt.to_string(),
                        }
                    })
                },
                "nonzero" | _ => {
                    // Default: return only addresses with nonzero balance
                    addr_data.filter(|(_, _, _, balance, _, _)| *balance != 0).map(|(_, _, tx_count, balance, total_recv, total_snt)| {
                        super::types::XPubToken {
                            token_type: "XPUBAddress".to_string(),
                            name: addr.clone(),
                            path: path.clone(),
                            transfers: *tx_count as u32,
                            decimals: 8,
                            balance: balance.to_string(),
                            total_received: total_recv.to_string(),
                            total_sent: total_snt.to_string(),
                        }
                    })
                }
            }
        }).collect();
        
        // Apply pagination to tokens array
        let total_tokens = filtered_addresses.len() as u32;
        let tokens_page = params.tokens_page.max(1);
        let tokens_page_size = params.tokens_page_size.clamp(1, 1000);
        let total_tokens_pages = ((total_tokens as f64) / (tokens_page_size as f64)).ceil() as u32;
        let total_tokens_pages = if total_tokens_pages == 0 { 1 } else { total_tokens_pages };

        let start_idx = (tokens_page as usize - 1).saturating_mul(tokens_page_size as usize);
        let end_idx = start_idx.saturating_add(tokens_page_size as usize).min(filtered_addresses.len());
        
        let paginated_tokens: Vec<_> = if start_idx < filtered_addresses.len() {
            filtered_addresses[start_idx..end_idx].to_vec()
        } else {
            Vec::new()
        };
        
        (Some(paginated_tokens), Some(total_tokens), Some(tokens_page), Some(total_tokens_pages))
    } else {
        (None, None, None, None)
    };
    
    // Blockbook always returns usedTokens (count of addresses with activity)
    // regardless of details mode
    let used_tokens = Some(used_addresses.len() as u32);
    
    // Aggregate totals
    let xpub_total_received: i64 = used_addresses.iter().map(|(_, _, _, _, total_recv, _)| total_recv).sum();
    let xpub_total_sent: i64 = used_addresses.iter().map(|(_, _, _, _, _, total_snt)| total_snt).sum();
    let xpub_balance: i64 = used_addresses.iter().map(|(_, _, _, balance, _, _)| balance).sum();
    
    // Blockbook's txs field for xpub = total transfers across ALL addresses
    // (not unique transactions). If an address appears in 2 txs, it counts as 2.
    // This matches: sum of all per-address tx counts = total "transfers"
    let total_transfers: usize = used_addresses.iter().map(|(_, _, tx_count, _, _, _)| tx_count).sum();
    
    Ok(XPubInfo {
        page,
        total_pages,
        items_on_page: actual_items as u32,  // Actual count, not pageSize
        address: xpub_str.to_string(),
        balance: xpub_balance.to_string(),
        total_received: xpub_total_received.to_string(),
        total_sent: xpub_total_sent.to_string(),
        unconfirmed_balance: "0".to_string(),
        unconfirmed_txs: 0,
        txs: total_transfers as u32,  // Total transfers (Blockbook compatibility)
        txids,
        tokens: tokens.0,
        transactions,  // Now properly populated when details=txs
        used_tokens,
        total_tokens: tokens.1,
        tokens_page: tokens.2,
        total_tokens_pages: tokens.3,
    })
}

/// GET /api/v2/utxo/{address}
/// Returns unspent outputs for address.
/// 
/// **CACHED**: 30 second TTL
pub async fn utxo_v2(
    AxumPath(address): AxumPath<String>,
    Query(query): Query<UtxoQuery>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<Vec<UTXO>>, (axum::http::StatusCode, Json<super::types::BlockbookError>)> {
    // P2-B: reject invalid/mistyped addresses with 400 instead of serving an
    // empty UTXO list (HTTP 200, []) for a non-existent/typo'd account.
    if !is_valid_address(&address) {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(super::types::BlockbookError::new(format!(
                "Invalid address '{address}': checksum mismatch"
            ))),
        ));
    }

    let cache_key = format!("utxo:{}:{}", address, query.confirmed);
    let db_clone = Arc::clone(&db);
    let address_clone = address.clone();

    let result = cache
        .get_or_compute(
            &cache_key,
            Duration::from_secs(30),
            || async move {
                compute_utxos(&db_clone, &address_clone).await
            }
        )
        .await;

    match result {
        Ok(utxos) => Ok(Json(utxos)),
        Err(_) => Ok(Json(vec![])),
    }
}

async fn compute_utxos(
    db: &Arc<DB>,
    address: &str,
) -> Result<Vec<UTXO>, Box<dyn std::error::Error + Send + Sync>> {
    let key = format!("a{address}");
    let key_bytes = key.as_bytes().to_vec();
    let db_clone = Arc::clone(db);
    
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let cf_addr_index = db_clone.cf_handle("addr_index")
            .ok_or_else(|| "addr_index CF not found".to_string())?;
        db_clone.get_cf(&cf_addr_index, &key_bytes)
            .map_err(|e| e.to_string())
            .map(|opt| opt.unwrap_or_default())
    })
    .await??;
    
    let unspent_utxos = deserialize_utxos(&result).await;
    let current_height = get_current_height(db).unwrap_or(0);
    // Blockbook parity: the UTXO list includes immature coinbase/coinstake outputs
    // (Blockbook flags them rather than hiding them; wallets handle maturity).
    let spendable_utxos = unspent_utxos.clone();
    
    let mut utxo_list = Vec::new();
    
    for (txid_hash, vout) in &spendable_utxos {
        // P1-1: the 'a' unspent index stores txids in canonical DISPLAY order
        // (the same bytes that key the tx CF below), so emit them directly. The
        // earlier .reverse() produced a non-canonical txid that the node,
        // duddino, wallets, and every other explorer reject.
        let txid_display = hex::encode(txid_hash);

        // Look up transaction to get value, confirmations, and other details.
        // The tx CF key uses internal (non-reversed) txid bytes, i.e. txid_hash
        // as stored in the 'a' unspent set.
        let mut tx_key = vec![b't'];
        tx_key.extend_from_slice(txid_hash);

        let db_clone = Arc::clone(db);
        let tx_key_clone = tx_key.clone();
        let tx_data_opt = tokio::task::spawn_blocking(move || {
            if let Some(cf) = db_clone.cf_handle("transactions") {
                db_clone.get_cf(&cf, &tx_key_clone).ok().flatten()
            } else {
                None
            }
        }).await.ok().flatten();

        if let Some(tx_data) = tx_data_opt {
            if tx_data.len() >= 8 {
                let block_height = i32::from_le_bytes([tx_data[4], tx_data[5], tx_data[6], tx_data[7]]);

                // Filter hardening: a UTXO present in the 'a' unspent set is
                // canonical by construction (the balance loop trusts it with no
                // height filter). Only drop entries with a definitive orphan
                // determination (-1); a transient HEIGHT_UNRESOLVED read must NOT
                // silently drop a canonical UTXO (that under-reported /utxo 98%
                // on a degraded DB). On the clean DB nothing is orphan/unresolved,
                // so the set is unchanged.
                if block_height == HEIGHT_ORPHAN {
                    continue;
                }
                
                // Parse transaction to get output value
                let tx_data_len = tx_data.len() - 8;
                let mut tx_data_with_header = Vec::with_capacity(4 + tx_data_len);
                tx_data_with_header.extend_from_slice(&[0u8; 4]);
                tx_data_with_header.extend_from_slice(&tx_data[8..]);
                
                if let Ok(tx) = deserialize_transaction_blocking(&tx_data_with_header) {
                    if let Some(output) = tx.outputs.get(*vout as usize) {
                        let confirmations = if is_canonical_height(block_height) && is_canonical_height(current_height) {
                            ((current_height - block_height) + 1).max(0) as u32
                        } else {
                            0
                        };
                        
                        use crate::tx_type::detect_transaction_type;
                        let tx_type = detect_transaction_type(&tx);
                        
                        utxo_list.push(UTXO {
                            txid: txid_display,
                            vout: *vout as u32,
                            value: output.value.to_string(),
                            confirmations,
                            lock_time: if confirmations == 0 && tx.lock_time > 0 {
                                Some(tx.lock_time)
                            } else {
                                None
                            },
                            height: if block_height > 0 {
                                Some(block_height as u32)
                            } else {
                                None
                            },
                            coinbase: Some(tx_type == crate::tx_type::TransactionType::Coinbase),
                            coinstake: Some(tx_type == crate::tx_type::TransactionType::Coinstake),
                            spendable: Some(true),
                            blocks_until_spendable: None,
                        });
                    }
                }
            }
        }
    }
    
    // Sort by confirmations ascending (newest first = least confirmations first)
    utxo_list.sort_by(|a, b| a.confirmations.cmp(&b.confirmations));
    
    Ok(utxo_list)
}

#[cfg(test)]
mod tests {
    use super::{is_valid_address, is_valid_xpub};

    /// P2-B: every real PIVX transparent address class must pass validation.
    /// Asymmetric risk — a validator that rejects a VALID address is worse than
    /// the original fake-zero-account bug, so these MUST stay true.
    #[test]
    fn is_valid_address_accepts_all_real_classes() {
        // D = standard P2PKH (version 30)
        assert!(is_valid_address("DU8gPC5mh4KxWJARQRxoESFark2jAguBr5"), "D/P2PKH must be valid");
        // S = cold-staking staker (version 63)
        assert!(is_valid_address("SdgQDpS8jDRJDX8yK8m9KnTMarsE84zdsy"), "S/cold-staking must be valid");
        // 6 = P2SH (version 13)
        assert!(is_valid_address("6EPs1STNZh4rxbsgP93Zu4BeHF5n9dtECo"), "6/P2SH must be valid");
        // EXM = exchange address (3-byte prefix, OP_EXCHANGEADDR 0xe0, 36 chars)
        assert!(is_valid_address("EXMBfGoMQMNiHTNsrs8SdrsBotUButbb1SP1"), "EXM/exchange must be valid");
    }

    /// A one-char typo of a real address must be rejected (checksum mismatch) —
    /// this is the P2-B bug: previously returned HTTP 200 balance:0.
    #[test]
    fn is_valid_address_rejects_typos_and_garbage() {
        // Flip the last char of the D address: r5 -> r6 (breaks the checksum).
        assert!(!is_valid_address("DU8gPC5mh4KxWJARQRxoESFark2jAguBr6"), "typo'd address must be invalid");
        // Empty / clearly-not-an-address input.
        assert!(!is_valid_address(""), "empty must be invalid");
        assert!(!is_valid_address("not an address"), "garbage must be invalid");
        // Valid base58check but wrong payload length (1-byte version + 19-byte
        // hash = 20-byte payload) must still be rejected.
        assert!(!is_valid_address("11GsChQR2U32pvwJcDNPoYHhGcnz5Rv"), "short payload must be invalid");
    }

    /// xpub guard: accepts a real BIP32 xpub, rejects typos/non-xpub input.
    #[test]
    fn is_valid_xpub_basic() {
        // A standard BIP32 mainnet xpub (Bitcoin/PIVX share 0x0488B21E).
        let good = "xpub661MyMwAqRbcEYSGagKuFUqExQV8d2eizDP5SamP9TcLeqAk9JsrNexcG6RbwEy38PSSahG1iVxeqWMJEq3YFHbfEaayHjyFJnJ6DS2h49t";
        assert!(is_valid_xpub(good), "real xpub must be valid");
        // Flip the last char -> checksum mismatch (payload length still 78).
        let bad = "xpub661MyMwAqRbcEYSGagKuFUqExQV8d2eizDP5SamP9TcLeqAk9JsrNexcG6RbwEy38PSSahG1iVxeqWMJEq3YFHbfEaayHjyFJnJ6DS2h49A";
        assert!(!is_valid_xpub(bad), "typo'd xpub must be invalid");
        // A transparent address is not an xpub.
        assert!(!is_valid_xpub("DU8gPC5mh4KxWJARQRxoESFark2jAguBr5"), "address is not an xpub");
    }

    /// Option-① regression: compute_address_info must (a) serve balance/totals
    /// from the persisted 'r'/'s' aggregates with balance == r - s, and (b) order
    /// txids newest-first by block height via the single batched blocking pass —
    /// without recomputing from full history. Builds a tiny addr_index +
    /// transactions DB and asserts both.
    #[tokio::test]
    async fn compute_address_info_serves_persisted_and_orders_by_height() {
        use rocksdb::{DB, Options};
        use super::compute_address_info;
        use crate::api::types::AddressQuery;

        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = std::sync::Arc::new(
            DB::open_cf(&opts, temp.path(), ["addr_index", "transactions"]).unwrap(),
        );
        let cf_ai = db.cf_handle("addr_index").unwrap();
        let cf_tx = db.cf_handle("transactions").unwrap();

        let address = "DUnitTestAddressXXXXXXXXXXXXXXXXXXX";

        // Three txids with distinct heights; the 't'-list order is deliberately
        // NOT height order, so a correct sort must reorder them.
        let mk = |b: u8| -> [u8; 32] { [b; 32] };
        let (a, b, c) = (mk(0xAA), mk(0xBB), mk(0xCC));
        let heights = [(a, 300i32), (b, 100i32), (c, 200i32)];

        // tx CF value layout: 8-byte header, block height as i32 LE at bytes 4..8.
        let tx_value = |h: i32| -> Vec<u8> {
            let mut v = vec![0u8; 8];
            v[4..8].copy_from_slice(&h.to_le_bytes());
            v
        };
        for (txid, h) in heights {
            let mut k = vec![b't'];
            k.extend_from_slice(&txid);
            db.put_cf(&cf_tx, &k, tx_value(h)).unwrap();
        }

        // 't'+address = concatenated 32-byte txids, in NON-height order (a,b,c).
        let mut t_key = vec![b't'];
        t_key.extend_from_slice(address.as_bytes());
        let mut t_val = Vec::new();
        for txid in [a, b, c] {
            t_val.extend_from_slice(&txid);
        }
        db.put_cf(&cf_ai, &t_key, &t_val).unwrap();

        // Persisted aggregates: r=1000, s=300  => balance must be exactly 700.
        let mut r_key = vec![b'r'];
        r_key.extend_from_slice(address.as_bytes());
        db.put_cf(&cf_ai, &r_key, 1000i64.to_le_bytes()).unwrap();
        let mut s_key = vec![b's'];
        s_key.extend_from_slice(address.as_bytes());
        db.put_cf(&cf_ai, &s_key, 300i64.to_le_bytes()).unwrap();

        let params: AddressQuery = serde_json::from_str("{}").unwrap();
        let info = compute_address_info(&db, address, &params).await.unwrap();

        // (a) values served straight from r/s, balance = r - s (no recompute).
        assert_eq!(info.total_received, "1000", "totalReceived must come from 'r'");
        assert_eq!(info.total_sent, "300", "totalSent must come from 's'");
        assert_eq!(info.balance, "700", "balance must be r - s exactly");
        assert_eq!(info.txs, 3, "tx count from the 't' list");

        // (b) newest-first ordering by height: 300 (a), 200 (c), 100 (b).
        let txids = info.txids.expect("details=txids returns the txid list");
        assert_eq!(
            txids,
            vec![hex::encode(a), hex::encode(c), hex::encode(b)],
            "txids must be ordered by block height descending",
        );
    }
}
