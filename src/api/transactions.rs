// Transaction-Related API Endpoints
//
// Endpoints for querying and broadcasting transactions.
// Confirmed transactions are immutable and cached heavily.

use axum::{Json, Extension, extract::Path, http::StatusCode};
use rocksdb::DB;
use pivx_rpc_rs::BitcoinRpcClient;
use std::sync::Arc;
use std::time::Duration;

use crate::cache::CacheManager;
use crate::chain_state::get_chain_state;
use crate::config::get_global_config;
use crate::parser::{get_script_type, deserialize_transaction_blocking};
use super::types::{BlockbookError, SendTxResponse, TxError};
use super::helpers::format_piv_amount;

pub use axum::extract::Path as AxumPath;

/// GET /api/v2/tx/{txid}
/// Returns full transaction details with inputs, outputs, and Sapling data.
/// 
/// **CACHED**: 300 second TTL (confirmed transactions are immutable)
pub async fn tx_v2(
    AxumPath(txid): AxumPath<String>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<BlockbookError>)> {
    let cache_key = format!("tx:{}", txid);
    let db_clone = Arc::clone(&db);
    let txid_clone = txid.clone();
    
    let result = cache
        .get_or_compute(
            &cache_key,
            Duration::from_secs(300),
            || async move {
                compute_transaction_details(&db_clone, &txid_clone).await
            }
        )
        .await;
    
    match result {
        Ok(tx) => Ok(Json(tx)),
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(BlockbookError::new(e.to_string()))
        )),
    }
}

