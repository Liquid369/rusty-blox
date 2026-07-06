// Address and UTXO API Endpoints
//
// NOTE: These are complex endpoints with significant database operations.
// Caching provides moderate benefits since addresses are frequently updated.

use axum::{
    extract::{Path as AxumPath, Query},
    Extension, Json,
};
use rocksdb::DB;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use super::transactions::fetch_transactions_batch;
use super::types::{AddressInfo, AddressQuery, UtxoQuery, XPubInfo, UTXO};
use crate::cache::CacheManager;
use crate::constants::{is_canonical_height, HEIGHT_ORPHAN, HEIGHT_UNRESOLVED};
use crate::maturity::get_current_height;
use crate::parser::deserialize_transaction_blocking;

/// Redact xpub for safe logging (privacy protection)
/// Shows first 8 and last 4 characters: "xpub661M...3Mzx"
pub(crate) fn redact_xpub(xpub: &str) -> String {
    // Char-indexed, not byte-indexed: this runs on attacker-controlled input
    // (the invalid-xpub branch), so a multi-byte UTF-8 char must not panic.
    let chars: Vec<char> = xpub.chars().collect();
    if chars.len() <= 12 {
        return "<invalid>".to_string();
    }
    let head: String = chars[..8].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}...{tail}")
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
/// A MISSING key reads as 0 (a never-seen address genuinely has no totals); a
/// read ERROR propagates — folding it into 0 served a confident zeroed account
/// for a rich address, minting the exact failure the handler's 500-on-error
/// path exists to catch, one layer below it.
async fn read_address_totals(db: &Arc<DB>, address: &str) -> Result<(i64, i64), String> {
    let r_key = format!("r{address}").into_bytes();
    let s_key = format!("s{address}").into_bytes();
    let db_clone = Arc::clone(db);

    tokio::task::spawn_blocking(move || -> Result<(i64, i64), String> {
        let cf = db_clone
            .cf_handle("addr_index")
            .ok_or_else(|| "addr_index CF not found".to_string())?;
        let read_i64 = |key: &[u8]| -> Result<i64, String> {
            Ok(db_clone
                .get_cf(&cf, key)
                .map_err(|e| e.to_string())?
                .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
                .map(i64::from_le_bytes)
                .unwrap_or(0))
        };
        Ok((read_i64(&r_key)?, read_i64(&s_key)?))
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
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

    // 503 while the addr_index is reindexing or not in the current (v2) on-disk format.
    // The web server is released independently of the enrich, so without this gate a
    // v1->v2 in-place upgrade would serve old-stride bytes / divergent StrideMismatch
    // errors (CR-1) until the rebuild finishes.
    if !crate::chain_state::addr_index_ready(&db) {
        return Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(super::types::BlockbookError::new(
                "Address index is reindexing; please retry shortly",
            )),
        ));
    }

    let cache_key = format!("addr:{}:{}:{}", address, params.page, params.details);
    let db_clone = Arc::clone(&db);
    let address_clone = address.clone();
    let params_clone = params.clone();

    let result = cache
        .get_or_compute(&cache_key, Duration::from_secs(30), || async move {
            compute_address_info(&db_clone, &address_clone, &params_clone).await
        })
        .await;

    match result {
        Ok(info) => Ok(Json(info)),
        Err(e) => {
            // A transient DB/compute error for a checksum-valid address must FAIL
            // the request. Returning a zeroed account here (as this used to) is a
            // confident false statement — "this address holds nothing" — that a
            // wallet will act on; Blockbook 5xxs the same case.
            warn!(address = %address, error = %e, "address compute failed");
            Err((
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(super::types::BlockbookError::new(
                    "Internal error computing address; please retry",
                )),
            ))
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
        let cf_addr_index = db_clone
            .cf_handle("addr_index")
            .ok_or_else(|| "addr_index CF not found".to_string())?;
        db_clone
            .get_cf(&cf_addr_index, &tx_list_key_bytes)
            .map_err(|e| e.to_string())
            .map(|opt| opt.unwrap_or_default())
    })
    .await??;

    // v2 't' records are 36 bytes (txid(32) + height(i32 LE)). The inline height is
    // authoritative, so newest-first ordering needs NO per-txid tx-CF lookup. A
    // stride mismatch (stale/legacy blob) is a hard error, surfaced — not silently
    // truncated.
    let tx_entries = crate::parser::deserialize_addr_txs(&tx_list_data).await?;

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
    let (total_received, total_sent) = read_address_totals(db, address).await?;
    let balance = total_received - total_sent;

    // P1-1: the stored 't' txids are ALREADY canonical display order (parser.rs
    // hash_txid reverses to display; the tx CF is keyed by 't' + display bytes), so
    // they are emitted verbatim.
    //
    // Order newest-first by the INLINE block height — no tx-CF lookups, no
    // spawn_blocking; an O(N log N) in-memory sort on data already loaded. This is
    // the O(history) cold-load fix (was one get_cf per txid). Above the order cap,
    // serve the stored order unchanged (preserves the prior over_cap behavior).
    // Stable sort keeps stored order within an equal height.
    let order_cap = address_recompute_cap();
    let all_txids: Vec<String> = if tx_entries.len() > order_cap {
        tx_entries.iter().map(|(t, _)| hex::encode(t)).collect()
    } else {
        let mut ordered = tx_entries.clone();
        ordered.sort_by(|a, b| b.1.cmp(&a.1));
        ordered.into_iter().map(|(t, _)| hex::encode(t)).collect()
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
    let end_idx = start_idx
        .saturating_add(page_size as usize)
        .min(total_tx_count);

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
        }
        "txs" => {
            // Full transaction objects - fetch them
            let txs = fetch_transactions_batch(db, &paginated_txids).await;
            (None, Some(txs))
        }
        _ => {
            // Default: "txids" or any other value = just txid strings
            (Some(paginated_txids), None)
        }
    };

    Ok(AddressInfo {
        page: Some(page),
        total_pages: Some(total_pages),
        items_on_page: Some(actual_items as u32), // Actual count, not pageSize
        address: address.to_string(),
        balance: balance.to_string(),
        total_received: total_received.to_string(),
        total_sent: total_sent.to_string(),
        unconfirmed_balance: "0".to_string(),
        unconfirmed_txs: 0,
        txs: total_tx_count as u32, // Total tx count (not paginated count)
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

    // 503 while the addr_index is reindexing / not the current (v2) format (see addr_v2).
    if !crate::chain_state::addr_index_ready(&db) {
        return Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(super::types::BlockbookError::new(
                "Address index is reindexing; please retry shortly",
            )),
        ));
    }

    let cache_key = format!("xpub:{}:{}:{}", xpub_str, params.page, params.details);
    let db_clone = Arc::clone(&db);
    let xpub_clone = xpub_str.clone();
    let params_clone = params.clone();

    let result = cache
        .get_or_compute(&cache_key, Duration::from_secs(300), || async move {
            compute_xpub_info(&db_clone, &xpub_clone, &params_clone).await
        })
        .await;

    match result {
        Ok(info) => Ok(Json(info)),
        Err(e) => {
            // Log error with redacted xpub (privacy protection)
            warn!(xpub = %redact_xpub(&xpub_str), error = %e, "xpub query error");
            Err((
                axum::http::StatusCode::BAD_REQUEST,
                Json(super::types::BlockbookError::new(e.to_string())),
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
    let xpub = ExtendedPubKey::from_str(xpub_str).map_err(|e| {
        format!("Invalid xpub format: {e}. Please provide a valid BIP32 extended public key.")
    })?;

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
            }
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
            }
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
        let cf_addr_index = db_clone
            .cf_handle("addr_index")
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
    let utxo_keys: Vec<Vec<u8>> = all_addresses
        .iter()
        .map(|(addr, _)| format!("a{addr}").into_bytes())
        .collect();

    // Build batch of transaction list keys ("t" + address)
    let tx_list_keys: Vec<Vec<u8>> = all_addresses
        .iter()
        .map(|(addr, _)| format!("t{addr}").into_bytes())
        .collect();

    let db_clone = db.clone();
    let utxo_keys_clone = utxo_keys.clone();
    let tx_list_keys_clone = tx_list_keys.clone();

    // Execute batched queries
    let (utxo_results, tx_list_results) = tokio::task::spawn_blocking(
        move || -> Result<(Vec<Option<Vec<u8>>>, Vec<Option<Vec<u8>>>), String> {
            let cf_addr_index = db_clone
                .cf_handle("addr_index")
                .ok_or_else(|| "addr_index CF not found".to_string())?;

            // Batch get UTXOs for all addresses
            let utxo_batch: Vec<_> = utxo_keys_clone
                .iter()
                .map(|k| (&cf_addr_index, k.as_slice()))
                .collect();
            // Absent key (None) = unused derived address, a legitimate zero.
            // A per-key read ERROR must fail the request — folded into None it
            // reads as "unused" and silently undercuts the xpub money totals.
            let utxo_results: Vec<Option<Vec<u8>>> = db_clone
                .multi_get_cf(utxo_batch)
                .into_iter()
                .collect::<Result<_, _>>()
                .map_err(|e| format!("addr_index batch read failed: {e}"))?;

            // Batch get transaction lists for all addresses
            let tx_list_batch: Vec<_> = tx_list_keys_clone
                .iter()
                .map(|k| (&cf_addr_index, k.as_slice()))
                .collect();
            let tx_list_results: Vec<Option<Vec<u8>>> = db_clone
                .multi_get_cf(tx_list_batch)
                .into_iter()
                .collect::<Result<_, _>>()
                .map_err(|e| format!("addr_index batch read failed: {e}"))?;

            Ok((utxo_results, tx_list_results))
        },
    )
    .await
    .map_err(|e| format!("Task join error: {e}"))?
    .map_err(|e| e.to_string())?;

    // Process each address with pre-fetched data
    for (idx, (address, path)) in all_addresses.iter().enumerate() {
        let utxo_data = utxo_results
            .get(idx)
            .and_then(|opt| opt.as_ref())
            .cloned()
            .unwrap_or_default();

        let utxos = crate::parser::deserialize_addr_utxos(&utxo_data).await?;

        // Blockbook parity: include immature outputs in xpub balances. Balance is the
        // sum of the inline 'a' values — the inline value IS the unspent output value,
        // so the result is unchanged while the per-UTXO tx-CF parse is eliminated.
        let address_balance: i64 = utxos.iter().map(|(_, _, value, _)| *value).sum();

        // Get transaction list from pre-fetched batch data
        let tx_list_data = tx_list_results
            .get(idx)
            .and_then(|opt| opt.as_ref())
            .cloned()
            .unwrap_or_default();

        // v2 't' list = 36-byte records (txid + height); only the txids are needed here.
        let tx_entries = crate::parser::deserialize_addr_txs(&tx_list_data).await?;
        let txids: Vec<Vec<u8>> = tx_entries.iter().map(|(t, _)| t.clone()).collect();

        // Calculate total received for this address
        let mut total_received: i64 = 0;

        // ONE blocking pass over this address's txs (a per-tx spawn_blocking dispatch
        // + async parse hop made large xpubs pay thousands of dispatches per uncached
        // request once the stub-safe reader started actually finding historical txs).
        // Reads are orphan-AWARE: a reorg orphan-marks only the display record, and
        // the internal-first body read would otherwise count a disconnected tx —
        // permanently inflating totalReceived. A read ERROR propagates (fails the
        // request): swallowing it would silently undercount a money total under 200.
        let txids_clone: Vec<Vec<u8>> = txids.clone();
        let db_clone = db.clone();
        let addr_clone = address.clone();
        total_received += tokio::task::spawn_blocking(move || -> Result<i64, String> {
            let cf_transactions = db_clone
                .cf_handle("transactions")
                .ok_or_else(|| "transactions CF not found".to_string())?;
            let mut sum: i64 = 0;
            for txid in &txids_clone {
                let (tx_data, orphan_marked) =
                    crate::api::transactions::read_tx_record_orphan_aware(
                        &db_clone,
                        &cf_transactions,
                        txid,
                    )
                    .map_err(|e| e.to_string())?;
                if orphan_marked {
                    continue;
                }
                let Some(tx_data) = tx_data else { continue };
                // Skip orphaned/unresolved sentinels on the served record itself.
                let block_height =
                    i32::from_le_bytes([tx_data[4], tx_data[5], tx_data[6], tx_data[7]]);
                if block_height == HEIGHT_ORPHAN || block_height == HEIGHT_UNRESOLVED {
                    continue;
                }
                let mut tx_data_with_header = Vec::with_capacity(4 + tx_data.len() - 8);
                tx_data_with_header.extend_from_slice(&[0u8; 4]);
                tx_data_with_header.extend_from_slice(&tx_data[8..]);
                if let Ok(tx) = deserialize_transaction_blocking(&tx_data_with_header) {
                    for output in &tx.outputs {
                        if output.address.contains(&addr_clone) {
                            sum += output.value;
                        }
                    }
                }
            }
            Ok(sum)
        })
        .await
        .map_err(|e| format!("Task join error: {e}"))?
        .map_err(|e| e.to_string())?;

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
            // Height via the shared stub-safe reader (BOTH key orders): a raw
            // display-only read sorted internal-keyed (initial-sync) txs as height 0
            // and read a stub's -1 sentinel. Sorting is cosmetic, so a read error
            // still degrades to height 0 rather than failing the request.
            let db_clone = db.clone();
            let height = tokio::task::spawn_blocking(move || -> i32 {
                if let Some(cf) = db_clone.cf_handle("transactions") {
                    if let Ok(Some(tx_data)) =
                        crate::api::transactions::read_valid_tx_record(&db_clone, &cf, &txid_bytes)
                    {
                        return i32::from_le_bytes([
                            tx_data[4], tx_data[5], tx_data[6], tx_data[7],
                        ]);
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
    let end_idx = start_idx
        .saturating_add(page_size as usize)
        .min(total_tx_count);

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
        }
        "txs" => {
            // Full transaction objects - fetch them
            let txs = fetch_transactions_batch(db, &paginated_txids).await;
            (None, Some(txs))
        }
        "tokens" | "tokenBalances" => {
            // Token modes don't return tx data
            (None, None)
        }
        _ => {
            // Default: "txids" or any other value = just txid strings
            (Some(paginated_txids), None)
        }
    };

    // Build tokens array if requested, filtered by tokens parameter
    let tokens = if params.details == "tokens" || params.details == "tokenBalances" {
        // Filter addresses based on tokens parameter
        let filtered_addresses: Vec<_> = all_addresses
            .iter()
            .filter_map(|(addr, path)| {
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
                    }
                    "used" => {
                        // Return addresses with at least one transaction
                        addr_data
                            .filter(|(_, _, tx_count, _, _, _)| *tx_count > 0)
                            .map(|(_, _, tx_count, balance, total_recv, total_snt)| {
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
                    "nonzero" | _ => {
                        // Default: return only addresses with nonzero balance
                        addr_data
                            .filter(|(_, _, _, balance, _, _)| *balance != 0)
                            .map(|(_, _, tx_count, balance, total_recv, total_snt)| {
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
            })
            .collect();

        // Apply pagination to tokens array
        let total_tokens = filtered_addresses.len() as u32;
        let tokens_page = params.tokens_page.max(1);
        let tokens_page_size = params.tokens_page_size.clamp(1, 1000);
        let total_tokens_pages = ((total_tokens as f64) / (tokens_page_size as f64)).ceil() as u32;
        let total_tokens_pages = if total_tokens_pages == 0 {
            1
        } else {
            total_tokens_pages
        };

        let start_idx = (tokens_page as usize - 1).saturating_mul(tokens_page_size as usize);
        let end_idx = start_idx
            .saturating_add(tokens_page_size as usize)
            .min(filtered_addresses.len());

        let paginated_tokens: Vec<_> = if start_idx < filtered_addresses.len() {
            filtered_addresses[start_idx..end_idx].to_vec()
        } else {
            Vec::new()
        };

        (
            Some(paginated_tokens),
            Some(total_tokens),
            Some(tokens_page),
            Some(total_tokens_pages),
        )
    } else {
        (None, None, None, None)
    };

    // Blockbook always returns usedTokens (count of addresses with activity)
    // regardless of details mode
    let used_tokens = Some(used_addresses.len() as u32);

    // Aggregate totals
    let xpub_total_received: i64 = used_addresses
        .iter()
        .map(|(_, _, _, _, total_recv, _)| total_recv)
        .sum();
    let xpub_total_sent: i64 = used_addresses
        .iter()
        .map(|(_, _, _, _, _, total_snt)| total_snt)
        .sum();
    let xpub_balance: i64 = used_addresses
        .iter()
        .map(|(_, _, _, balance, _, _)| balance)
        .sum();

    // Blockbook's txs field for xpub = total transfers across ALL addresses
    // (not unique transactions). If an address appears in 2 txs, it counts as 2.
    // This matches: sum of all per-address tx counts = total "transfers"
    let total_transfers: usize = used_addresses
        .iter()
        .map(|(_, _, tx_count, _, _, _)| tx_count)
        .sum();

    Ok(XPubInfo {
        page,
        total_pages,
        items_on_page: actual_items as u32, // Actual count, not pageSize
        address: xpub_str.to_string(),
        balance: xpub_balance.to_string(),
        total_received: xpub_total_received.to_string(),
        total_sent: xpub_total_sent.to_string(),
        unconfirmed_balance: "0".to_string(),
        unconfirmed_txs: 0,
        txs: total_transfers as u32, // Total transfers (Blockbook compatibility)
        txids,
        tokens: tokens.0,
        transactions, // Now properly populated when details=txs
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

    // 503 while the addr_index is reindexing / not the current (v2) format (see addr_v2).
    if !crate::chain_state::addr_index_ready(&db) {
        return Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(super::types::BlockbookError::new(
                "Address index is reindexing; please retry shortly",
            )),
        ));
    }

    let cache_key = format!("utxo:{}:{}", address, query.confirmed);
    let db_clone = Arc::clone(&db);
    let address_clone = address.clone();

    let result = cache
        .get_or_compute(&cache_key, Duration::from_secs(30), || async move {
            compute_utxos(&db_clone, &address_clone).await
        })
        .await;

    match result {
        Ok(utxos) => Ok(Json(utxos)),
        Err(e) => {
            // An internal error must FAIL the request: a 200 [] tells a wallet
            // "no coins to spend" — a confident false statement it will act on.
            warn!(address = %address, error = %e, "utxo compute failed");
            Err((
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(super::types::BlockbookError::new(
                    "Internal error computing UTXOs; please retry",
                )),
            ))
        }
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
        let cf_addr_index = db_clone
            .cf_handle("addr_index")
            .ok_or_else(|| "addr_index CF not found".to_string())?;
        db_clone
            .get_cf(&cf_addr_index, &key_bytes)
            .map_err(|e| e.to_string())
            .map(|opt| opt.unwrap_or_default())
    })
    .await??;

    let unspent_utxos = crate::parser::deserialize_addr_utxos(&result).await?;
    let current_height = get_current_height(db).unwrap_or(0);
    // Blockbook parity: the UTXO list includes immature coinbase/coinstake outputs
    // (Blockbook flags them rather than hiding them; wallets handle maturity).
    let spendable_utxos = unspent_utxos.clone();

    let mut utxo_list = Vec::new();

    for (txid_hash, vout, value, kind) in &spendable_utxos {
        // P1-1: the 'a' unspent index stores txids in canonical DISPLAY order
        // (the same bytes that key the tx CF below), so emit them directly. The
        // earlier .reverse() produced a non-canonical txid that the node,
        // duddino, wallets, and every other explorer reject.
        let txid_display = hex::encode(txid_hash);

        // Look up the tx record for height/confirmations via the shared stub-safe
        // reader (BOTH key orders; a body-less 8-byte stub must not shadow the real
        // record — a raw one-key read here used to silently DROP a canonical UTXO
        // when the probed key held a stub or the record lived at the other order).
        // Read errors degrade to "record missing" (UTXO skipped), as before.
        let db_clone = Arc::clone(db);
        let txid_owned = txid_hash.clone();
        let tx_data_opt = tokio::task::spawn_blocking(move || {
            let cf = db_clone.cf_handle("transactions")?;
            crate::api::transactions::read_valid_tx_record(&db_clone, &cf, &txid_owned)
                .unwrap_or(None)
        })
        .await
        .ok()
        .flatten();

        if let Some(tx_data) = tx_data_opt {
            let block_height = i32::from_le_bytes([tx_data[4], tx_data[5], tx_data[6], tx_data[7]]);

            // A UTXO present in the 'a' unspent set is canonical by construction;
            // only drop a definitive orphan (-1). A transient HEIGHT_UNRESOLVED
            // read must NOT silently drop a canonical UTXO. Height/orphan are
            // derived LIVE from the tx CF (not the inline 'a' fields), exactly as
            // before — only the expensive full-tx parse is removed.
            if block_height == HEIGHT_ORPHAN {
                continue;
            }

            let confirmations =
                if is_canonical_height(block_height) && is_canonical_height(current_height) {
                    ((current_height - block_height) + 1).max(0) as u32
                } else {
                    0
                };

            // value + coinbase/coinstake come from the inline 'a' record (49B):
            // no deserialize_transaction on the hot (confirmed) path. lock_time is
            // only ever emitted when confirmations == 0, so parse the tx ONLY in
            // that rare degraded/unconfirmed case to preserve exact prior output.
            let tx_type = crate::tx_type::u8_to_ty(*kind);
            let lock_time = if confirmations == 0 {
                let tx_data_len = tx_data.len() - 8;
                let mut tx_with_header = Vec::with_capacity(4 + tx_data_len);
                tx_with_header.extend_from_slice(&[0u8; 4]);
                tx_with_header.extend_from_slice(&tx_data[8..]);
                deserialize_transaction_blocking(&tx_with_header)
                    .ok()
                    .filter(|tx| tx.lock_time > 0)
                    .map(|tx| tx.lock_time)
            } else {
                None
            };

            utxo_list.push(UTXO {
                txid: txid_display,
                vout: *vout as u32,
                value: value.to_string(),
                confirmations,
                lock_time,
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

    // Sort by confirmations ascending (newest first = least confirmations first)
    utxo_list.sort_by(|a, b| a.confirmations.cmp(&b.confirmations));

    Ok(utxo_list)
}

#[cfg(test)]
mod tests {
    use super::{is_valid_address, is_valid_xpub, redact_xpub};

    /// P2: `redact_xpub` is called on the invalid-xpub branch with fully
    /// attacker-controlled input, so it must never panic. The old code
    /// byte-sliced `&xpub[..8]` guarded only by byte-length, which panicked when
    /// byte 8 (or len-4) fell inside a multi-byte UTF-8 char.
    #[test]
    fn redact_xpub_never_panics_on_multibyte() {
        // 13 multi-byte chars (39 bytes). Old `&xpub[..8]` split the 3rd char.
        let long = "中".repeat(13);
        assert_eq!(
            redact_xpub(&long),
            format!("{}...{}", "中".repeat(8), "中".repeat(4))
        );
        // 7 chars but 13 bytes: passes the old byte-length guard (>12) and
        // panicked; char-length (7 <= 12) correctly reports "<invalid>".
        assert_eq!(redact_xpub("xpub中中中"), "<invalid>");
        // ASCII behavior is unchanged (char count == byte count).
        assert_eq!(redact_xpub("xpub661MyMwAqRbc"), "xpub661M...qRbc");
    }

    /// P2-B: every real PIVX transparent address class must pass validation.
    /// Asymmetric risk — a validator that rejects a VALID address is worse than
    /// the original fake-zero-account bug, so these MUST stay true.
    #[test]
    fn is_valid_address_accepts_all_real_classes() {
        // D = standard P2PKH (version 30)
        assert!(
            is_valid_address("DU8gPC5mh4KxWJARQRxoESFark2jAguBr5"),
            "D/P2PKH must be valid"
        );
        // S = cold-staking staker (version 63)
        assert!(
            is_valid_address("SdgQDpS8jDRJDX8yK8m9KnTMarsE84zdsy"),
            "S/cold-staking must be valid"
        );
        // 6 = P2SH (version 13)
        assert!(
            is_valid_address("6EPs1STNZh4rxbsgP93Zu4BeHF5n9dtECo"),
            "6/P2SH must be valid"
        );
        // EXM = exchange address (3-byte prefix, OP_EXCHANGEADDR 0xe0, 36 chars)
        assert!(
            is_valid_address("EXMBfGoMQMNiHTNsrs8SdrsBotUButbb1SP1"),
            "EXM/exchange must be valid"
        );
    }

    /// A one-char typo of a real address must be rejected (checksum mismatch) —
    /// this is the P2-B bug: previously returned HTTP 200 balance:0.
    #[test]
    fn is_valid_address_rejects_typos_and_garbage() {
        // Flip the last char of the D address: r5 -> r6 (breaks the checksum).
        assert!(
            !is_valid_address("DU8gPC5mh4KxWJARQRxoESFark2jAguBr6"),
            "typo'd address must be invalid"
        );
        // Empty / clearly-not-an-address input.
        assert!(!is_valid_address(""), "empty must be invalid");
        assert!(
            !is_valid_address("not an address"),
            "garbage must be invalid"
        );
        // Valid base58check but wrong payload length (1-byte version + 19-byte
        // hash = 20-byte payload) must still be rejected.
        assert!(
            !is_valid_address("11GsChQR2U32pvwJcDNPoYHhGcnz5Rv"),
            "short payload must be invalid"
        );
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
        assert!(
            !is_valid_xpub("DU8gPC5mh4KxWJARQRxoESFark2jAguBr5"),
            "address is not an xpub"
        );
    }

    /// Option-① regression: compute_address_info must (a) serve balance/totals
    /// from the persisted 'r'/'s' aggregates with balance == r - s, and (b) order
    /// txids newest-first by block height via the single batched blocking pass —
    /// without recomputing from full history. Builds a tiny addr_index +
    /// transactions DB and asserts both.
    #[tokio::test]
    async fn compute_address_info_serves_persisted_and_orders_by_height() {
        use super::compute_address_info;
        use crate::api::types::AddressQuery;
        use rocksdb::{Options, DB};

        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = std::sync::Arc::new(
            DB::open_cf(&opts, temp.path(), ["addr_index", "transactions"]).unwrap(),
        );
        let cf_ai = db.cf_handle("addr_index").unwrap();

        let address = "DUnitTestAddressXXXXXXXXXXXXXXXXXXX";

        // Three txids with distinct heights; the 't'-list order is deliberately
        // NOT height order, so a correct sort must reorder them.
        let mk = |b: u8| -> [u8; 32] { [b; 32] };
        let (a, b, c) = (mk(0xAA), mk(0xBB), mk(0xCC));
        let heights = [(a, 300i32), (b, 100i32), (c, 200i32)];

        // 't'+address = v2 36-byte records (txid + inline height i32 LE), written in
        // NON-height order (a,b,c). NO tx-CF height entries are written, so this also
        // proves the reader sorts on the inline height, not a tx-CF lookup.
        let height_of = |txid: [u8; 32]| -> i32 {
            heights
                .iter()
                .find(|(t, _)| *t == txid)
                .map(|(_, h)| *h)
                .unwrap()
        };
        let mut t_key = vec![b't'];
        t_key.extend_from_slice(address.as_bytes());
        let mut t_val = Vec::new();
        for txid in [a, b, c] {
            t_val.extend_from_slice(&txid);
            t_val.extend_from_slice(&height_of(txid).to_le_bytes());
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
        assert_eq!(
            info.total_received, "1000",
            "totalReceived must come from 'r'"
        );
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
