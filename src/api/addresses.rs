// Address and UTXO API Endpoints
//
// NOTE: These are complex endpoints with significant database operations.
// Caching provides moderate benefits since addresses are frequently updated.

use axum::{Json, Extension, extract::{Path as AxumPath, Query}};
use rocksdb::DB;
use std::sync::Arc;
use std::time::Duration;

use crate::cache::CacheManager;
use crate::constants::{HEIGHT_ORPHAN, HEIGHT_UNRESOLVED, is_canonical_height};
use crate::parser::{deserialize_utxos, deserialize_transaction, deserialize_transaction_blocking};
use crate::maturity::{filter_spendable_utxos, get_current_height};
use super::types::{AddressInfo, AddressQuery, XPubInfo, UTXO, UtxoQuery};

/// Redact xpub for safe logging (privacy protection)
/// Shows first 8 and last 4 characters: "xpub661M...3Mzx"
pub(crate) fn redact_xpub(xpub: &str) -> String {
    if xpub.len() <= 12 {
        return "<invalid>".to_string();
    }
    format!("{}...{}", &xpub[..8], &xpub[xpub.len()-4..])
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
) -> Json<AddressInfo> {
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
        Ok(info) => Json(info),
        Err(_) => {
            // Fallback: return empty address info
            Json(AddressInfo {
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
            })
        }
    }
}

