/// Block Detail API
/// Comprehensive block information with transactions
use std::sync::Arc;
use rocksdb::DB;
use serde::{Serialize, Deserialize};
use axum::{
    extract::{Path, Extension},
    http::StatusCode,
    Json,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDetail {
    pub height: i32,
    pub hash: String,
    pub confirmations: i32,
    pub size: usize,
    pub version: u32,
    pub merkleroot: String,
    pub time: u32,
    pub nonce: u32,
    pub bits: String,
    pub difficulty: f64,
    pub previousblockhash: Option<String>,
    pub nextblockhash: Option<String>,
    pub tx: Vec<TransactionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionSummary {
    pub txid: String,
    pub version: i16,
    pub size: usize,
    pub locktime: u32,
    pub vin: Vec<TxInput>,
    pub vout: Vec<TxOutput>,
    pub value_in: f64,
    pub value_out: f64,
    pub fees: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxInput {
    pub txid: Option<String>,
    pub vout: Option<u32>,
    pub address: Option<String>,
    pub value: Option<f64>,
    pub coinbase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxOutput {
    pub n: u64,
    pub value: f64,
    pub addresses: Vec<String>,
    pub spent: bool,
}

/// API Handler for /api/v2/block-detail/{height}
pub async fn block_detail_v2(
    Path(height): Path<i32>,
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<BlockDetail>, StatusCode> {
    let db_clone = Arc::clone(&db);
    
    tokio::task::spawn_blocking(move || {
        get_block_detail(&db_clone, height)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}

fn get_block_detail(db: &Arc<DB>, height: i32) -> Result<Json<BlockDetail>, StatusCode> {
    // Get chain metadata CF
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    
    // Get current network height for confirmations
    let cf_chain_state = db.cf_handle("chain_state")
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let network_height = db.get_cf(&cf_chain_state, b"network_height")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .and_then(|bytes| {
            if bytes.len() >= 4 {
                Some(i32::from_le_bytes(bytes[0..4].try_into().ok()?))
            } else {
                None
            }
        })
        .unwrap_or(height);
    
    let confirmations = if network_height >= height {
        network_height - height + 1
    } else {
        0
    };
    
    // Get block hash from height
    let height_key = height.to_le_bytes();
    let block_hash_bytes = db.get_cf(&cf_metadata, &height_key)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    
    // Hash is stored in display format (reversed), so use directly
    let hash = hex::encode(&block_hash_bytes);
    
    // Get block header from blocks CF
    let cf_blocks = db.cf_handle("blocks")
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    
    // Reverse back for internal lookup
    let internal_hash: Vec<u8> = block_hash_bytes.iter().rev().cloned().collect();
    let block_header_bytes = db.get_cf(&cf_blocks, &internal_hash)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    
    // Parse block header
    let header = parse_block_header(&block_header_bytes)?;
    
    // Get transactions for this block
    let transactions = get_block_transactions(db, height)?;
    
    // Get previous block hash
    let previousblockhash = if height > 0 {
        let prev_height = (height - 1).to_le_bytes();
        db.get_cf(&cf_metadata, &prev_height)
            .ok()
            .flatten()
            .map(|bytes| hex::encode(&bytes))
    } else {
        None
    };
    
    // Get next block hash
    let nextblockhash = {
        let next_height = (height + 1).to_le_bytes();
        db.get_cf(&cf_metadata, &next_height)
            .ok()
            .flatten()
            .map(|bytes| hex::encode(&bytes))
    };
    
    let block_detail = BlockDetail {
        height,
        hash,
        confirmations,
        size: block_header_bytes.len() + transactions.iter().map(|tx| tx.size).sum::<usize>(),
        version: header.version,
        merkleroot: header.merkleroot,
        time: header.time,
        nonce: header.nonce,
        bits: header.bits,
        difficulty: header.difficulty,
        previousblockhash,
        nextblockhash,
        tx: transactions,
    };
    
    Ok(Json(block_detail))
}

#[derive(Debug)]
struct BlockHeader {
    version: u32,
    merkleroot: String,
    time: u32,
    nonce: u32,
    bits: String,
    difficulty: f64,
}

fn parse_block_header(data: &[u8]) -> Result<BlockHeader, StatusCode> {
    if data.len() < 80 {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    // Parse version (4 bytes)
    let version = u32::from_le_bytes(data[0..4].try_into().unwrap());
    
    // Skip prev block hash (32 bytes)
    
    // Parse merkle root (32 bytes at offset 36)
    let merkle_bytes = &data[36..68];
    let merkleroot = hex::encode(merkle_bytes.iter().rev().cloned().collect::<Vec<u8>>());
    
    // Parse time (4 bytes at offset 68)
    let time = u32::from_le_bytes(data[68..72].try_into().unwrap());
    
    // Parse bits (4 bytes at offset 72)
    let bits_value = u32::from_le_bytes(data[72..76].try_into().unwrap());
    let bits = format!("{:08x}", bits_value);
    
    // Parse nonce (4 bytes at offset 76)
    let nonce = u32::from_le_bytes(data[76..80].try_into().unwrap());
    
    // Calculate difficulty from bits
    let difficulty = bits_to_difficulty(bits_value);
    
    Ok(BlockHeader {
        version,
        merkleroot,
        time,
        nonce,
        bits,
        difficulty,
    })
}

fn bits_to_difficulty(bits: u32) -> f64 {
    let exponent = (bits >> 24) as i32;
    let mantissa = (bits & 0x00ffffff) as f64;
    
    if exponent <= 3 {
        mantissa / 256_f64.powi(3 - exponent)
    } else {
        mantissa * 256_f64.powi(exponent - 3)
    }
}

fn get_block_transactions(db: &Arc<DB>, height: i32) -> Result<Vec<TransactionSummary>, StatusCode> {
    let cf_transactions = db.cf_handle("transactions")
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let mut transactions = Vec::new();
    
    // Create prefix for this block's transaction index: 'B' + height
    let mut prefix = vec![b'B'];
    prefix.extend(&height.to_le_bytes());
    
    eprintln!("Looking for transactions in block {} with prefix {:?}", height, prefix);
    
    // Iterate through block transaction index entries
    let iter = db.prefix_iterator_cf(&cf_transactions, &prefix);
    
    let mut count = 0;
    for item in iter {
        match item {
            Ok((key, value)) => {
                // Check if key still has our prefix
                if !key.starts_with(&prefix) {
                    eprintln!("  Prefix mismatch, stopping iteration");
                    break;
                }
                
                count += 1;
                
                // Value is the txid in hex format
                if let Ok(txid_str) = std::str::from_utf8(&value) {
                    eprintln!("  Found tx index entry: {}", txid_str);
                    // Look up the full transaction data
                    if let Ok(txid_bytes) = hex::decode(txid_str) {
                        let mut tx_key = vec![b't'];
                        tx_key.extend_from_slice(&txid_bytes);
                        
                        if let Ok(Some(tx_data)) = db.get_cf(&cf_transactions, &tx_key) {
                            eprintln!("  Found tx data, size: {} bytes", tx_data.len());
                            if let Ok(mut tx) = parse_transaction(&tx_data) {
                                // Enrich inputs with values and addresses
                                enrich_transaction_inputs(db, &cf_transactions, &mut tx);
                                transactions.push(tx);
                                eprintln!("  Parsed tx successfully");
                            } else {
                                eprintln!("  Failed to parse tx");
                            }
                        } else {
                            eprintln!("  No tx data found for txid");
                        }
                    }
                } else {
                    eprintln!("  Invalid UTF-8 in txid value");
                }
            }
            Err(e) => {
                eprintln!("  Iterator error: {:?}", e);
                break;
            }
        }
    }
    
    eprintln!("Block {} has {} transaction entries, parsed {} transactions", height, count, transactions.len());
    
    Ok(transactions)
}

fn enrich_transaction_inputs(db: &Arc<DB>, cf_transactions: &rocksdb::ColumnFamily, tx: &mut TransactionSummary) {
    for input in &mut tx.vin {
        // Skip coinbase inputs
        if input.coinbase.is_some() {
            continue;
        }
        
        // Get the previous transaction if we have txid
        if let Some(ref prev_txid) = input.txid {
            if let Ok(prev_txid_bytes) = hex::decode(prev_txid) {
                let mut prev_key = vec![b't'];
                prev_key.extend_from_slice(&prev_txid_bytes);
                
                if let Ok(Some(prev_data)) = db.get_cf(cf_transactions, &prev_key) {
                    if prev_data.len() >= 8 {
                        let prev_tx_json = &prev_data[8..];
                        if let Ok(prev_tx) = serde_json::from_slice::<serde_json::Value>(prev_tx_json) {
                            // Extract the output at vout index
                            if let Some(vout_idx) = input.vout {
                                if let Some(vout_array) = prev_tx.get("vout").and_then(|v| v.as_array()) {
                                    if let Some(output) = vout_array.get(vout_idx as usize) {
                                        // Add value from the output
                                        if let Some(value) = output.get("value").and_then(|v| v.as_f64()) {
                                            input.value = Some(value);
                                        }
                                        
                                        // Add address from the output
                                        if let Some(script_pubkey) = output.get("scriptPubKey") {
                                            if let Some(addresses) = script_pubkey.get("addresses").and_then(|a| a.as_array()) {
                                                if let Some(first_addr) = addresses.get(0).and_then(|a| a.as_str()) {
                                                    input.address = Some(first_addr.to_string());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Recalculate value_in and fees after enrichment
    tx.value_in = tx.vin.iter()
        .filter_map(|i| i.value)
        .sum::<f64>();
    
    // Recalculate fees
    if tx.value_in > 0.0 {
        let calculated_fee = tx.value_in - tx.value_out;
        tx.fees = if calculated_fee < 0.0 {
            // Coinstake transaction (outputs include reward)
            0.0
        } else {
            calculated_fee
        };
    } else {
        // Coinbase transaction
        tx.fees = 0.0;
    }
}

fn parse_transaction(data: &[u8]) -> Result<TransactionSummary, Box<dyn std::error::Error>> {
    // Data format: block_version (4 bytes) + block_height (4 bytes) + transaction_data
    if data.len() < 8 {
        return Err("Transaction data too short".into());
    }
    
    // Skip the block_version and block_height
    let tx_data = &data[8..];
    
    // Try to parse as JSON first (for RPC-indexed blocks)
    if let Ok(json_tx) = serde_json::from_slice::<serde_json::Value>(tx_data) {
        return parse_transaction_from_json(&json_tx);
    }
    
    // Otherwise parse binary format
    parse_transaction_binary(tx_data)
}

fn parse_transaction_from_json(json: &serde_json::Value) -> Result<TransactionSummary, Box<dyn std::error::Error>> {
    let txid = json["txid"].as_str().unwrap_or("").to_string();
    let version = json["version"].as_i64().unwrap_or(1) as i16;
    let locktime = json["locktime"].as_u64().unwrap_or(0) as u32;
    let size = json["size"].as_u64().unwrap_or(0) as usize;
    
    let mut vin = Vec::new();
    if let Some(inputs) = json["vin"].as_array() {
        for input in inputs {
            if let Some(coinbase) = input["coinbase"].as_str() {
                vin.push(TxInput {
                    txid: None,
                    vout: None,
                    address: None,
                    value: None,
                    coinbase: Some(coinbase.to_string()),
                });
            } else {
                vin.push(TxInput {
                    txid: input["txid"].as_str().map(|s| s.to_string()),
                    vout: input["vout"].as_u64().map(|v| v as u32),
                    address: input["address"].as_str().map(|s| s.to_string()),
                    value: input["value"].as_f64(),
                    coinbase: None,
                });
            }
        }
    }
    
    let mut vout = Vec::new();
    let mut value_out = 0.0;
    if let Some(outputs) = json["vout"].as_array() {
        for (n, output) in outputs.iter().enumerate() {
            let value = output["value"].as_f64().unwrap_or(0.0);
            value_out += value;
            
            let mut addresses = Vec::new();
            if let Some(addrs) = output["scriptPubKey"]["addresses"].as_array() {
                addresses = addrs.iter()
                    .filter_map(|a| a.as_str().map(|s| s.to_string()))
                    .collect();
            }
            
            vout.push(TxOutput {
                n: n as u64,
                value,
                addresses,
                spent: false,
            });
        }
    }
    
    let value_in = vin.iter()
        .filter_map(|i| i.value)
        .sum::<f64>();
    
    // Calculate fees
    // For coinstake transactions (value_out > value_in), the "fee" would be negative
    // because the output includes staking rewards. We should report 0 fee for these.
    // For regular transactions and coinbase, calculate normally.
    let fees = if value_in > 0.0 {
        let calculated_fee = value_in - value_out;
        if calculated_fee < 0.0 {
            // This is a coinstake (outputs include reward)
            0.0
        } else {
            calculated_fee
        }
    } else {
        // Coinbase transaction (no inputs)
        0.0
    };
    
    Ok(TransactionSummary {
        txid,
        version,
        size,
        locktime,
        vin,
        vout,
        value_in,
        value_out,
        fees,
    })
}

fn parse_transaction_binary(_data: &[u8]) -> Result<TransactionSummary, Box<dyn std::error::Error>> {
    // TODO: Implement binary transaction parsing
    // For now, return a minimal transaction
    Ok(TransactionSummary {
        txid: "unknown".to_string(),
        version: 1,
        size: _data.len(),
        locktime: 0,
        vin: vec![],
        vout: vec![],
        value_in: 0.0,
        value_out: 0.0,
        fees: 0.0,
    })
}
