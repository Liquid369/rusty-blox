use rusty_piv::{GetRawTransactionInfo, BlockChainInfo, MasternodeList, MasternodeCount, Block, FullBlock, PivxStatus, BitcoinRpcClient, BudgetInfo};
use std::{
    io::{self, BufRead, Cursor, ErrorKind, Read, Seek, SeekFrom}
};
use std::error::Error;
use axum::{
    extract::{Query, Extension, Path},
    Json,
    http::StatusCode,
    Router,
};
use crate::AxumPath;
use rocksdb::DB;
use hex;
use crate::{
    read_varint,
    deserialize_utxos,
    deserialize_transaction,
    deserialize_out_point,
    deserialize_tx_in,
    deserialize_tx_out,
};
use crate::{CScript, CTransaction, CTxOut, CTxIn, COutPoint};
use crate::config::{get_global_config, init_global_config};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use byteorder::LittleEndian;
use serde_json::{Value};
use std::sync::Arc;

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
    pub lockTime: Option<u32>,
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
    pub masternodes: Vec<MasternodeList>,
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
    pub blockHash: String,
    pub blockHeight: u32,
    pub confirmations: u32,
    pub blockTime: u64,
    pub value: String,
    pub valueIn: String,
    pub fees: String,
    pub hex: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TxInput {
    pub n: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isAddress: Option<bool>,
    pub value: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TxOutput {
    pub value: String,
    pub n: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isAddress: Option<bool>,
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
    pub blockHash: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RelayMNB {
    pub hexstring: String,
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
    let key = format!("h{}", param);
    let result = db.get(key.as_bytes())
                   .expect("Failed to read from DB")
                   .map(|value| String::from_utf8(value).expect("Invalid UTF-8"))
                   .unwrap_or_else(|| "Not found".to_string());

    Json(BlockHash{blockHash: result})
}

pub async fn tx_v2(AxumPath(param): AxumPath<String>, db: Arc<DB>) -> Result<CTransaction, Box<dyn Error>> {
    let key = format!("t{}", param);
    let result = db.get(key.as_bytes())
                   .expect("Failed to read from DB")
                   .ok_or("Transaction not found in DB")?;

    let txs = deserialize_transaction(&result)?;

    Ok(txs)
}

pub async fn addr_v2(AxumPath(param): AxumPath<String>, db: Arc<DB>) -> AddressInfo {
   let key = format!("a{}", param);
   let result = db.get(key.as_bytes())
                  .expect("Failed to read from DB")
                  .unwrap_or_else(|| "Not found".to_string().into());

    let utxos = deserialize_utxos(&result);
    let mut balance: f64 = 0.0;
    for utxo in &utxos {
        let (txid_hash, _output_index) = utxo;
        let txid_hex = hex::encode(txid_hash);
        let key = format!("t{}", txid_hex);
        if let Ok(Some(tx_data)) = db.get(key.as_bytes()) {
            let tx_value = deserialize_transaction(&tx_data); // Ensure tx_value is Result<CTransaction, _>
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
    let tx_amt: u32 = txids.len().try_into().unwrap();
    AddressInfo {
        address: param,
        balance,
        txs: tx_amt,
        txids: txids,
    }
}

pub async fn xpub_v2(AxumPath(param): AxumPath<String>, db: Arc<DB>) -> XPubInfo {
    let key = format!("p{}", param);
    let result = db.get(key.as_bytes())
                   .expect("Failed to read from DB")
                   .unwrap_or_else(|| "Not found".to_string().into());

    let utxos = deserialize_utxos(&result);
    let mut balance: f64 = 0.0;
    for utxo in &utxos {
        let (txid_hash, _output_index) = utxo;
        let txid_hex = hex::encode(txid_hash);
        let key = format!("t{}", txid_hex);
        if let Ok(Some(tx_data)) = db.get(key.as_bytes()) {
            let tx_value = deserialize_transaction(&tx_data); // Ensure tx_value is Result<CTransaction, _>
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
    let tx_amt: u32 = txids.len().try_into().unwrap();
    XPubInfo {
        address: param,
        balance,
        txs: tx_amt,
        txids: txids,
    }
}

//pub async fn utxo_v2(AxumPath(param): AxumPath<String>) -> Result<Json<UTXO>, StatusCode> {
   
//}

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
) -> Result<Json<rusty_piv::Block>, StatusCode> {
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

async fn get_block_from_db(db: &Arc<DB>, key: &[u8]) -> Result<Json<rusty_piv::Block>, StatusCode> {
    let db_clone = Arc::clone(db);
    let key_owned = key.to_vec();

    tokio::task::spawn_blocking(move || {
        let cf_handle = db_clone.cf_handle("cf_blocks").ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        let key_hex = "68d1851300";
        let key_binary = hex::decode(key_hex).expect("Decoding Failed");
        db_clone.get_cf(cf_handle, &key_binary)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
            .and_then(|value_opt| value_opt.ok_or_else(|| StatusCode::NOT_FOUND))
            .and_then(|value| {
                serde_json::from_slice::<rusty_piv::Block>(&value)
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
        rpc_host.expect("REASON"),
        Some(rpc_user.expect("REASON")),
        Some(rpc_pass.expect("REASON")),
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
    let rpc_host = config.get::<String>("rpc.host");
    let rpc_user = config.get::<String>("rpc.user");
    let rpc_pass = config.get::<String>("rpc.pass");

    let client = BitcoinRpcClient::new(
        rpc_host.expect("REASON"),
        Some(rpc_user.expect("REASON")),
        Some(rpc_pass.expect("REASON")),
        3,    // Max retries
        10,   // Connection timeout
        1000, // Read/write timeout
    );

    let result = client.getmasternodecount();

    match result {
        Ok(mn_count) => {
            let response = MNCount {
                total: mn_count.total,
                stable: mn_count.stable,
                enabled: mn_count.enabled,
                inqueue: mn_count.inqueue,
                ipv4: mn_count.ipv4,
                ipv6: mn_count.ipv6,
                onion: mn_count.onion,
            };
            Ok(Json(response))
        },
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn mn_list_v2(Path(param): Path<String>) -> Result<Json<MNList>, StatusCode> {
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host");
    let rpc_user = config.get::<String>("rpc.user");
    let rpc_pass = config.get::<String>("rpc.pass");

    let client = BitcoinRpcClient::new(
        rpc_host.expect("REASON"),
        Some(rpc_user.expect("REASON")),
        Some(rpc_pass.expect("REASON")),
        3,    // Max retries
        10,   // Connection timeout
        1000, // Read/write timeout
    );

    let result = client.listmasternodes(Some(&param));

    match result {
        Ok(masternode_data) => {
            let mn_list = MNList {
                masternodes: masternode_data,
            };
            Ok(Json(mn_list))
        },
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn money_supply_v2() -> Result<Json<MoneySupply>, StatusCode> {
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host");
    let rpc_user = config.get::<String>("rpc.user");
    let rpc_pass = config.get::<String>("rpc.pass");

    let client = BitcoinRpcClient::new(
        rpc_host.expect("REASON"),
        Some(rpc_user.expect("REASON")),
        Some(rpc_pass.expect("REASON")),
        3,    // Max retries
        10,   // Connection timeout
        1000, // Read/write timeout
    );

    let result = client.getsupplyinfo(true);

    match result {
        Ok(money_supply) => {
            let supply = MoneySupply {
                moneysupply: money_supply.totalsupply,
                transparentsupply: money_supply.transparentsupply,
                shieldsupply: money_supply.shieldsupply,
            };
            Ok(Json(supply))
        },
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn budget_info_v2() -> Result<Json<Vec<BudgetInfo>>, StatusCode> {
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host");
    let rpc_user = config.get::<String>("rpc.user");
    let rpc_pass = config.get::<String>("rpc.pass");

    let client = BitcoinRpcClient::new(
        rpc_host.expect("REASON"),
        Some(rpc_user.expect("REASON")),
        Some(rpc_pass.expect("REASON")),
        3,    // Max retries
        10,   // Connection timeout
        1000, // Read/write timeout
    );

    let result = client.getbudgetinfo();

    match result {
        Ok(budget_info) => {
            Ok(Json(budget_info))
        },
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn relay_mnb_v2(AxumPath(param): AxumPath<String>) -> Result<Json<String>, StatusCode> {
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host");
    let rpc_user = config.get::<String>("rpc.user");
    let rpc_pass = config.get::<String>("rpc.pass");

    let client = BitcoinRpcClient::new(
        rpc_host.expect("REASON"),
        Some(rpc_user.expect("REASON")),
        Some(rpc_pass.expect("REASON")),
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
