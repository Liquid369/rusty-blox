use pivx_rpc_rs::{BitcoinRpcClient, MasternodeList, BudgetInfo as RpcBudgetInfo};
use axum::{
    extract::{Path, Extension},
    Json,
    http::StatusCode,
    Router,
};
use rocksdb::DB;
use hex;
use crate::types::{CTransaction};
use crate::parser::{deserialize_transaction, deserialize_utxos};
use crate::db_utils::{db_get_blocking, db_put_blocking, db_delete_blocking};
use crate::config::{get_global_config, init_global_config};
use serde::{Deserialize, Serialize};
use serde_json;
use std::sync::Arc;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub use axum::extract::Path as AxumPath;
#[derive(Serialize, Deserialize, Debug)]
pub struct XPubInfo {
    pub address: String,
    pub balance: f64,
    pub txs: u32,
    pub txids: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AddressInfo {
    pub address: String,
    pub balance: f64,
    pub txs: u32,
    pub txids: Vec<String>,
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
    pub n: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isAddress")]
    pub is_address: Option<bool>,
    pub value: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TxOutput {
    pub value: String,
    pub n: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isAddress")]
    pub is_address: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spent: Option<bool>,
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

pub async fn tx_v2(AxumPath(param): AxumPath<String>, Extension(db): Extension<Arc<DB>>) -> Json<serde_json::Value> {
    let key = format!("t{}", param);
    let result = match db_get_blocking(db.clone(), key.as_bytes()).await {
        Ok(Some(data)) => data,
        _ => return Json(serde_json::json!({"error": "Transaction not found"})),
    };

    match deserialize_transaction(&result).await {
        Ok(_tx) => Json(serde_json::json!({"txid": param, "status": "found"})),
        Err(_) => Json(serde_json::json!({"error": "Failed to deserialize transaction"})),
    }
}

pub async fn addr_v2(AxumPath(param): AxumPath<String>, Extension(db): Extension<Arc<DB>>) -> Json<AddressInfo> {
   let key = format!("a{}", param);
   let result = match db_get_blocking(db.clone(), key.as_bytes()).await {
       Ok(Some(data)) => data,
       _ => vec![],
   };

    let utxos = deserialize_utxos(&result).await;
    let mut balance: f64 = 0.0;
    for utxo in &utxos {
        let (txid_hash, _output_index) = utxo;
        let txid_hex = hex::encode(txid_hash);
        let key = format!("t{}", txid_hex);
        if let Ok(Some(tx_data)) = db_get_blocking(db.clone(), key.as_bytes()).await {
            let tx_value = deserialize_transaction(&tx_data).await;
            if let Ok(tx) = tx_value {
                balance += tx.outputs.iter().map(|output| output.value as f64).sum::<f64>();
            }
        } else {
            eprintln!("Failed to read from DB for key: {}", key);
        }
    }
    let txids: Vec<String> = utxos.iter()
        .map(|(txid_hash, _output_index)| hex::encode(txid_hash))
        .collect();
    let tx_amt: u32 = txids.len().try_into().unwrap_or(0);
    Json(AddressInfo {
        address: param,
        balance,
        txs: tx_amt,
        txids: txids,
    })
}

pub async fn xpub_v2(AxumPath(param): AxumPath<String>, Extension(db): Extension<Arc<DB>>) -> Json<XPubInfo> {
    let key = format!("p{}", param);
    let result = match db_get_blocking(db.clone(), key.as_bytes()).await {
        Ok(Some(data)) => data,
        _ => vec![],
    };

    let utxos = deserialize_utxos(&result).await;
    let mut balance: f64 = 0.0;
    for utxo in &utxos {
        let (txid_hash, _output_index) = utxo;
        let txid_hex = hex::encode(txid_hash);
        let key = format!("t{}", txid_hex);
        if let Ok(Some(tx_data)) = db_get_blocking(db.clone(), key.as_bytes()).await {
            let tx_value = deserialize_transaction(&tx_data).await;
            if let Ok(tx) = tx_value {
                balance += tx.outputs.iter().map(|output| output.value as f64).sum::<f64>();
            }
        } else {
            eprintln!("Failed to read from DB for key: {}", key);
        }
    }
    let txids: Vec<String> = utxos.iter()
        .map(|(txid_hash, _output_index)| hex::encode(txid_hash))
        .collect();
    let tx_amt: u32 = txids.len().try_into().unwrap_or(0);
    Json(XPubInfo {
        address: param,
        balance,
        txs: tx_amt,
        txids: txids,
    })
}

pub async fn utxo_v2(AxumPath(param): AxumPath<String>, Extension(db): Extension<Arc<DB>>) -> Result<Json<Vec<UTXO>>, StatusCode> {
    // Parse the address parameter to get UTXOs
    let key = format!("a{}", param);
    let result = match db_get_blocking(db.clone(), key.as_bytes()).await {
        Ok(Some(data)) => data,
        _ => return Err(StatusCode::NOT_FOUND),
    };

    let utxos = deserialize_utxos(&result).await;
    let mut utxo_list: Vec<UTXO> = Vec::new();
    
    for (txid_hash, output_index) in &utxos {
        let txid_hex = hex::encode(txid_hash);
        let key = format!("t{}", txid_hex);
        if let Ok(Some(tx_data)) = db_get_blocking(db.clone(), key.as_bytes()).await {
            if let Ok(tx) = deserialize_transaction(&tx_data).await {
                if let Some(output) = tx.outputs.get(*output_index as usize) {
                    utxo_list.push(UTXO {
                        txid: txid_hex,
                        vout: *output_index as u32,
                        value: format!("{}", output.value),
                        confirmations: 0, // Would need block height to calculate
                        lock_time: None,
                        height: None,
                        coinbase: None,
                    });
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

async fn get_block_from_db(db: &Arc<DB>, _key: &[u8]) -> Result<Json<crate::types::Block>, StatusCode> {
    let db_clone = Arc::clone(db);

    tokio::task::spawn_blocking(move || {
        let cf_handle = db_clone.cf_handle("cf_blocks").ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        let key_hex = "68d1851300";
        let key_binary = hex::decode(key_hex).map_err(|_| StatusCode::BAD_REQUEST)?;
        db_clone.get_cf(cf_handle, &key_binary)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
            .and_then(|value_opt| value_opt.ok_or_else(|| StatusCode::NOT_FOUND))
            .and_then(|value| {
                serde_json::from_slice::<crate::types::Block>(&value)
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
                    .map(Json)
            })
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
    
    eprintln!("Attempting manual RPC call to {}", host_port);
    
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
    
    eprintln!("Sending request ({} bytes)...", request.len());
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
    eprintln!("Response received ({} bytes)", response.len());
    eprintln!("First 500 chars: {}", response_str.chars().take(500).collect::<String>());
    
    // Find the JSON body after headers
    if let Some(pos) = response_str.find("\r\n\r\n") {
        let json_start = pos + 4;
        let body = &response_str[json_start..].trim();
        eprintln!("Parsing JSON: {}", body);
        
        match serde_json::from_str::<serde_json::Value>(body) {
            Ok(value) => {
                if let Some(result) = value.get("result") {
                    eprintln!("Got result: {:?}", result);
                    match serde_json::from_value::<MNCount>(result.clone()) {
                        Ok(mn_count) => {
                            eprintln!("Success!");
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
    
    if let Err(_) = stream.write_all(request.as_bytes()) {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let mut response = Vec::new();
    if let Err(_) = stream.read_to_end(&mut response) {
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
    
    if let Err(_) = stream.write_all(request.as_bytes()) {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let mut response = Vec::new();
    if let Err(_) = stream.read_to_end(&mut response) {
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
    
    if let Err(_) = stream.write_all(request.as_bytes()) {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    let mut response = Vec::new();
    if let Err(_) = stream.read_to_end(&mut response) {
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