async fn compute_address_info(
    db: &Arc<DB>,
    address: &str,
    params: &AddressQuery,
) -> Result<AddressInfo, Box<dyn std::error::Error + Send + Sync>> {
    let key = format!("a{}", address);
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
    
    // Get transaction list
    let tx_list_key = format!("t{}", address);
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
    
    let current_height = get_current_height(db).unwrap_or(0);
    let spendable_utxos = filter_spendable_utxos(
        unspent_utxos.clone(),
        Arc::clone(db),
        current_height,
    ).await;
    
    let mut balance: i64 = 0;
    
    for (txid_hash, output_index) in &spendable_utxos {
        let mut key = vec![b't'];
        key.extend(txid_hash);
        let key_clone = key.clone();
        let db_clone = Arc::clone(db);
        
        let tx_data = tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
            let cf_transactions = db_clone.cf_handle("transactions")?;
            db_clone.get_cf(&cf_transactions, &key_clone).ok()?
        })
        .await
        .ok()
        .flatten();
        
        if let Some(tx_data) = tx_data {
            if tx_data.len() >= 8 {
                let tx_data_len = tx_data.len() - 8;
                if tx_data_len > 0 {
                    let mut tx_data_with_header = Vec::with_capacity(4 + tx_data_len);
                    tx_data_with_header.extend_from_slice(&[0u8; 4]);
                    tx_data_with_header.extend_from_slice(&tx_data[8..]);
                
                    if let Ok(tx) = deserialize_transaction(&tx_data_with_header).await {
                        if let Some(output) = tx.outputs.get(*output_index as usize) {
                            balance += output.value;
                        }
                    }
                }
            }
        }
    }
    
    let mut total_received: i64 = 0;
    for txid_internal in &all_txids {
        let mut key = vec![b't'];
        key.extend(txid_internal);
        let key_clone = key.clone();
        let db_clone = Arc::clone(db);
        let address_clone = address.to_string();
        
        let tx_data = tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
            let cf_transactions = db_clone.cf_handle("transactions")?;
            db_clone.get_cf(&cf_transactions, &key_clone).ok()?
        })
        .await
        .ok()
        .flatten();
        
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
                            if output.address.contains(&address_clone) {
                                total_received += output.value;
                            }
                        }
                    }
                }
            }
        }
    }
    
    let total_sent = total_received - balance;
    
    let unique_txids: Vec<String> = all_txids.iter()
        .map(|txid_internal| {
            let mut txid_display = txid_internal.clone();
            txid_display.reverse();
            hex::encode(&txid_display)
        })
        .collect();
    
    // Sort transactions by block height (descending = newest first)
    let mut txid_heights: Vec<(String, i32)> = Vec::new();
    for txid in &unique_txids {
        if let Ok(txid_bytes) = hex::decode(txid) {
            // Reverse to internal format for database lookup
            let txid_internal: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
            let mut key = vec![b't'];
            key.extend(&txid_internal);
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
    
    let tx_count = unique_txids.len() as u32;
    let total_pages = ((tx_count as f64) / (params.page_size as f64)).ceil() as u32;
    let total_pages = if total_pages == 0 { 1 } else { total_pages };
    
    let txids = if params.details == "basic" {
        None
    } else {
        Some(unique_txids)
    };
    
    Ok(AddressInfo {
        page: Some(params.page),
        total_pages: Some(total_pages),
        items_on_page: Some(params.page_size),
        address: address.to_string(),
        balance: balance.to_string(),
        total_received: total_received.to_string(),
        total_sent: total_sent.to_string(),
        unconfirmed_balance: "0".to_string(),
        unconfirmed_txs: 0,
        txs: tx_count,
        txids,
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
    let cache_key = format!("xpub:{}:{}", xpub_str, params.page);
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
            eprintln!("xpub query error for {}: {}", redact_xpub(&xpub_str), e);
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
        .map_err(|e| format!("Invalid xpub format: {}. Please provide a valid BIP32 extended public key.", e))?;
    
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
    let path = format!("m/44'/119'/{}'/{}/{}", account, chain, index);
    
    Ok((address, path))
}

/// Check if an address has any activity (UTXOs or transactions)
async fn check_address_activity(
    db: &Arc<DB>,
    address: &str,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Check UTXO key: 'a' + address
    let utxo_key = format!("a{}", address);
    let utxo_key_bytes = utxo_key.as_bytes().to_vec();
    
    // Check transaction list key: 't' + address
    let tx_key = format!("t{}", address);
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
    let current_height = get_current_height(db).unwrap_or(0);
    
    // PERFORMANCE OPTIMIZATION: Batch all address lookups into single multi_get_cf call
    // This replaces N sequential queries with 1 batched query (10-50x faster)
    
    // Build batch of UTXO keys ("a" + address)
    let utxo_keys: Vec<Vec<u8>> = all_addresses.iter()
        .map(|(addr, _)| format!("a{}", addr).into_bytes())
        .collect();
    
    // Build batch of transaction list keys ("t" + address)
    let tx_list_keys: Vec<Vec<u8>> = all_addresses.iter()
        .map(|(addr, _)| format!("t{}", addr).into_bytes())
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
    .map_err(|e| format!("Task join error: {}", e))?
    .map_err(|e| e.to_string())?;
    
    // Process each address with pre-fetched data
    for (idx, (address, path)) in all_addresses.iter().enumerate() {
        let utxo_data = utxo_results.get(idx)
            .and_then(|opt| opt.as_ref())
            .map(|v| v.clone())
            .unwrap_or_default();
        
        let utxos = deserialize_utxos(&utxo_data).await;
        
        // Filter for spendable UTXOs (maturity rules)
        let spendable_utxos = filter_spendable_utxos(
            utxos.clone(),
            db.clone(),
            current_height,
        ).await;
        
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
            .map_err(|e| format!("Task join error: {}", e))?
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
            .and_then(|opt| opt.as_ref())
            .map(|v| v.clone())
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
            .map_err(|e| format!("Task join error: {}", e))?
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
            // Reverse to internal format for database lookup
            let txid_internal: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
            let mut key = vec![b't'];
            key.extend(&txid_internal);
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
    
    let tx_count = unique_txids.len() as u32;
    let total_pages = ((tx_count as f64) / (params.page_size as f64)).ceil() as u32;
    let total_pages = if total_pages == 0 { 1 } else { total_pages };
    
    // Determine response details based on params
    let txids = if params.details == "txids" {
        Some(unique_txids.clone())
    } else {
        None
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
        let tokens_page = params.tokens_page;
        let tokens_page_size = params.tokens_page_size;
        let total_tokens_pages = ((total_tokens as f64) / (tokens_page_size as f64)).ceil() as u32;
        let total_tokens_pages = if total_tokens_pages == 0 { 1 } else { total_tokens_pages };
        
        let start_idx = ((tokens_page - 1) * tokens_page_size) as usize;
        let end_idx = (start_idx + tokens_page_size as usize).min(filtered_addresses.len());
        
        let paginated_tokens: Vec<_> = if start_idx < filtered_addresses.len() {
            filtered_addresses[start_idx..end_idx].to_vec()
        } else {
            Vec::new()
        };
        
        (Some(paginated_tokens), Some(total_tokens), Some(tokens_page), Some(total_tokens_pages))
    } else {
        (None, None, None, None)
    };
    
    let used_tokens = if params.details == "txs" || params.details == "tokens" || params.details == "tokenBalances" {
        Some(used_addresses.len() as u32)
    } else {
        None
    };
    
    // Aggregate totals
    let xpub_total_received: i64 = used_addresses.iter().map(|(_, _, _, _, total_recv, _)| total_recv).sum();
    let xpub_total_sent: i64 = used_addresses.iter().map(|(_, _, _, _, _, total_snt)| total_snt).sum();
    let xpub_balance: i64 = used_addresses.iter().map(|(_, _, _, balance, _, _)| balance).sum();
    
    Ok(XPubInfo {
        page: params.page,
        total_pages,
        items_on_page: params.page_size,
        address: xpub_str.to_string(),
        balance: xpub_balance.to_string(),
        total_received: xpub_total_received.to_string(),
        total_sent: xpub_total_sent.to_string(),
        unconfirmed_balance: "0".to_string(),
        unconfirmed_txs: 0,
        txs: tx_count,
        txids,
        tokens: tokens.0,
        transactions: None, // TODO: Implement full tx details if needed
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
) -> Json<Vec<UTXO>> {
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
        Ok(utxos) => Json(utxos),
        Err(_) => Json(vec![]),
    }
}

async fn compute_utxos(
    db: &Arc<DB>,
    address: &str,
) -> Result<Vec<UTXO>, Box<dyn std::error::Error + Send + Sync>> {
    let key = format!("a{}", address);
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
    let spendable_utxos = filter_spendable_utxos(
        unspent_utxos.clone(),
        Arc::clone(db),
        current_height,
    ).await;
    
    let mut utxo_list = Vec::new();
    
    for (txid_hash, vout) in &spendable_utxos {
        // txid_hash is already in display order (per tx_keys.rs format), no reversal needed
        let txid_display = hex::encode(txid_hash);
        
        // Look up transaction to get value, confirmations, and other details
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
                
                // Skip orphaned and unresolved transactions
                if block_height == HEIGHT_ORPHAN || block_height == HEIGHT_UNRESOLVED {
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
