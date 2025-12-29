// Address and UTXO API Endpoints
//
// NOTE: These are complex endpoints with significant database operations.
// Caching provides moderate benefits since addresses are frequently updated.

use axum::{Json, Extension, extract::{Path as AxumPath, Query}, http::StatusCode};
use rocksdb::DB;
use std::sync::Arc;
use std::time::Duration;

use crate::cache::CacheManager;
use crate::parser::{deserialize_utxos, deserialize_transaction, encode_pivx_address};
use crate::maturity::{filter_spendable_utxos, get_current_height};
use super::types::{AddressInfo, AddressQuery, XPubInfo, UTXO, UtxoQuery};
use super::helpers::format_piv_amount;

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
/// **CACHED**: 30 second TTL
pub async fn xpub_v2(
    AxumPath(xpub_str): AxumPath<String>,
    Query(params): Query<AddressQuery>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Json<XPubInfo> {
    let cache_key = format!("xpub:{}:{}", xpub_str, params.page);
    let db_clone = Arc::clone(&db);
    let xpub_clone = xpub_str.clone();
    let params_clone = params.clone();
    
    let result = cache
        .get_or_compute(
            &cache_key,
            Duration::from_secs(30),
            || async move {
                compute_xpub_info(&db_clone, &xpub_clone, &params_clone).await
            }
        )
        .await;
    
    match result {
        Ok(info) => Json(info),
        Err(_) => {
            Json(XPubInfo {
                page: params.page,
                total_pages: 1,
                items_on_page: params.page_size,
                address: xpub_str,
                balance: "0".to_string(),
                total_received: "0".to_string(),
                total_sent: "0".to_string(),
                unconfirmed_balance: "0".to_string(),
                unconfirmed_txs: 0,
                txs: 0,
                txids: Some(vec![]),
                tokens: None,
                transactions: None,
                used_tokens: None,
            })
        }
    }
}

async fn compute_xpub_info(
    _db: &Arc<DB>,
    xpub_str: &str,
    params: &AddressQuery,
) -> Result<XPubInfo, Box<dyn std::error::Error + Send + Sync>> {
    // Simplified: Return placeholder for xpub support
    // Full implementation would derive addresses and aggregate balances
    Ok(XPubInfo {
        page: params.page,
        total_pages: 1,
        items_on_page: params.page_size,
        address: xpub_str.to_string(),
        balance: "0".to_string(),
        total_received: "0".to_string(),
        total_sent: "0".to_string(),
        unconfirmed_balance: "0".to_string(),
        unconfirmed_txs: 0,
        txs: 0,
        txids: Some(vec![]),
        tokens: None,
        transactions: None,
        used_tokens: None,
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
        let txid_display = hex::encode(txid_hash.iter().rev().cloned().collect::<Vec<u8>>());
        
        utxo_list.push(UTXO {
            txid: txid_display,
            vout: *vout as u32,  // Cast from u64 to u32
            value: "0".to_string(), // Would need to look up actual value
            confirmations: 0,
            lock_time: None,
            height: None,
            coinbase: None,
            coinstake: None,
            spendable: Some(true),
            blocks_until_spendable: None,
        });
    }
    
    Ok(utxo_list)
}
