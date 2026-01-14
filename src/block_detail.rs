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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reward: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sapling: Option<SaplingInfo>,
}

/// Detailed Sapling transaction information exposed via API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaplingInfo {
    /// Net value moved between shielded and transparent pools (PIV)
    /// Positive: unshielding (shield → transparent)
    /// Negative: shielding (transparent → shield)
    pub value_balance: f64,
    
    /// Number of shielded spends (inputs from shielded pool)
    pub shielded_spend_count: u64,
    
    /// Number of shielded outputs (new notes in shielded pool)
    pub shielded_output_count: u64,
    
    /// Binding signature proving balance (hex)
    pub binding_sig: String,
    
    /// Detailed spend descriptions (optional, for full tx details)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spends: Option<Vec<SpendInfo>>,
    
    /// Detailed output descriptions (optional, for full tx details)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<OutputInfo>>,
}

/// Information about a single shielded spend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendInfo {
    /// Value commitment (hex) - commitment to the value being spent
    pub cv: String,
    
    /// Merkle tree anchor (hex) - root at some past block height
    pub anchor: String,
    
    /// Nullifier (hex) - prevents double-spending
    pub nullifier: String,
    
    /// Randomized public key (hex) - for signature verification
    pub rk: String,
    
    /// Zero-knowledge proof (hex) - Groth16 proof (192 bytes)
    pub zkproof: String,
    
    /// Spend authorization signature (hex) - authorizes this spend
    pub spend_auth_sig: String,
}

