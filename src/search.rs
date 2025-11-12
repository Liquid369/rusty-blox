/// Search Functionality
/// 
/// Universal search that handles:
/// - Block height (numeric)
/// - Block hash (64 hex chars)
/// - Transaction ID (64 hex chars)
/// - Address (base58, starts with D for PIVX)

use std::sync::Arc;
use rocksdb::DB;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SearchResult {
    Block {
        height: i32,
        hash: String,
    },
    Transaction {
        txid: String,
        block_height: Option<i32>,
    },
    Address {
        address: String,
        balance: Option<String>,
    },
    NotFound {
        query: String,
    },
}

/// Detect what type of search query this is
fn detect_query_type(query: &str) -> QueryType {
    // Remove whitespace
    let q = query.trim();
    
    // Numeric = block height
    if q.chars().all(|c| c.is_numeric()) {
        return QueryType::BlockHeight;
    }
    
    // 64 hex chars = block hash or txid
    if q.len() == 64 && q.chars().all(|c| c.is_ascii_hexdigit()) {
        return QueryType::HashOrTxid;
    }
    
    // Starts with D = PIVX address
    if q.starts_with('D') && q.len() >= 26 && q.len() <= 35 {
        return QueryType::Address;
    }
    
    QueryType::Unknown
}

enum QueryType {
    BlockHeight,
    HashOrTxid,
    Address,
    Unknown,
}

/// Universal search
pub fn search(db: &Arc<DB>, query: &str) -> Result<SearchResult, Box<dyn std::error::Error>> {
    let query_type = detect_query_type(query);
    
    match query_type {
        QueryType::BlockHeight => search_block_by_height(db, query),
        QueryType::HashOrTxid => search_hash_or_txid(db, query),
        QueryType::Address => search_address(db, query),
        QueryType::Unknown => Ok(SearchResult::NotFound {
            query: query.to_string(),
        }),
    }
}

/// Search for block by height
fn search_block_by_height(db: &Arc<DB>, query: &str) -> Result<SearchResult, Box<dyn std::error::Error>> {
    let height: i32 = query.parse()?;
    
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    let height_key = height.to_le_bytes().to_vec();
    
    match db.get_cf(&cf_metadata, &height_key)? {
        Some(hash_bytes) => {
            let hash = hex::encode(&hash_bytes);
            Ok(SearchResult::Block {
                height,
                hash,
            })
        }
        None => Ok(SearchResult::NotFound {
            query: query.to_string(),
        }),
    }
}

/// Search for block hash or transaction ID
fn search_hash_or_txid(db: &Arc<DB>, query: &str) -> Result<SearchResult, Box<dyn std::error::Error>> {
    // Try as block hash first
    if let Ok(result) = search_block_by_hash(db, query) {
        if !matches!(result, SearchResult::NotFound { .. }) {
            return Ok(result);
        }
    }
    
    // Try as transaction ID
    search_transaction(db, query)
}

/// Search for block by hash
fn search_block_by_hash(db: &Arc<DB>, hash: &str) -> Result<SearchResult, Box<dyn std::error::Error>> {
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    let hash_bytes = hex::decode(hash)?;
    
    // Method 1: Try direct hash -> height lookup (for newly indexed blocks)
    let mut key = vec![b'h'];
    key.extend_from_slice(&hash_bytes);
    
    if let Some(height_bytes) = db.get_cf(&cf_metadata, &key)? {
        let height = i32::from_le_bytes(height_bytes.as_slice().try_into()?);
        return Ok(SearchResult::Block {
            height,
            hash: hash.to_string(),
        });
    }
    
    // Method 2: Fallback - iterate through height -> hash mappings (for old blocks)
    // This searches all numeric keys (heights) to find matching hash
    let iter = db.iterator_cf(&cf_metadata, rocksdb::IteratorMode::Start);
    
    for item in iter {
        if let Ok((key, value)) = item {
            // Skip non-numeric keys (like 'h' prefix keys)
            if key.len() == 4 && key[0] != b'h' {
                // This is a height key (4 bytes little-endian)
                if value.as_ref() == hash_bytes.as_slice() {
                    let height = i32::from_le_bytes(key.as_ref().try_into()?);
                    return Ok(SearchResult::Block {
                        height,
                        hash: hash.to_string(),
                    });
                }
            }
        }
    }
    
    Ok(SearchResult::NotFound {
        query: hash.to_string(),
    })
}

/// Search for transaction
fn search_transaction(db: &Arc<DB>, txid: &str) -> Result<SearchResult, Box<dyn std::error::Error>> {
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    // Transaction key format: 't' + txid_bytes
    let txid_bytes = hex::decode(txid)?;
    let mut key = vec![b't'];
    key.extend_from_slice(&txid_bytes);
    
    match db.get_cf(&cf_transactions, &key)? {
        Some(tx_data) => {
            // Data format: version (4 bytes) + height (4 bytes) + JSON
            let block_height = if tx_data.len() >= 8 {
                Some(i32::from_le_bytes(tx_data[4..8].try_into()?))
            } else {
                None
            };
            
            Ok(SearchResult::Transaction {
                txid: txid.to_string(),
                block_height,
            })
        }
        None => Ok(SearchResult::NotFound {
            query: txid.to_string(),
        }),
    }
}

/// Search for address
fn search_address(db: &Arc<DB>, address: &str) -> Result<SearchResult, Box<dyn std::error::Error>> {
    let cf_addr = db.cf_handle("addr_index")
        .ok_or("addr_index CF not found")?;
    
    // Address key format: 'a' + address
    let mut key = vec![b'a'];
    key.extend_from_slice(address.as_bytes());
    
    match db.get_cf(&cf_addr, &key)? {
        Some(_utxo_data) => {
            // TODO: Calculate balance from UTXO data
            Ok(SearchResult::Address {
                address: address.to_string(),
                balance: None,
            })
        }
        None => Ok(SearchResult::NotFound {
            query: address.to_string(),
        }),
    }
}