async fn compute_transaction_details(
    db: &Arc<DB>,
    txid: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let db_clone = Arc::clone(db);
    let txid_clone = txid.to_string();
    
    tokio::task::spawn_blocking(move || {
        // Transaction key: 't' + txid_bytes_reversed (internal format)
        let txid_bytes = hex::decode(&txid_clone)?;
        
        // Try reversed format first (new/correct format)
        let txid_reversed: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
        let mut key = vec![b't'];
        key.extend_from_slice(&txid_reversed);
        
        let cf_transactions = db_clone
            .cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        
        let data = if let Ok(Some(d)) = db_clone.get_cf(&cf_transactions, &key) {
            d
        } else {
            // Fallback: try display format (old/incorrect format for migration)
            let mut key_display = vec![b't'];
            key_display.extend_from_slice(&txid_bytes);
            db_clone
                .get_cf(&cf_transactions, &key_display)?
                .ok_or("Transaction not found")?
        };
        
        // Data format: block_version (4 bytes) + height (4 bytes) + raw_tx_bytes
        if data.len() < 8 {
            return Err("Invalid transaction data".into());
        }
        
        if data.len() > 10_000_000 {
            return Err("Transaction data too large".into());
        }
        
        let _block_version = u32::from_le_bytes(data[0..4].try_into().unwrap_or([0; 4]));
        let block_height = i32::from_le_bytes(data[4..8].try_into().unwrap_or([0; 4]));
        
        let tx_data_len = data.len() - 8;
        if tx_data_len == 0 {
            return Err("Empty transaction data".into());
        }
        
        let mut tx_data_with_header = Vec::with_capacity(4 + tx_data_len);
        tx_data_with_header.extend_from_slice(&[0u8; 4]); // Dummy block_version
        tx_data_with_header.extend_from_slice(&data[8..]); // Actual tx data
        
        // Parse the binary transaction data
        let tx = deserialize_transaction_blocking(&tx_data_with_header)?;
        
        // Convert to JSON format (Blockbook-compatible)
        let mut vin = Vec::new();
        let mut value_in: i64 = 0;
        
        for (idx, input) in tx.inputs.iter().enumerate() {
            if let Some(coinbase_data) = &input.coinbase {
                // Coinbase transaction
                vin.push(serde_json::json!({
                    "coinbase": hex::encode(coinbase_data),
                    "sequence": input.sequence,
                    "n": idx,
                }));
            } else if let Some(prevout) = &input.prevout {
                // Regular input - look up previous output for value and address
                let mut input_json = serde_json::json!({
                    "txid": prevout.hash.clone(),
                    "vout": prevout.n,
                    "sequence": input.sequence,
                    "n": idx,
                });
                
                // Try to get value and address from previous transaction
                if let Ok(prev_txid_bytes) = hex::decode(&prevout.hash) {
                    let reversed: Vec<u8> = prev_txid_bytes.iter().rev().cloned().collect();
                    let mut prev_key = vec![b't'];
                    prev_key.extend_from_slice(&reversed);
                    
                    let prev_data_opt = if let Ok(Some(d)) = db_clone.get_cf(&cf_transactions, &prev_key) {
                        Some(d)
                    } else {
                        // Fallback: try display format
                        let mut prev_key_display = vec![b't'];
                        prev_key_display.extend_from_slice(&prev_txid_bytes);
                        db_clone.get_cf(&cf_transactions, &prev_key_display).ok().flatten()
                    };
                    
                    if let Some(prev_data) = prev_data_opt {
                        if prev_data.len() > 10_000_000 {
                            eprintln!("Warning: Previous transaction data too large for {}", prevout.hash);
                        } else if prev_data.len() >= 8 {
                            let prev_tx_data_len = prev_data.len() - 8;
                            if prev_tx_data_len > 0 {
                                let mut prev_tx_data_with_header = Vec::with_capacity(4 + prev_tx_data_len);
                                prev_tx_data_with_header.extend_from_slice(&[0u8; 4]);
                                prev_tx_data_with_header.extend_from_slice(&prev_data[8..]);
                            
                                if let Ok(prev_tx) = deserialize_transaction_blocking(&prev_tx_data_with_header) {
                                    if let Some(output) = prev_tx.outputs.get(prevout.n as usize) {
                                        input_json["value"] = serde_json::json!(format_piv_amount(output.value));
                                        value_in += output.value;
                                        if !output.address.is_empty() {
                                            input_json["addresses"] = serde_json::json!(output.address.clone());
                                            input_json["isAddress"] = serde_json::json!(true);
                                        }
                                        
                                        let script_type = get_script_type(&output.script_pubkey.script);
                                        input_json["type"] = serde_json::json!(script_type);
                                    }
                                }
                            }
                        }
                    }
                }
                
                vin.push(input_json);
            }
        }
        
        let mut vout = Vec::new();
        let mut value_out: i64 = 0;
        
        for (idx, output) in tx.outputs.iter().enumerate() {
            value_out += output.value;
            let script_type = get_script_type(&output.script_pubkey.script);
            vout.push(serde_json::json!({
                "value": format_piv_amount(output.value),
                "n": idx,
                "hex": hex::encode(&output.script_pubkey.script),
                "addresses": output.address,
                "isAddress": !output.address.is_empty(),
                "type": script_type,
            }));
        }
        
        let tx_size = data.len() - 8;
        let tx_vsize = tx_size;
        
        // Get block hash and time if we have a valid height
        let (block_hash, block_time) = if block_height > 0 {
            let height_key = (block_height as u32).to_le_bytes().to_vec();
            
            if let Some(cf_metadata) = db_clone.cf_handle("chain_metadata") {
                if let Ok(Some(hash_bytes)) = db_clone.get_cf(&cf_metadata, &height_key) {
                    let hash_hex = hex::encode(&hash_bytes);
                    
                    if let Some(cf_blocks) = db_clone.cf_handle("blocks") {
                        let internal_hash: Vec<u8> = hash_bytes.iter().rev().cloned().collect();
                        if let Ok(Some(header_bytes)) = db_clone.get_cf(&cf_blocks, &internal_hash) {
                            if header_bytes.len() >= 72 {
                                let time = u32::from_le_bytes(
                                    header_bytes[68..72].try_into().unwrap_or([0; 4])
                                ) as u64;
                                (hash_hex, time)
                            } else {
                                (hash_hex, 0)
                            }
                        } else {
                            (hash_hex, 0)
                        }
                    } else {
                        (hash_hex, 0)
                    }
                } else {
                    (String::new(), 0)
                }
            } else {
                (String::new(), 0)
            }
        } else {
            (String::new(), 0)
        };
        
        // Get current height for confirmations
        let chain_state = get_chain_state(&db_clone).ok();
        let current_height = chain_state.as_ref().map(|cs| cs.height).unwrap_or(0);
        
        let confirmations = if block_height > 0 && current_height > 0 {
            (current_height - block_height + 1).max(0) as u32
        } else {
            0
        };
        
        // Calculate fees (for non-coinbase)
        let fees = if value_in > 0 && value_in >= value_out {
            value_in - value_out
        } else {
            0
        };
        
        // Convert Sapling data if present
        let sapling_json = tx.sapling_data.as_ref().map(|sap| {
            let spends: Vec<_> = sap.vshielded_spend.iter().map(|spend| serde_json::json!({
                "cv": hex::encode(spend.cv),
                "anchor": hex::encode(spend.anchor),
                "nullifier": hex::encode(spend.nullifier),
                "rk": hex::encode(spend.rk),
                "zkproof": hex::encode(spend.zkproof),
                "spend_auth_sig": hex::encode(spend.spend_auth_sig),
            })).collect();
            
            let outputs: Vec<_> = sap.vshielded_output.iter().map(|output| serde_json::json!({
                "cv": hex::encode(output.cv),
                "cmu": hex::encode(output.cmu),
                "ephemeral_key": hex::encode(output.ephemeral_key),
                "enc_ciphertext": hex::encode(output.enc_ciphertext),
                "out_ciphertext": hex::encode(output.out_ciphertext),
                "zkproof": hex::encode(output.zkproof),
            })).collect();
            
            // Determine transaction type based on value_balance
            let tx_type = if sap.value_balance < 0 {
                "shielding" // Transparent → Shielded (negative balance means adding to shield pool)
            } else if sap.value_balance > 0 {
                "unshielding" // Shielded → Transparent (positive balance means removing from shield pool)
            } else {
                "shielded_transfer" // Shielded → Shielded (zero balance means pure shielded transfer)
            };
            
            serde_json::json!({
                "value_balance": format_piv_amount(sap.value_balance),
                "value_balance_sat": sap.value_balance,
                "shielded_spend_count": sap.vshielded_spend.len(),
                "shielded_output_count": sap.vshielded_output.len(),
                "transaction_type": tx_type,
                "binding_sig": hex::encode(sap.binding_sig),
                "spends": spends,
                "outputs": outputs,
            })
        });
        
        let mut tx_json = serde_json::json!({
            "txid": tx.txid,
            "version": tx.version,
            "lockTime": tx.lock_time,
            "vin": vin,
            "vout": vout,
            "blockHash": block_hash,
            "blockHeight": block_height,
            "confirmations": confirmations,
            "blockTime": block_time,
            "value": format_piv_amount(value_out),
            "valueIn": format_piv_amount(value_in),
            "fees": format_piv_amount(fees),
            "size": tx_size,
            "vsize": tx_vsize,
            "hex": hex::encode(&data[8..]),
        });
        
        if let Some(sapling) = sapling_json {
            tx_json["sapling"] = sapling;
        }
        
        Ok(tx_json)
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
}

/// GET /api/v2/sendtx/{hex}
/// Legacy endpoint for broadcasting raw transactions.
/// 
/// **NO CACHE**: Write operation
pub async fn send_tx_v2(
    AxumPath(param): AxumPath<String>
) -> Result<Json<SendTxResponse>, (StatusCode, Json<BlockbookError>)> {
    send_transaction_internal(param).await
}

/// POST /api/v2/sendtx
/// Blockbook-compatible endpoint for broadcasting transactions.
/// Accepts raw transaction hex in request body (plain text or JSON).
/// 
/// **NO CACHE**: Write operation
pub async fn send_tx_post_v2(
    body: String
) -> Result<Json<SendTxResponse>, (StatusCode, Json<BlockbookError>)> {
    // Body can be either plain hex or JSON {"hex": "..."}
    let tx_hex = if body.trim().starts_with('{') {
        match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(json) => {
                json.get("hex")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&body)
                    .trim()
                    .to_string()
            },
            Err(_) => body.trim().to_string(),
        }
    } else {
        body.trim().to_string()
    };
    
    send_transaction_internal(tx_hex).await
}

async fn send_transaction_internal(
    tx_hex: String
) -> Result<Json<SendTxResponse>, (StatusCode, Json<BlockbookError>)> {
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host");
    let rpc_user = config.get::<String>("rpc.user");
    let rpc_pass = config.get::<String>("rpc.pass");

    let client = BitcoinRpcClient::new(
        rpc_host.unwrap_or_else(|_| "127.0.0.1:9998".to_string()),
        Some(rpc_user.unwrap_or_default()),
        Some(rpc_pass.unwrap_or_default()),
        3,    // Max retries
        10,   // Connection timeout
        1000, // Read/write timeout
    );

    let result = client.sendrawtransaction(&tx_hex, Some(false));

    match result {
        Ok(txid) => {
            let response = SendTxResponse {
                result: Some(txid),
                error: None, 
            };
            Ok(Json(response))
        },
        Err(e) => {
            Err((
                StatusCode::BAD_REQUEST,
                Json(BlockbookError::new(format!("Failed to send transaction: {}", e)))
            ))
        },
    }
}