/// Information about a single shielded output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputInfo {
    /// Value commitment (hex) - commitment to output value
    pub cv: String,
    
    /// Note commitment u-coordinate (hex) - commitment to the new note
    pub cmu: String,
    
    /// Ephemeral public key (hex) - for note encryption
    pub ephemeral_key: String,
    
    /// Encrypted note for recipient (hex) - 580 bytes
    pub enc_ciphertext: String,
    
    /// Encrypted note for sender (hex) - 80 bytes  
    pub out_ciphertext: String,
    
    /// Zero-knowledge proof (hex) - Groth16 proof (192 bytes)
    pub zkproof: String,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxInput {
    pub txid: Option<String>,
    pub vout: Option<u32>,
    pub address: Option<String>,
    pub addresses: Option<Vec<String>>,
    pub value: Option<f64>,
    pub coinbase: Option<String>,
    #[serde(rename = "type")]
    pub script_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxOutput {
    pub n: u64,
    pub value: f64,
    pub addresses: Vec<String>,
    pub spent: bool,
    #[serde(rename = "type")]
    pub script_type: Option<String>,
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
    let block_hash_bytes = db.get_cf(&cf_metadata, height_key)
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
        db.get_cf(&cf_metadata, prev_height)
            .ok()
            .flatten()
            .map(|bytes| hex::encode(&bytes))
    } else {
        None
    };
    
    // Get next block hash
    let nextblockhash = {
        let next_height = (height + 1).to_le_bytes();
        db.get_cf(&cf_metadata, next_height)
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
    let version = u32::from_le_bytes(
        data[0..4].try_into()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    );
    
    // Skip prev block hash (32 bytes)
    
    // Parse merkle root (32 bytes at offset 36)
    let merkle_bytes = &data[36..68];
    let merkleroot = hex::encode(merkle_bytes.iter().rev().cloned().collect::<Vec<u8>>());
    
    // Parse time (4 bytes at offset 68)
    let time = u32::from_le_bytes(
        data[68..72].try_into()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    );
    
    // Parse bits (4 bytes at offset 72)
    let bits_value = u32::from_le_bytes(
        data[72..76].try_into()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    );
    let bits = format!("{:08x}", bits_value);
    
    // Parse nonce (4 bytes at offset 76)
    let nonce = u32::from_le_bytes(
        data[76..80].try_into()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    );
    
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
    
    // Iterate through block transaction index entries
    let iter = db.prefix_iterator_cf(&cf_transactions, &prefix);
    
    for item in iter {
        match item {
            Ok((key, value)) => {
                // Check if key still has our prefix
                if !key.starts_with(&prefix) {
                    break;
                }
                
                // Value is the txid in hex format (display format)
                if let Ok(txid_str) = std::str::from_utf8(&value) {
                    // Look up the full transaction data
                    if let Ok(txid_bytes) = hex::decode(txid_str) {
                        // Try reversed format first (new/correct format)
                        let reversed: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
                        
                        let mut tx_key = vec![b't'];
                        tx_key.extend_from_slice(&reversed);
                        
                        let tx_data = if let Ok(Some(data)) = db.get_cf(&cf_transactions, &tx_key) {
                            Some(data)
                        } else {
                            // Fallback: try display format (old/incorrect format for migration)
                            let mut tx_key_display = vec![b't'];
                            tx_key_display.extend_from_slice(&txid_bytes);
                            db.get_cf(&cf_transactions, &tx_key_display).ok().flatten()
                        };
                        
                        if let Some(tx_data) = tx_data {
                            if let Ok(mut tx) = parse_transaction(&tx_data) {
                                // Enrich inputs with values and addresses
                                enrich_transaction_inputs(db, cf_transactions, &mut tx);
                                transactions.push(tx);
                            } else {
                                eprintln!("Failed to parse transaction: {}", txid_str);
                            }
                        } else {
                            eprintln!("Transaction data not found for txid: {}", txid_str);
                        }
                    }
                }
            }
            Err(_) => {
                break;
            }
        }
    }
    
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
                // Try reversed format first (new/correct format)
                let reversed: Vec<u8> = prev_txid_bytes.iter().rev().cloned().collect();
                let mut prev_key = vec![b't'];
                prev_key.extend_from_slice(&reversed);
                
                let prev_data = if let Ok(Some(data)) = db.get_cf(cf_transactions, &prev_key) {
                    Some(data)
                } else {
                    // Fallback: try display format (old/incorrect format for migration)
                    let mut prev_key_display = vec![b't'];
                    prev_key_display.extend_from_slice(&prev_txid_bytes);
                    db.get_cf(cf_transactions, &prev_key_display).ok().flatten()
                };
                
                if let Some(prev_data) = prev_data {
                    if prev_data.len() >= 8 {
                        let prev_tx_data = &prev_data[8..]; // Skip block_version + height
                        
                        // Prepend dummy header for parser
                        let mut data_with_header = vec![0u8; 4];
                        data_with_header.extend_from_slice(prev_tx_data);
                        
                        // Parse the previous transaction
                                                if let Ok(prev_tx) = crate::parser::deserialize_transaction_blocking(&data_with_header) {
                            // Extract the output at vout index
                            if let Some(vout_idx) = input.vout {
                                if let Some(output) = prev_tx.outputs.get(vout_idx as usize) {
                                    // Add value from the output (in satoshis)
                                    input.value = Some(output.value as f64);
                                    
                                    // Add address(es) from the output
                                    if !output.address.is_empty() {
                                        input.address = output.address.first().cloned();
                                        input.addresses = Some(output.address.clone());
                                    }
                                    
                                    // Add script type from the output
                                    use crate::parser::get_script_type;
                                    let script_type = get_script_type(&output.script_pubkey.script);
                                    input.script_type = Some(script_type.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Recalculate value_in and fees after enrichment (convert satoshis to PIV)
    tx.value_in = tx.vin.iter()
        .filter_map(|i| i.value)
        .sum::<f64>() / 100_000_000.0;
    
    // Calculate fees and rewards based on transaction type
    if let Some(ref tx_type) = tx.tx_type {
        if tx_type == "coinstake" {
            // Coinstake: reward is output - input (stake reward)
            tx.reward = Some(tx.value_out - tx.value_in);
            tx.fees = 0.0;
        } else if tx_type == "coinbase" {
            // Coinbase: reward already set to value_out in parse_transaction_binary
            tx.fees = 0.0;
        }
    } else {
        // Normal transaction: calculate fees
        if tx.value_in > 0.0 {
            let calculated_fee = tx.value_in - tx.value_out;
            tx.fees = if calculated_fee < 0.0 { 0.0 } else { calculated_fee };
        } else {
            tx.fees = 0.0;
        }
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
                    addresses: None,
                    value: None,
                    coinbase: Some(coinbase.to_string()),
                    script_type: None,
                });
            } else {
                vin.push(TxInput {
                    txid: input["txid"].as_str().map(|s| s.to_string()),
                    vout: input["vout"].as_u64().map(|v| v as u32),
                    address: input["address"].as_str().map(|s| s.to_string()),
                    addresses: None, // Will be enriched if prev tx is cold staking
                    value: input["value"].as_f64(),
                    coinbase: None,
                    script_type: None, // Will be enriched from prev tx
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
            
            // Get script type if available
            let script_type = output["scriptPubKey"]["type"]
                .as_str()
                .map(|s| s.to_string());
            
            vout.push(TxOutput {
                n: n as u64,
                value,
                addresses,
                spent: false,
                script_type,
            });
        }
    }
    
    let value_in = vin.iter()
        .filter_map(|i| i.value)
        .sum::<f64>();
    
    // Calculate fees and identify transaction type (fallback parsing)
    // For coinstake transactions (value_out > value_in), the "fee" would be negative
    // because the output includes staking rewards. We should report 0 fee for these.
    // For regular transactions and coinbase, calculate normally.
    let has_coinbase_input = vin.iter().any(|i| i.coinbase.is_some());
    let first_output_empty = vout.first().map_or(false, |o| o.value == 0.0 && o.addresses.is_empty());
    
    let (tx_type, reward, fees) = if has_coinbase_input {
        if first_output_empty && vout.len() > 1 {
            // Coinstake: reward is output - input
            let coinstake_reward = value_out - value_in;
            (Some("coinstake".to_string()), Some(coinstake_reward), 0.0)
        } else {
            // Coinbase: reward is total output
            (Some("coinbase".to_string()), Some(value_out), 0.0)
        }
    } else {
        // Normal transaction
        let calculated_fee = if value_in > 0.0 {
            let fee = value_in - value_out;
            if fee < 0.0 { 0.0 } else { fee }
        } else {
            0.0
        };
        (None, None, calculated_fee)
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
        tx_type,
        reward,
        sapling: None,  // JSON-based parsing doesn't include Sapling details
    })
}

fn parse_transaction_binary(data: &[u8]) -> Result<TransactionSummary, Box<dyn std::error::Error>> {
    use crate::parser::deserialize_transaction;
    
    // Parse using the binary parser
    // Need to prepend 4-byte dummy block_version header since parser expects it
    let mut data_with_header = vec![0u8; 4]; // Dummy block_version
    data_with_header.extend_from_slice(data);
    
    // Use blocking runtime since this is called from a blocking context
    let tx = tokio::runtime::Handle::current()
        .block_on(async {
            deserialize_transaction(&data_with_header).await
        })?;
    
    // Convert inputs
    let mut vin = Vec::new();
    for input in &tx.inputs {
        if let Some(coinbase_data) = &input.coinbase {
            vin.push(TxInput {
                txid: None,
                vout: None,
                address: None,
                addresses: None,
                value: None,
                coinbase: Some(hex::encode(coinbase_data)),
                script_type: None,
            });
        } else if let Some(prevout) = &input.prevout {
            vin.push(TxInput {
                txid: Some(prevout.hash.clone()),
                vout: Some(prevout.n),
                address: None, // Will be enriched later
                addresses: None, // Will be enriched later
                value: None,   // Will be enriched later
                coinbase: None,
                script_type: None, // Will be enriched later
            });
        }
    }
    
    // Convert outputs
    let mut vout = Vec::new();
    let mut value_out = 0.0;
    
    use crate::parser::get_script_type;
    
    for (idx, output) in tx.outputs.iter().enumerate() {
        let value_satoshis = output.value as f64;
        value_out += value_satoshis / 100_000_000.0;  // Still calculate PIV totals for backward compatibility
        
        let script_type = get_script_type(&output.script_pubkey.script);
        
        vout.push(TxOutput {
            n: idx as u64,
            value: value_satoshis,  // Return raw satoshis, not PIV
            addresses: output.address.clone(),
            spent: false,
            script_type: Some(script_type.to_string()),
        });
    }
    
    // Identify transaction type based on PIVX rules:
    // - Coinbase: first input has coinbase data, first output has value (standard mining reward)
    // - Coinstake: first output is empty (0 value, 0-length script), has regular inputs
    // - Normal: regular transaction
    let has_coinbase_input = vin.first().map_or(false, |i| i.coinbase.is_some());
    let first_output_empty = vout.first().map_or(false, |o| 
        o.value == 0.0 && o.addresses.is_empty()
    );
    
    let (tx_type, reward) = if has_coinbase_input {
        // Has coinbase input
        if first_output_empty && vout.len() > 1 {
            // Should not happen in practice (coinbase with empty output)
            (Some("coinbase".to_string()), Some(value_out))
        } else {
            // Standard coinbase: reward is total output value
            (Some("coinbase".to_string()), Some(value_out))
        }
    } else if first_output_empty && vout.len() > 1 {
        // Coinstake: no coinbase input, but empty first output
        // Reward will be calculated after enrichment (output - input)
        (Some("coinstake".to_string()), None)
    } else {
        // Normal transaction
        (None, None)
    };
    
    // Convert Sapling data if present
    let sapling = tx.sapling_data.as_ref().map(|sap| {
        // Convert spends to API format
        let spends = sap.vshielded_spend.iter().map(|spend| SpendInfo {
            cv: hex::encode(spend.cv),
            anchor: hex::encode(spend.anchor),
            nullifier: hex::encode(spend.nullifier),
            rk: hex::encode(spend.rk),
            zkproof: hex::encode(spend.zkproof),
            spend_auth_sig: hex::encode(spend.spend_auth_sig),
        }).collect();
        
        // Convert outputs to API format
        let outputs = sap.vshielded_output.iter().map(|output| OutputInfo {
            cv: hex::encode(output.cv),
            cmu: hex::encode(output.cmu),
            ephemeral_key: hex::encode(output.ephemeral_key),
            enc_ciphertext: hex::encode(output.enc_ciphertext),
            out_ciphertext: hex::encode(output.out_ciphertext),
            zkproof: hex::encode(output.zkproof),
        }).collect();
        
        SaplingInfo {
            value_balance: sap.value_balance as f64 / 100_000_000.0, // Convert satoshis to PIV
            shielded_spend_count: sap.vshielded_spend.len() as u64,
            shielded_output_count: sap.vshielded_output.len() as u64,
            binding_sig: hex::encode(sap.binding_sig),
            spends: Some(spends),
            outputs: Some(outputs),
        }
    });
    
    Ok(TransactionSummary {
        txid: tx.txid.clone(),
        version: tx.version,
        size: data.len(),
        locktime: tx.lock_time,
        vin,
        vout,
        value_in: 0.0, // Will be calculated after enrichment
        value_out,
        fees: 0.0,     // Will be calculated after enrichment
        tx_type,
        reward,
        sapling,
    })
}

