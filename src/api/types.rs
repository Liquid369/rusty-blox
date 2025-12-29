// API Type Definitions
// 
// All serializable types used by API endpoints.
// Extracted from the monolithic api.rs for better organization.

use serde::{Deserialize, Serialize};

// ========== XPub Types ==========

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct XPubToken {
    #[serde(rename = "type")]
    pub token_type: String,
    pub name: String,
    pub path: String,
    pub transfers: u32,
    pub decimals: u32,
    pub balance: String,
    #[serde(rename = "totalReceived")]
    pub total_received: String,
    #[serde(rename = "totalSent")]
    pub total_sent: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<XPubToken>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transactions: Option<Vec<Transaction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "usedTokens")]
    pub used_tokens: Option<u32>,
}

// ========== Address Types ==========

#[derive(Serialize, Deserialize, Debug, Clone)]
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

// ========== UTXO Types ==========

#[derive(Serialize, Deserialize, Debug, Clone)]
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

// ========== Money Supply Types ==========

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MoneySupply {
    pub moneysupply: f64,
    pub transparentsupply: f64,
    pub shieldsupply: f64,
}

// ========== Masternode Types ==========

#[derive(Serialize, Deserialize, Debug, Clone)]
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MNList {
    pub masternodes: Vec<MasternodeInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MNCount {
    pub total: i32,
    pub stable: i32,
    pub enabled: i32,
    pub inqueue: i32,
    pub ipv4: i32,
    pub ipv6: i32,
    pub onion: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RelayMNB {
    pub hexstring: String,
}

// ========== Transaction Types ==========

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Transaction {
    pub txid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "lockTime")]
    pub lock_time: Option<u32>,
    pub vin: Vec<TxInput>,
    pub vout: Vec<TxOutput>,
    #[serde(rename = "blockHash")]
    pub block_hash: String,
    #[serde(rename = "blockHeight")]
    pub block_height: i32, // i32 to support -1 for mempool txs
    pub confirmations: u32,
    #[serde(rename = "blockTime")]
    pub block_time: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vsize: Option<usize>,
    pub value: String,
    #[serde(rename = "valueIn")]
    pub value_in: String,
    pub fees: String,
    pub hex: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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

#[derive(Serialize, Deserialize, Debug, Clone)]
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SendTxResponse {
    pub result: Option<String>,
    pub error: Option<TxError>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TxError {
    pub message: String,
}

// ========== Block Types ==========

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlockHash {
    #[serde(rename = "blockHash")]
    pub block_hash: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BlockQuery {
    pub block_hash: Option<String>,
    pub block_height: Option<i32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BlockParams {
    pub block_height: i32,
}

// ========== Budget/Governance Types ==========

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

// ========== Query Parameter Types ==========

// Custom deserializer for from parameter that accepts "-Infinity" (MPW compatibility)
fn deserialize_from_param<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    
    let value: Option<String> = Option::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(s) if s == "-Infinity" => Ok(Some(0)), // Treat -Infinity as 0 (from beginning)
        Some(s) => s.parse::<u32>()
            .map(Some)
            .map_err(|_| D::Error::custom(format!("Invalid 'from' parameter: {}", s))),
    }
}

fn default_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    1000
}

fn default_details() -> String {
    "txids".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct AddressQuery {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    #[serde(rename = "pageSize")]
    pub page_size: u32,
    #[serde(default, deserialize_with = "deserialize_from_param")]
    pub from: Option<u32>,
    pub to: Option<u32>,
    #[serde(default = "default_details")]
    pub details: String,
    pub contract: Option<String>,
    pub secondary: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UtxoQuery {
    #[serde(default)]
    pub confirmed: bool,
}

// ========== Error Types ==========

/// Blockbook-compatible error response wrapper
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlockbookError {
    pub error: ErrorDetail,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ErrorDetail {
    pub message: String,
}

impl BlockbookError {
    pub fn new(message: impl Into<String>) -> Self {
        BlockbookError {
            error: ErrorDetail {
                message: message.into(),
            },
        }
    }
}
