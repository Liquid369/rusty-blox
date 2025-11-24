use pivx_rpc_rs::{BitcoinRpcClient, MasternodeList, BudgetInfo as RpcBudgetInfo};
use axum::{
    extract::{Path, Extension, Query},
    Json,
    http::StatusCode,
};
use rocksdb::DB;
use hex;
use crate::parser::{deserialize_transaction, deserialize_utxos};
use crate::config::get_global_config;
use crate::chain_state::{get_chain_state, ChainState};
use crate::search::{search, SearchResult};
use crate::mempool::{MempoolState, MempoolInfo};
use crate::maturity::{filter_spendable_utxos, get_current_height};
use serde::{Deserialize, Serialize};
use serde_json;
use std::sync::Arc;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub use axum::extract::Path as AxumPath;
#[derive(Serialize, Deserialize, Debug)]
pub struct XPubInfo {
    pub page: u32,
    #[serde(rename = "totalPages")]
    pub total_pages: u32,
    #[serde(rename = "itemsOnPage")]
    pub items_on_page: u32,
    pub address: String,
    pub balance: String,
    #[serde(rename = "totalReceived")]
    pub total_received: String,
    #[serde(rename = "totalSent")]
    pub total_sent: String,
    #[serde(rename = "unconfirmedBalance")]
    pub unconfirmed_balance: String,
    #[serde(rename = "unconfirmedTxs")]
    pub unconfirmed_txs: u32,
    pub txs: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub txids: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AddressInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "totalPages")]
    pub total_pages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "itemsOnPage")]
    pub items_on_page: Option<u32>,
    pub address: String,
    pub balance: String,
    #[serde(rename = "totalReceived")]
    pub total_received: String,
    #[serde(rename = "totalSent")]
    pub total_sent: String,
    #[serde(rename = "unconfirmedBalance")]
    pub unconfirmed_balance: String,
    #[serde(rename = "unconfirmedTxs")]
    pub unconfirmed_txs: u32,
    pub txs: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub txids: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UTXO {
    pub txid: String,
    pub vout: u32,
    pub value: String,
    pub confirmations: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "lockTime")]
    pub lock_time: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coinbase: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coinstake: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spendable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "blocksUntilSpendable")]
    pub blocks_until_spendable: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MoneySupply {
    pub moneysupply: f64,
    pub transparentsupply: f64,
    pub shieldsupply: f64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MasternodeInfo {
    pub rank: u32,
    #[serde(rename = "type")]
    pub masternode_type: String,
    pub network: String,
    pub txhash: String,
    pub outidx: u32,
    pub pubkey: String,
    pub status: String,
    pub addr: String,
    pub version: u32,
    pub lastseen: u64,
    pub activetime: u64,
    pub lastpaid: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MNList {
    pub masternodes: Vec<MasternodeInfo>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MNCount {
    pub total: i32,
    pub stable: i32,
    pub enabled: i32,
    pub inqueue: i32,
    pub ipv4: i32,
    pub ipv6: i32,
    pub onion: i32,
}

#[derive(Debug, Deserialize)]
pub struct BlockQuery {
    block_hash: Option<String>,
    block_height: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Transaction {
    pub txid: String,
    pub vin: Vec<TxInput>,
    pub vout: Vec<TxOutput>,
    #[serde(rename = "blockHash")]
    pub block_hash: String,
    #[serde(rename = "blockHeight")]
    pub block_height: u32,
    pub confirmations: u32,
    #[serde(rename = "blockTime")]
    pub block_time: u64,
    pub value: String,
    #[serde(rename = "valueIn")]
    pub value_in: String,
    pub fees: String,
    pub hex: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TxInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub txid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vout: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
    pub n: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isAddress")]
    pub is_address: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hex: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TxOutput {
    pub value: String,
    pub n: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isAddress")]
    pub is_address: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spent: Option<bool>,
}

// Query parameters for address and xpub endpoints
#[derive(Debug, Deserialize)]
pub struct AddressQuery {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    #[serde(rename = "pageSize")]
    pub page_size: u32,
    pub from: Option<u32>,
    pub to: Option<u32>,
    #[serde(default = "default_details")]
    pub details: String,
    pub contract: Option<String>,
    pub secondary: Option<String>,
}

fn default_page() -> u32 { 1 }
fn default_page_size() -> u32 { 1000 }
fn default_details() -> String { "txids".to_string() }

// Query parameters for UTXO endpoint
#[derive(Debug, Deserialize)]
pub struct UtxoQuery {
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SendTxResponse {
    pub result: Option<String>,
    pub error: Option<TxError>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TxError {
    pub message: String,
}

#[derive(Serialize)]
pub struct BlockHash {
    #[serde(rename = "blockHash")]
    pub block_hash: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RelayMNB {
    pub hexstring: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BudgetInfo {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "URL")]
    pub url: String,
    #[serde(rename = "Hash")]
    pub hash: String,
    #[serde(rename = "FeeHash")]
    pub fee_hash: String,
    #[serde(rename = "BlockStart")]
    pub block_start: u32,
    #[serde(rename = "BlockEnd")]
    pub block_end: u32,
    #[serde(rename = "TotalPaymentCount")]
    pub total_payment_count: u32,
    #[serde(rename = "RemainingPaymentCount")]
    pub remaining_payment_count: u32,
    #[serde(rename = "PaymentAddress")]
    pub payment_address: String,
    #[serde(rename = "Ratio")]
    pub ratio: f64,
    #[serde(rename = "Yeas")]
    pub yeas: u32,
    #[serde(rename = "Nays")]
    pub nays: u32,
    #[serde(rename = "Abstains")]
    pub abstains: u32,
    #[serde(rename = "TotalPayment")]
    pub total_payment: f64,
    #[serde(rename = "MonthlyPayment")]
    pub monthly_payment: f64,
    #[serde(rename = "IsEstablished")]
    pub is_established: bool,
    #[serde(rename = "IsValid")]
    pub is_valid: bool,
    #[serde(rename = "Allotted")]
    pub allotted: f64,
}

#[derive(Debug, Deserialize)]
pub struct BlockParams {
    pub block_height: i32,
}

pub async fn root_handler() -> &'static str {
    "Welcome to the PIVX Rusty Blox"
}

pub async fn api_handler() -> &'static str {
    "API response"
}

pub async fn block_index_v2(Path(param): Path<String>, Extension(db): Extension<Arc<DB>>) -> Json<BlockHash> {
    // Parse height and convert to bytes (matching how it's stored in blocks.rs)
    let height: u32 = match param.parse() {
        Ok(h) => h,
        Err(_) => return Json(BlockHash{block_hash: "Invalid height".to_string()}),
    };
    
    let key = height.to_le_bytes().to_vec();
    
    // Query the "blocks" column family
    let result = match db.cf_handle("blocks") {
        Some(cf) => {
            match db.get_cf(&cf, &key) {
                Ok(Some(value)) => hex::encode(&value),
                _ => "Not found".to_string(),
            }
        },
        None => "Blocks CF not found".to_string(),
    };

    Json(BlockHash{block_hash: result})
}

pub async fn tx_v2(AxumPath(txid): AxumPath<String>, Extension(db): Extension<Arc<DB>>) -> Json<serde_json::Value> {
    let db_clone = Arc::clone(&db);
    let txid_clone = txid.clone();
    
    let result = tokio::task::spawn_blocking(move || {
        // Transaction key: 't' + txid_bytes_reversed (internal format)
        // The txid from URL is in display format, need to reverse for database lookup
        let txid_bytes = match hex::decode(&txid_clone) {
            Ok(bytes) => bytes,
            Err(_) => return Err("Invalid transaction ID format"),
        };
        
        // Try reversed format first (new/correct format)
        let txid_reversed: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
        let mut key = vec![b't'];
        key.extend_from_slice(&txid_reversed);
        
        let cf_transactions = db_clone.cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        
        let data = if let Ok(Some(d)) = db_clone.get_cf(&cf_transactions, &key) {
            d
        } else {
            // Fallback: try display format (old/incorrect format for migration)
            let mut key_display = vec![b't'];
            key_display.extend_from_slice(&txid_bytes);
            db_clone.get_cf(&cf_transactions, &key_display)
                .map_err(|_| "Database error")?
                .ok_or("Transaction not found")?
        };
        
        // Data format: block_version (4 bytes) + height (4 bytes) + raw_tx_bytes
        if data.len() < 8 {
            return Err("Invalid transaction data");
        }
        
        // Sanity check: max transaction size is 10MB (reasonable limit)
        if data.len() > 10_000_000 {
            return Err("Transaction data too large");
        }
        
        let _block_version = u32::from_le_bytes(data[0..4].try_into().unwrap_or([0; 4]));
        let block_height = i32::from_le_bytes(data[4..8].try_into().unwrap_or([0; 4]));
        
        // Transaction data starts at byte 8 (after block_version and height)
        // We need to prepend a dummy block_version for the parser
        let tx_data_len = data.len() - 8;
        if tx_data_len == 0 {
            return Err("Empty transaction data");
        }
        
        let mut tx_data_with_header = Vec::with_capacity(4 + tx_data_len);
        tx_data_with_header.extend_from_slice(&[0u8; 4]); // Dummy block_version
        tx_data_with_header.extend_from_slice(&data[8..]); // Actual tx data
        
        // Parse the binary transaction data
    use crate::parser::get_script_type;
        let tx = crate::parser::deserialize_transaction_blocking(&tx_data_with_header)
            .map_err(|_| "Failed to parse transaction")?;
        
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
                    // Reverse for database lookup (same as main tx lookup)
                    let reversed: Vec<u8> = prev_txid_bytes.iter().rev().cloned().collect();
                    let mut prev_key = vec![b't'];
                    prev_key.extend_from_slice(&reversed);
                    
                    
                    let prev_data_opt = if let Ok(Some(d)) = db_clone.get_cf(&cf_transactions, &prev_key) {
                        Some(d)
                    } else {
                        // Fallback: try display format (old/incorrect format for migration)
                        let mut prev_key_display = vec![b't'];
                        prev_key_display.extend_from_slice(&prev_txid_bytes);
                        db_clone.get_cf(&cf_transactions, &prev_key_display).ok().flatten()
                    };
                    
                    if let Some(prev_data) = prev_data_opt {
                        // Validate previous transaction data size
                        if prev_data.len() > 10_000_000 {
                            eprintln!("Warning: Previous transaction data too large for {}", prevout.hash);
                        } else if prev_data.len() >= 8 {
                            let prev_tx_data_len = prev_data.len() - 8;
                            if prev_tx_data_len == 0 {
                                eprintln!("Warning: Empty previous transaction data for {}", prevout.hash);
                            } else {
                                // Parse previous transaction (skip block_version + height)
                                let mut prev_tx_data_with_header = Vec::with_capacity(4 + prev_tx_data_len);
                                prev_tx_data_with_header.extend_from_slice(&[0u8; 4]);
                                prev_tx_data_with_header.extend_from_slice(&prev_data[8..]);
                            
                            if let Ok(prev_tx) = crate::parser::deserialize_transaction_blocking(&prev_tx_data_with_header) {
                                if let Some(output) = prev_tx.outputs.get(prevout.n as usize) {
                                    input_json["value"] = serde_json::json!(output.value.to_string());
                                    value_in += output.value;
                                    if !output.address.is_empty() {
                                        input_json["addresses"] = serde_json::json!(output.address.clone());
                                        input_json["isAddress"] = serde_json::json!(true);
                                    }
                                    
                                    // Add script type from previous output
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
                "value": output.value.to_string(),
                "n": idx,
                "hex": hex::encode(&output.script_pubkey.script),
                "addresses": output.address,
                "isAddress": !output.address.is_empty(),
                "type": script_type,
            }));
        }
        
        let tx_size = data.len() - 8; // Subtract the 8 header bytes
        
        // Get current height for confirmations
        let chain_state = get_chain_state(&db_clone).ok();
        let current_height = chain_state.as_ref().map(|cs| cs.height).unwrap_or(0);
        
        // Note: Some transactions may have block_height = 0 due to indexing issues during initial sync
        // These will show 0 confirmations until the transaction index is rebuilt
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
        
        // Convert Sapling data if present with detailed spend/output info
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
            
            serde_json::json!({
                "value_balance": sap.value_balance.to_string(),
                "shielded_spend_count": sap.vshielded_spend.len(),
                "shielded_output_count": sap.vshielded_output.len(),
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
            "blockHeight": block_height,
            "confirmations": confirmations,
            "blockTime": 0, // TODO: get from block data
            "value": value_out.to_string(),
            "valueIn": value_in.to_string(),
            "fees": fees.to_string(),
            "size": tx_size,
        });
        
        // Add sapling field if present
        if let Some(sapling) = sapling_json {
            tx_json["sapling"] = sapling;
        }
        
        Ok(tx_json)
    })
    .await;
    
    match result {
        Ok(Ok(tx)) => Json(tx),
        Ok(Err(e)) => Json(serde_json::json!({"error": e})),
        Err(_) => Json(serde_json::json!({"error": "Internal server error"})),
    }
}

pub async fn addr_v2(
    AxumPath(address): AxumPath<String>, 
    Query(params): Query<AddressQuery>,
    Extension(db): Extension<Arc<DB>>
) -> Json<AddressInfo> {
    
    let key = format!("a{}", address);
    let key_bytes = key.as_bytes().to_vec();
    let db_clone = db.clone();
    
    // Get from addr_index column family in blocking task
    let result = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, String> {
        let cf_addr_index = db_clone.cf_handle("addr_index")
            .ok_or_else(|| "addr_index CF not found".to_string())?;
        db_clone.get_cf(&cf_addr_index, &key_bytes)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or(Ok(None))
    .unwrap_or(None)
    .unwrap_or_else(std::vec::Vec::new);
    
    // The data is in simple format (no spent flags) - Bitcoin Core approach
    // Entries are removed when spent, so everything here is unspent
    let unspent_utxos = deserialize_utxos(&result).await;
    
    // Get transaction list for this address (key: 't' + address)
    let tx_list_key = format!("t{}", address);
    let tx_list_key_bytes = tx_list_key.as_bytes().to_vec();
    let db_clone = db.clone();
    
    let tx_list_data = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, String> {
        let cf_addr_index = db_clone.cf_handle("addr_index")
            .ok_or_else(|| "addr_index CF not found".to_string())?;
        db_clone.get_cf(&cf_addr_index, &tx_list_key_bytes)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or(Ok(None))
    .unwrap_or(None)
    .unwrap_or_else(std::vec::Vec::new);
    
    // Parse transaction list (32 bytes per txid, reversed format)
    let all_txids: Vec<Vec<u8>> = tx_list_data.chunks_exact(32)
        .map(|chunk| chunk.to_vec())
        .collect();
    
    // Get current chain height for maturity checks
    let current_height = get_current_height(&db).unwrap_or(0);
    
    // Filter UTXOs by maturity rules (coinbase/coinstake must meet maturity requirements)
    let spendable_utxos = filter_spendable_utxos(
        unspent_utxos.clone(),
        db.clone(),
        current_height,
    ).await;
    
    let mut balance: i64 = 0;
    let mut total_received: i64 = 0;
    
    // Calculate balance from SPENDABLE unspent UTXOs only
    for utxo in &spendable_utxos {
        let (txid_hash, output_index) = utxo;
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
        .unwrap_or(Ok(None))
        .unwrap_or(None);
        
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
    
    // Calculate total received from ALL transactions
    for txid_internal in &all_txids {
        let mut key = vec![b't'];
        key.extend(txid_internal);
        let key_clone = key.clone();
        let db_clone = db.clone();
        let address_clone = address.clone();
        
        let tx_data = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, String> {
            let cf_transactions = db_clone.cf_handle("transactions")
                .ok_or_else(|| "transactions CF not found".to_string())?;
            db_clone.get_cf(&cf_transactions, &key_clone)
                .map_err(|e| e.to_string())
        })
        .await
        .unwrap_or(Ok(None))
        .unwrap_or(None);
        
        if let Some(tx_data) = tx_data {
            if tx_data.len() >= 8 {
                let tx_data_len = tx_data.len() - 8;
                if tx_data_len > 0 {
                    let mut tx_data_with_header = Vec::with_capacity(4 + tx_data_len);
                    tx_data_with_header.extend_from_slice(&[0u8; 4]);
                    tx_data_with_header.extend_from_slice(&tx_data[8..]);
                
                    if let Ok(tx) = deserialize_transaction(&tx_data_with_header).await {
                        // Sum all outputs to this address
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
    
    // Use the transaction list for history (all transactions)
    // Convert from internal format (reversed) to display format
    let unique_txids: Vec<String> = all_txids.iter()
        .map(|txid_internal| {
            let mut txid_display = txid_internal.clone();
            txid_display.reverse();
            hex::encode(&txid_display)
        })
        .collect();
    
    // Sort txids by block height (newest first = least confirmations first)
    let mut txid_heights: Vec<(String, u32)> = Vec::new();
    for txid in &unique_txids {
        let txid_bytes = match hex::decode(txid) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let mut key = vec![b't'];
        key.extend(&txid_bytes);
        let db_clone = db.clone();
        
        let height = tokio::task::spawn_blocking(move || -> Option<u32> {
            let cf_transactions = db_clone.cf_handle("transactions")?;
            let tx_data = db_clone.get_cf(&cf_transactions, &key).ok()??;
            if tx_data.len() >= 8 {
                let height_bytes: [u8; 4] = tx_data[4..8].try_into().ok()?;
                Some(u32::from_le_bytes(height_bytes))
            } else {
                None
            }
        })
        .await
        .ok()
        .flatten()
        .unwrap_or(0);
        
        txid_heights.push((txid.clone(), height));
    }
    
    // Sort by height descending (newest first)
    txid_heights.sort_by(|a, b| b.1.cmp(&a.1));
    let sorted_txids: Vec<String> = txid_heights.iter().map(|(txid, _)| txid.clone()).collect();
    
    let tx_count = sorted_txids.len() as u32;
    let total_pages = ((tx_count as f64) / (params.page_size as f64)).ceil() as u32;
    let total_pages = if total_pages == 0 { 1 } else { total_pages };
    
    // Return txids only if details >= "txids"
    let txids = if params.details == "basic" || params.details == "tokens" || params.details == "tokenBalances" {
        None
    } else {
        Some(sorted_txids)
    };
    
    Json(AddressInfo {
        page: Some(params.page),
        total_pages: Some(total_pages),
        items_on_page: Some(params.page_size),
        address,
        balance: balance.to_string(),
        total_received: total_received.to_string(),
        total_sent: total_sent.to_string(),
        unconfirmed_balance: "0".to_string(), // TODO: track mempool
        unconfirmed_txs: 0, // TODO: track mempool
        txs: tx_count,
        txids,
    })
}

pub async fn xpub_v2(
    AxumPath(xpub): AxumPath<String>,
    Query(params): Query<AddressQuery>,
    Extension(db): Extension<Arc<DB>>
) -> Json<XPubInfo> {
    
    let key = format!("p{}", xpub);
    let key_bytes = key.as_bytes().to_vec();
    let db_clone = db.clone();
    
    // Get pubkey CF in blocking task
    let result = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, String> {
        let cf_pubkey = db_clone.cf_handle("pubkey")
            .ok_or_else(|| "pubkey CF not found".to_string())?;
        db_clone.get_cf(&cf_pubkey, &key_bytes)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or(Ok(None))
    .unwrap_or(None)
    .unwrap_or_else(std::vec::Vec::new);

    let utxos = deserialize_utxos(&result).await;
    
    // Get current chain height for maturity checks
    let current_height = get_current_height(&db).unwrap_or(0);
    
    // Filter UTXOs by maturity rules
    let spendable_utxos = filter_spendable_utxos(
        utxos.clone(),
        db.clone(),
        current_height,
    ).await;
    
    let mut balance: i64 = 0;
    
    for utxo in &spendable_utxos {
        let (txid_hash, output_index) = utxo;
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
        .unwrap_or(Ok(None))
        .unwrap_or(None);
        
        if let Some(tx_data) = tx_data {
            // Validate transaction data size for xpub UTXO lookup
            if tx_data.len() > 10_000_000 {
                eprintln!("Warning: xpub UTXO transaction data too large");
            } else if tx_data.len() >= 8 {
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
    
    // Deduplicate txids
    let mut unique_txids: Vec<String> = utxos.iter()
        .map(|(txid_hash, _output_index)| hex::encode(txid_hash))
        .collect();
    unique_txids.sort();
    unique_txids.dedup();
    
    let tx_count = unique_txids.len() as u32;
    let total_pages = ((tx_count as f64) / (params.page_size as f64)).ceil() as u32;
    let total_pages = if total_pages == 0 { 1 } else { total_pages };
    
    let txids = if params.details == "basic" || params.details == "tokens" || params.details == "tokenBalances" {
        None
    } else {
        Some(unique_txids)
    };
    
    Json(XPubInfo {
        page: params.page,
        total_pages,
        items_on_page: params.page_size,
        address: xpub,
        balance: balance.to_string(),
        total_received: balance.to_string(), // TODO: track separately
        total_sent: "0".to_string(), // TODO: track spent outputs
        unconfirmed_balance: "0".to_string(), // TODO: track mempool
        unconfirmed_txs: 0, // TODO: track mempool
        txs: tx_count,
        txids,
    })
}

pub async fn utxo_v2(
    AxumPath(address): AxumPath<String>,
    Query(params): Query<UtxoQuery>,
    Extension(db): Extension<Arc<DB>>
) -> Result<Json<Vec<UTXO>>, StatusCode> {
    
    // Parse the address parameter to get UTXOs from addr_index CF
    let key = format!("a{}", address);
    let key_bytes = key.as_bytes().to_vec();
    let db_clone = db.clone();
    
    // Get data from addr_index CF in blocking task
    let result = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, String> {
        let cf_addr_index = db_clone.cf_handle("addr_index")
            .ok_or_else(|| "addr_index CF not found".to_string())?;
        db_clone.get_cf(&cf_addr_index, &key_bytes)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let data = match result {
        Ok(Some(d)) => d,
        Ok(None) => return Ok(Json(vec![])), // Empty list if address not found  
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    let utxos = deserialize_utxos(&data).await;
    let mut utxo_list: Vec<UTXO> = Vec::new();
    
    // Get current height for confirmations calculation
    let chain_state = get_chain_state(&db).ok();
    let current_height = chain_state.as_ref().map(|cs| cs.height).unwrap_or(0);
    
    // Get transactions from transactions CF
    for (txid_hash, output_index) in &utxos {
        let txid_hex = hex::encode(txid_hash);
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
        .unwrap_or(Ok(None))
        .unwrap_or(None);
        
        if let Some(tx_data) = tx_data {
            // Validate transaction data size for UTXO details
            if tx_data.len() > 10_000_000 {
                eprintln!("Warning: UTXO detail transaction data too large for {}", txid_hex);
                continue;
            } else if tx_data.len() >= 8 {
                let tx_data_len = tx_data.len() - 8;
                if tx_data_len == 0 {
                    eprintln!("Warning: Empty UTXO transaction data for {}", txid_hex);
                    continue;
                }
                
                // Extract block height (bytes 4-8)
                let block_height = i32::from_le_bytes(tx_data[4..8].try_into().unwrap_or([0; 4]));
                
                // CRITICAL FIX: Skip orphaned transactions (height = -1)
                // These are either:
                // 1. Transactions from reorganized-out blocks (reorg victims)
                // 2. Ghost transactions that never existed on canonical chain
                // Both should NOT appear in UTXO set until properly re-indexed
                if block_height == -1 {
                    eprintln!("WARNING: Skipping orphaned UTXO {}:{} (height=-1, needs re-indexing)", txid_hex, output_index);
                    continue;
                }
                
                // Parse transaction for value
                let mut tx_data_with_header = Vec::with_capacity(4 + tx_data_len);
                tx_data_with_header.extend_from_slice(&[0u8; 4]);
                tx_data_with_header.extend_from_slice(&tx_data[8..]);
                
                if let Ok(tx) = deserialize_transaction(&tx_data_with_header).await {
                    if let Some(output) = tx.outputs.get(*output_index as usize) {
                        let confirmations = if block_height > 0 && current_height > 0 {
                            ((current_height - block_height) + 1).max(0) as u32
                        } else {
                            0
                        };
                        
                        // Skip unconfirmed if confirmed=true
                        if params.confirmed && confirmations == 0 {
                            continue;
                        }
                        
                        // Detect transaction type and check maturity
                        use crate::tx_type::detect_transaction_type;
                        use crate::maturity::get_maturity_status;
                        let tx_type = detect_transaction_type(&tx);
                        let (is_spendable, blocks_needed) = get_maturity_status(
                            tx_type,
                            block_height,
                            current_height,
                        );
                        
                        utxo_list.push(UTXO {
                            txid: txid_hex,
                            vout: *output_index as u32,
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
                            spendable: Some(is_spendable),
                            blocks_until_spendable: if !is_spendable && blocks_needed > 0 {
                                Some(blocks_needed)
                            } else {
                                None
                            },
                        });
                    }
                }
            }
        }
    }
    
    Ok(Json(utxo_list))
}

/*pub async fn block_v2(
    Extension(db): Extension<Arc<DB>>,
    Query(query): Query<BlockQuery>,
) -> Result<Json<rusty_piv::Block>, StatusCode> {
    let key = match (&query.block_hash, &query.block_height) {
        (Some(hash), _) => format!("b{}", hash),
        (_, Some(height)) => format!("b{}", height),
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    let result = get_block_from_db(&db, &key).await;

    match result {
        Ok(block) => Ok(block),
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}*/

pub async fn block_v2(
    Path(params): Path<BlockParams>,
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<crate::types::Block>, StatusCode> {
    let mut key = vec![b'h'];
    key.extend(&params.block_height.to_le_bytes());
    //let key_str = hex::encode(key);

    match get_block_from_db(&db, &key).await {
        Ok(block) => Ok(block),
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

/*async fn get_block_from_db(db: &Arc<DB>, key: &[u8]) -> Result<Json<rusty_piv::Block>, StatusCode> {
    //let db_clone = db.clone();
    //let cf_handle = db_clone.cf_handle("cf_blocks").ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    tokio::task::spawn_blocking(move || {
        let cf_handle = db.cf_handle("cf_blocks").ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        db.get_cf(cf_handle, key)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
            .and_then(|value_opt| {
                value_opt.ok_or_else(|| StatusCode::NOT_FOUND)
            })
            .and_then(|value| {
                serde_json::from_slice::<rusty_piv::Block>(&value)
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
                    .map(Json)
            })
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}*/
/*async fn get_block_from_db(db: Arc<DB>, key: &[u8]) -> Result<Json<rusty_piv::Block>, StatusCode> {
    tokio::task::spawn_blocking(move || {
        // Obtain cf_handle inside the closure to ensure thread safety
        let cf_handle = db.cf_handle("cf_blocks").ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        db.get_cf(cf_handle, key)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
            .and_then(|value_opt| value_opt.ok_or_else(|| StatusCode::NOT_FOUND))
            .and_then(|value| serde_json::from_slice::<rusty_piv::Block>(&value).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR))
            .map(Json)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}*/

async fn get_block_from_db(db: &Arc<DB>, height_key: &[u8]) -> Result<Json<crate::types::Block>, StatusCode> {
    let db_clone = Arc::clone(db);
    let height_key = height_key.to_vec();

    tokio::task::spawn_blocking(move || {
        // Extract height from key ('h' prefix + 4-byte i32)
        if height_key.len() < 5 {
            return Err(StatusCode::BAD_REQUEST);
        }
        let height_bytes = &height_key[1..5];
        let height = match height_bytes.try_into() {
            Ok(bytes) => i32::from_le_bytes(bytes),
            Err(_) => return Err(StatusCode::BAD_REQUEST),
        };
        
        // Get block hash from chain_metadata
        let cf_metadata = db_clone.cf_handle("chain_metadata").ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        let block_hash = db_clone.get_cf(&cf_metadata, height_bytes)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        
        // Get block header from blocks CF
        let cf_blocks = db_clone.cf_handle("blocks").ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        // Reverse the hash back to internal format for block lookup
        let internal_hash: Vec<u8> = block_hash.iter().rev().cloned().collect();
        let header_bytes = db_clone.get_cf(&cf_blocks, &internal_hash)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        
        // Parse block header
        use crate::blocks::parse_block_header_sync;
        let header = parse_block_header_sync(&header_bytes, header_bytes.len())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        
        // Get transaction IDs for this block
        let cf_transactions = db_clone.cf_handle("transactions").ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut tx_ids = Vec::new();
        
        // Query all entries starting with 'B' + height
        let mut block_tx_prefix = vec![b'B'];
        block_tx_prefix.extend_from_slice(height_bytes);
        
        let iter = db_clone.prefix_iterator_cf(&cf_transactions, &block_tx_prefix);
        for item in iter {
            match item {
                Ok((key, value)) => {
                    // Verify this is for our block height
                    if key.len() >= 5 && &key[0..5] == block_tx_prefix.as_slice() {
                        // Value is the txid as a hex string
                        if let Ok(txid_str) = String::from_utf8(value.to_vec()) {
                            tx_ids.push(txid_str);
                        }
                    } else {
                        break; // Past our prefix
                    }
                }
                Err(_) => break,
            }
        }
        
        // Calculate difficulty from nBits
        // Simplified - just use a constant approximation
        let difficulty = if header.n_bits != 0 {
            let compact = header.n_bits;
            let size = (compact >> 24) as u32;
            let word = compact & 0x00ffffff;
            
            let target = if size <= 3 {
                (word >> (8 * (3 - size))) as f64
            } else {
                (word as f64) * (256.0_f64).powi((size - 3) as i32)
            };
            
            if target > 0.0 {
                // Approximate difficulty
                (256.0_f64).powi(26) / target
            } else {
                0.0
            }
        } else {
            0.0
        };
        
        // Get previous block hash (if not genesis)
        let previousblockhash = if header.hash_prev_block != [0u8; 32] {
            Some(hex::encode(header.hash_prev_block.iter().rev().cloned().collect::<Vec<u8>>()))
        } else {
            None
        };
        
        // Build Block struct
        let block = crate::types::Block {
            hash: hex::encode(block_hash),
            height: height as u32,
            version: header.n_version,
            merkleroot: hex::encode(header.hash_merkle_root.iter().rev().cloned().collect::<Vec<u8>>()),
            time: header.n_time,
            nonce: header.n_nonce,
            bits: format!("{:08x}", header.n_bits),
            difficulty,
            tx: tx_ids,
            previousblockhash,
        };
        
        Ok(Json(block))
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}

pub async fn send_tx_v2(AxumPath(param): AxumPath<String>) -> Result<Json<SendTxResponse>, StatusCode> {
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

    let result = client.sendrawtransaction(&param, Some(false));

    match result {
        Ok(tx_hex) => {
            let response = SendTxResponse {
                result: Some(tx_hex),
                error: None, 
            };
            Ok(Json(response))
        },
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn mn_count_v2() -> Result<Json<MNCount>, StatusCode> {

    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config.get::<String>("rpc.user")
        .unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config.get::<String>("rpc.pass")
        .unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    // Extract host:port from URL
    let host_port = rpc_host
        .replace("http://", "")
        .replace("https://", "");
    
    let mut stream = match TcpStream::connect(&host_port) {
        Ok(s) => {
            eprintln!("Connected");
            s
        },
        Err(e) => {
            eprintln!("Connection failed: {}", e);
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    };
    
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();
    
    // Proper HTTP/JSON-RPC request with authentication
    let json_body = r#"{"jsonrpc":"1.0","id":"1","method":"getmasternodecount","params":[]}"#;
    let content_length = json_body.len();
    
    // Create basic auth header
    let auth_str = format!("{}:{}", rpc_user, rpc_pass);
    let auth_b64 = base64::encode(&auth_str);
    
    let request = format!(
        "POST / HTTP/1.1\r\n\
         Host: {}\r\n\
         Authorization: Basic {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        host_port, auth_b64, content_length, json_body
    );

    if let Err(e) = stream.write_all(request.as_bytes()) {
        eprintln!("Write failed: {}", e);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let mut response = Vec::new();
    if let Err(e) = stream.read_to_end(&mut response) {
        eprintln!("Read failed: {}", e);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let response_str = String::from_utf8_lossy(&response);

    // Find the JSON body after headers
    if let Some(pos) = response_str.find("\r\n\r\n") {
        let json_start = pos + 4;
        let body = &response_str[json_start..].trim();
        
        match serde_json::from_str::<serde_json::Value>(body) {
            Ok(value) => {
                if let Some(result) = value.get("result") {
                    match serde_json::from_value::<MNCount>(result.clone()) {
                        Ok(mn_count) => {
                            return Ok(Json(mn_count));
                        },
                        Err(e) => eprintln!("Parse error: {}", e),
                    }
                } else if let Some(error) = value.get("error") {
                    eprintln!("RPC error: {}", error);
                }
            },
            Err(e) => eprintln!("JSON parse error: {}", e),
        }
    } else {
        eprintln!("No JSON body found in response");
    }
    
    Err(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn mn_list_v2() -> Result<Json<Vec<MasternodeList>>, StatusCode> {
    
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config.get::<String>("rpc.user")
        .unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config.get::<String>("rpc.pass")
        .unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    let host_port = rpc_host
        .replace("http://", "")
        .replace("https://", "");
    
    let mut stream = match TcpStream::connect(&host_port) {
        Ok(s) => s,
        Err(_) => return Err(StatusCode::SERVICE_UNAVAILABLE),
    };
    
    stream.set_read_timeout(Some(Duration::from_secs(15))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(15))).ok();
    
    let json_body = r#"{"jsonrpc":"1.0","id":"1","method":"listmasternodes","params":[]}"#;
    let content_length = json_body.len();
    
    let auth_str = format!("{}:{}", rpc_user, rpc_pass);
    let auth_b64 = base64::encode(&auth_str);
    
    let request = format!(
        "POST / HTTP/1.1\r\n\
         Host: {}\r\n\
         Authorization: Basic {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        host_port, auth_b64, content_length, json_body
    );
    
    if stream.write_all(request.as_bytes()).is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let mut response = Vec::new();
    if stream.read_to_end(&mut response).is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let response_str = String::from_utf8_lossy(&response);
    
    if let Some(pos) = response_str.find("\r\n\r\n") {
        let json_start = pos + 4;
        let body = response_str[json_start..].trim();
        
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            if let Some(result) = value.get("result") {
                if let Ok(masternodes) = serde_json::from_value::<Vec<MasternodeList>>(result.clone()) {
                    return Ok(Json(masternodes));
                }
            }
        }
    }
    
    Err(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn money_supply_v2() -> Result<Json<MoneySupply>, StatusCode> {
    
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config.get::<String>("rpc.user")
        .unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config.get::<String>("rpc.pass")
        .unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    let host_port = rpc_host
        .replace("http://", "")
        .replace("https://", "");
    
    let mut stream = match TcpStream::connect(&host_port) {
        Ok(s) => s,
        Err(_) => return Err(StatusCode::SERVICE_UNAVAILABLE),
    };
    
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();
    
    let json_body = r#"{"jsonrpc":"1.0","id":"1","method":"getsupplyinfo","params":[true]}"#;
    let content_length = json_body.len();
    
    let auth_str = format!("{}:{}", rpc_user, rpc_pass);
    let auth_b64 = base64::encode(&auth_str);
    
    let request = format!(
        "POST / HTTP/1.1\r\n\
         Host: {}\r\n\
         Authorization: Basic {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        host_port, auth_b64, content_length, json_body
    );
    
    if stream.write_all(request.as_bytes()).is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let mut response = Vec::new();
    if stream.read_to_end(&mut response).is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let response_str = String::from_utf8_lossy(&response);
    
    if let Some(pos) = response_str.find("\r\n\r\n") {
        let json_start = pos + 4;
        let body = response_str[json_start..].trim();
        
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            if let Some(result) = value.get("result") {
                let total = result.get("totalsupply").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let transparent = result.get("transparentsupply").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let shield = result.get("shieldsupply").and_then(|v| v.as_f64()).unwrap_or(0.0);
                
                let supply = MoneySupply {
                    moneysupply: total,
                    transparentsupply: transparent,
                    shieldsupply: shield,
                };
                return Ok(Json(supply));
            }
        }
    }
    
    Err(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn budget_info_v2() -> Result<Json<Vec<RpcBudgetInfo>>, StatusCode> {
    
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config.get::<String>("rpc.user")
        .unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config.get::<String>("rpc.pass")
        .unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    let host_port = rpc_host
        .replace("http://", "")
        .replace("https://", "");
    
    let mut stream = match TcpStream::connect(&host_port) {
        Ok(s) => s,
        Err(_) => return Err(StatusCode::SERVICE_UNAVAILABLE),
    };
    
    stream.set_read_timeout(Some(Duration::from_secs(15))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(15))).ok();
    
    let json_body = r#"{"jsonrpc":"1.0","id":"1","method":"getbudgetinfo","params":[]}"#;
    let content_length = json_body.len();
    
    let auth_str = format!("{}:{}", rpc_user, rpc_pass);
    let auth_b64 = base64::encode(&auth_str);
    
    let request = format!(
        "POST / HTTP/1.1\r\n\
         Host: {}\r\n\
         Authorization: Basic {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        host_port, auth_b64, content_length, json_body
    );
    
    if stream.write_all(request.as_bytes()).is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let mut response = Vec::new();
    if stream.read_to_end(&mut response).is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let response_str = String::from_utf8_lossy(&response);
    
    if let Some(pos) = response_str.find("\r\n\r\n") {
        let json_start = pos + 4;
        let body = response_str[json_start..].trim();
        
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            if let Some(result) = value.get("result") {
                if let Ok(budget_info) = serde_json::from_value::<Vec<RpcBudgetInfo>>(result.clone()) {
                    return Ok(Json(budget_info));
                }
            }
        }
    }
    
    Err(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn budget_votes_v2(AxumPath(proposal_name): AxumPath<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config.get::<String>("rpc.user")
        .unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config.get::<String>("rpc.pass")
        .unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    let host_port = rpc_host
        .replace("http://", "")
        .replace("https://", "");
    
    let mut stream = match TcpStream::connect(&host_port) {
        Ok(s) => s,
        Err(_) => return Err(StatusCode::SERVICE_UNAVAILABLE),
    };
    
    stream.set_read_timeout(Some(Duration::from_secs(15))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(15))).ok();
    
    let json_body = format!(r#"{{"jsonrpc":"1.0","id":"1","method":"getbudgetvotes","params":["{}"]}}"#, proposal_name);
    let content_length = json_body.len();
    
    let auth_str = format!("{}:{}", rpc_user, rpc_pass);
    let auth_b64 = base64::encode(&auth_str);
    
    let request = format!(
        "POST / HTTP/1.1\r\n\
         Host: {}\r\n\
         Authorization: Basic {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        host_port, auth_b64, content_length, json_body
    );
    
    if stream.write_all(request.as_bytes()).is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let mut response = Vec::new();
    if stream.read_to_end(&mut response).is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let response_str = String::from_utf8_lossy(&response);
    
    if let Some(pos) = response_str.find("\r\n\r\n") {
        let json_start = pos + 4;
        let body = response_str[json_start..].trim();
        
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            if let Some(result) = value.get("result") {
                return Ok(Json(result.clone()));
            }
        }
    }
    
    Err(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn budget_projection_v2() -> Result<Json<serde_json::Value>, StatusCode> {
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config.get::<String>("rpc.user")
        .unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config.get::<String>("rpc.pass")
        .unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    let host_port = rpc_host
        .replace("http://", "")
        .replace("https://", "");
    
    let mut stream = match TcpStream::connect(&host_port) {
        Ok(s) => s,
        Err(_) => return Err(StatusCode::SERVICE_UNAVAILABLE),
    };
    
    stream.set_read_timeout(Some(Duration::from_secs(15))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(15))).ok();
    
    let json_body = r#"{"jsonrpc":"1.0","id":"1","method":"getbudgetprojection","params":[]}"#;
    let content_length = json_body.len();
    
    let auth_str = format!("{}:{}", rpc_user, rpc_pass);
    let auth_b64 = base64::encode(&auth_str);
    
    let request = format!(
        "POST / HTTP/1.1\r\n\
         Host: {}\r\n\
         Authorization: Basic {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        host_port, auth_b64, content_length, json_body
    );
    
    if stream.write_all(request.as_bytes()).is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let mut response = Vec::new();
    if stream.read_to_end(&mut response).is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let response_str = String::from_utf8_lossy(&response);
    
    if let Some(pos) = response_str.find("\r\n\r\n") {
        let json_start = pos + 4;
        let body = response_str[json_start..].trim();
        
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
            if let Some(result) = value.get("result") {
                return Ok(Json(result.clone()));
            }
        }
    }
    
    Err(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn relay_mnb_v2(AxumPath(param): AxumPath<String>) -> Result<Json<String>, StatusCode> {
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host");
    let rpc_user = config.get::<String>("rpc.user");
    let rpc_pass = config.get::<String>("rpc.pass");

    let client = BitcoinRpcClient::new(
        rpc_host.unwrap_or_else(|_| "127.0.0.1:51472".to_string()),
        Some(rpc_user.unwrap_or_default()),
        Some(rpc_pass.unwrap_or_default()),
        3,    // Max retries
        10,   // Connection timeout
        1000, // Read/write timeout
    );

    let result = client.relaymasternodebroadcast(&param);

    match result {
        Ok(mnb_relay) => {
            Ok(Json(mnb_relay))
        },
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

// ============================================================================
// New V2 API Endpoints
// ============================================================================

/// GET /api/v2/status
/// Returns current sync status and chain state
pub async fn status_v2(
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<ChainState>, StatusCode> {
    match get_chain_state(&db) {
        Ok(state) => Ok(Json(state)),
        Err(_e) => Err(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

/// GET /api/v2/search/{query}
/// Universal search for blocks, transactions, or addresses
pub async fn search_v2(
    Path(query): Path<String>,
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<SearchResult>, StatusCode> {
    match search(&db, &query) {
        Ok(result) => Ok(Json(result)),
        Err(_e) => Err(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

/// GET /api/v2/mempool
/// Returns current mempool information
pub async fn mempool_v2(
    Extension(mempool_state): Extension<Arc<MempoolState>>,
) -> Result<Json<MempoolInfo>, StatusCode> {
    let info = mempool_state.get_info().await;
    Ok(Json(info))
}

/// GET /api/v2/mempool/{txid}
/// Returns specific mempool transaction
pub async fn mempool_tx_v2(
    Path(txid): Path<String>,
    Extension(mempool_state): Extension<Arc<MempoolState>>,
) -> Result<Json<crate::mempool::MempoolTransaction>, StatusCode> {
    match mempool_state.get_transaction(&txid).await {
        Some(tx) => Ok(Json(tx)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BlockStats {
    pub height: u32,
    pub hash: String,
    pub time: u64,
    pub tx_count: usize,
    pub size: usize,
    pub difficulty: f64,
}

/// GET /api/v2/block-stats/{count}
/// Returns statistics for the last N blocks
pub async fn block_stats_v2(
    Path(count): Path<u32>,
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<Vec<BlockStats>>, StatusCode> {
    let db_clone = Arc::clone(&db);
    let count_clone = count;
    
    let result = tokio::task::spawn_blocking(move || {
        // Get current block height from chain state
        let chain_state = match get_chain_state(&db_clone) {
            Ok(state) => state,
            Err(_) => return Err("Failed to get chain state"),
        };
        let tip_height = chain_state.height as u32;
        
        let mut stats = Vec::new();
        let start_height = tip_height.saturating_sub(count_clone);
        
        for height in (start_height..=tip_height).rev() {
            let key = height.to_le_bytes().to_vec();
            
            // Get block hash from blocks CF
            let cf_blocks = match db_clone.cf_handle("blocks") {
                Some(cf) => cf,
                None => continue,
            };
            
            let block_hash = match db_clone.get_cf(&cf_blocks, &key) {
                Ok(Some(hash)) => hex::encode(&hash),
                _ => continue,
            };
            
            // Get block details
            let block_key = hex::decode(&block_hash).unwrap_or_default();
            let cf_block_data = match db_clone.cf_handle("block_data") {
                Some(cf) => cf,
                None => continue,
            };
            
            if let Ok(Some(block_data)) = db_clone.get_cf(&cf_block_data, &block_key) {
                // Parse block data to get stats
                // Block data format: height (4) + time (4) + nonce (4) + version (4) + 
                // prev_hash (32) + merkle_root (32) + bits (4) + tx_count (4) + size (4) + difficulty (8)
                if block_data.len() >= 100 {
                    let time = u32::from_le_bytes(block_data[4..8].try_into().unwrap_or([0; 4])) as u64;
                    let tx_count = u32::from_le_bytes(block_data[92..96].try_into().unwrap_or([0; 4])) as usize;
                    let size = u32::from_le_bytes(block_data[96..100].try_into().unwrap_or([0; 4])) as usize;
                    let difficulty = if block_data.len() >= 108 {
                        f64::from_le_bytes(block_data[100..108].try_into().unwrap_or([0; 8]))
                    } else {
                        0.0
                    };
                    
                    stats.push(BlockStats {
                        height,
                        hash: block_hash,
                        time,
                        tx_count,
                        size,
                        difficulty,
                    });
                }
            }
        }
        
        Ok(stats)
    }).await;
    
    match result {
        Ok(Ok(stats)) => Ok(Json(stats)),
        _ => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Health check endpoint - shows database and sync status
#[derive(Serialize)]
pub struct HealthStatus {
    pub status: String,
    pub database_ok: bool,
    pub address_index_complete: bool,
    pub total_transactions: u64,
    pub valid_transactions: u64,
    pub orphaned_transactions: u64,
    pub indexed_addresses: u64,
    pub warnings: Vec<String>,
}

pub async fn health_check_v2(
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<HealthStatus>, StatusCode> {
    let result = tokio::task::spawn_blocking(move || -> Result<HealthStatus, Box<dyn std::error::Error + Send + Sync>> {
        let mut warnings = Vec::new();
        
        // Check transaction counts
        let tx_cf = db.cf_handle("transactions").ok_or("transactions CF not found")?;
        let mut total_txs = 0u64;
        let mut orphaned_txs = 0u64;
        let mut valid_txs = 0u64;
        
        let iter = db.iterator_cf(tx_cf, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item?;
            if key.first() == Some(&b'B') {
                continue;
            }
            
            total_txs += 1;
            
            if value.len() >= 8 {
                let height_bytes: [u8; 4] = value[4..8].try_into().unwrap_or([0,0,0,0]);
                let height = i32::from_le_bytes(height_bytes);
                if height == -1 {
                    orphaned_txs += 1;
                } else {
                    valid_txs += 1;
                }
            }
        }
        
        // Check address index
        let addr_cf = db.cf_handle("addr_index").ok_or("addr_index CF not found")?;
        let mut indexed_addresses = 0u64;
        let iter = db.iterator_cf(addr_cf, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, _) = item?;
            if key.first() == Some(&b't') {
                indexed_addresses += 1;
            }
        }
        
        // Check address index completeness marker
        let state_cf = db.cf_handle("chain_state").ok_or("chain_state CF not found")?;
        let addr_complete = db.get_cf(state_cf, b"address_index_complete")?;
        let address_index_complete = addr_complete.is_some();
        
        // Generate warnings
        if !address_index_complete && valid_txs > 0 {
            warnings.push("Address index not marked as complete. Run rebuild_address_index if sync is done.".to_string());
        }
        
        if indexed_addresses == 0 && valid_txs > 100000 {
            warnings.push("Address index is empty but many transactions exist. Run rebuild_address_index.".to_string());
        }
        
        if orphaned_txs > valid_txs / 100 {
            warnings.push(format!("High orphaned transaction count: {} ({:.1}%)", 
                                 orphaned_txs, 
                                 (orphaned_txs as f64 / valid_txs as f64) * 100.0));
        }
        
        let status = if warnings.is_empty() { "healthy" } else { "degraded" };
        
        Ok(HealthStatus {
            status: status.to_string(),
            database_ok: true,
            address_index_complete,
            total_transactions: total_txs,
            valid_transactions: valid_txs,
            orphaned_transactions: orphaned_txs,
            indexed_addresses,
            warnings,
        })
    }).await;
    
    match result {
        Ok(Ok(status)) => Ok(Json(status)),
        _ => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
