// Network-Level API Endpoints
//
// Endpoints that provide network-wide statistics and status information.
// These are frequently accessed and use caching for performance.

use axum::{Json, Extension, http::StatusCode};
use rocksdb::DB;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use std::io::{Read, Write};
use std::net::TcpStream;

use crate::cache::CacheManager;
use crate::chain_state::{get_chain_state, ChainState};
use crate::config::get_global_config;
use super::types::{BlockbookError, MoneySupply};
use super::helpers::internal_error;

/// GET /api/v2/status
/// Returns the current status of the blockchain and explorer.
/// 
/// **CACHED**: 5 second TTL (polled frequently by frontend)
pub async fn status_v2(
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<ChainState>, (StatusCode, Json<BlockbookError>)> {
    let db_clone = Arc::clone(&db);
    
    let result = cache
        .get_or_compute(
            "status:latest",
            Duration::from_secs(5),
            || async move {
                get_chain_state(&db_clone)
                    .map_err(|e| Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Failed to get chain state: {}", e)
                    )) as Box<dyn std::error::Error + Send + Sync>)
            }
        )
        .await;
    
    match result {
        Ok(state) => Ok(Json(state)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(BlockbookError::new(format!("Failed to get chain state: {}", e)))
        ))
    }
}

#[derive(Serialize, Clone, Debug)]
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

/// GET /api/v2/health
/// Health check endpoint - shows database and sync status.
/// 
/// **NO CACHE**: Real-time health checks should not be cached
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

/// GET /api/v2/moneysupply
/// Returns money supply statistics from RPC.
/// 
/// **CACHED**: 300 second TTL (changes slowly)
pub async fn money_supply_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<MoneySupply>, StatusCode> {
    let result = cache
        .get_or_compute(
            "supply:latest",
            Duration::from_secs(300),
            || async {
                compute_money_supply().await
            }
        )
        .await;
    
    match result {
        Ok(supply) => Ok(Json(supply)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Compute money supply from PIVX RPC (direct TCP call for compatibility)
async fn compute_money_supply() -> Result<MoneySupply, Box<dyn std::error::Error + Send + Sync>> {
    let config = get_global_config();
    let rpc_host = config
        .get_string("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config
        .get_string("rpc.user")
        .unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config
        .get_string("rpc.pass")
        .unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    let host_port = rpc_host
        .replace("http://", "")
        .replace("https://", "");
    
    let mut stream = TcpStream::connect(&host_port)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    
    let json_body = r#"{"jsonrpc":"1.0","id":"1","method":"getsupplyinfo","params":[true]}"#;
    let auth = format!("{}:{}", rpc_user, rpc_pass);
    let auth_b64 = base64::encode(auth);
    
    let http_request = format!(
        "POST / HTTP/1.1\r\n\
         Host: {}\r\n\
         User-Agent: rustyblox/1.0\r\n\
         Authorization: Basic {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        host_port, auth_b64, json_body.len(), json_body
    );
    
    stream.write_all(http_request.as_bytes())?;
    
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    
    // Parse response - find JSON after headers
    let json_start = response.find("{").ok_or("Invalid response format")?;
    let json_str = &response[json_start..];
    
    let json: serde_json::Value = serde_json::from_str(json_str)?;
    let result = json.get("result").ok_or("No result in RPC response")?;
    
    Ok(MoneySupply {
        moneysupply: result.get("totalsupply").and_then(|v| v.as_f64()).unwrap_or(0.0),
        transparentsupply: result.get("transparentsupply").and_then(|v| v.as_f64()).unwrap_or(0.0),
        shieldsupply: result.get("shieldsupply").and_then(|v| v.as_f64()).unwrap_or(0.0),
    })
}

/// GET /api/v2/cache/stats
/// Returns cache statistics for monitoring
pub async fn cache_stats_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Json<crate::cache::CacheStats> {
    Json(cache.get_stats().await)
}
