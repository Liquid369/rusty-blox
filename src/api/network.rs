// Network-Level API Endpoints
//
// Endpoints that provide network-wide statistics and status information.
// These are frequently accessed and use caching for performance.

use axum::{http::StatusCode, Extension, Json};
use rocksdb::DB;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

use std::io::{Read, Write};
use std::net::TcpStream;

use super::types::{BlockbookError, MoneySupply};
use crate::cache::CacheManager;
use crate::chain_state::{get_chain_state, ChainState};
use crate::config::get_global_config;

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
        .get_or_compute("status:latest", Duration::from_secs(5), || async move {
            get_chain_state(&db_clone).map_err(|e| {
                Box::new(std::io::Error::other(format!(
                    "Failed to get chain state: {e}"
                ))) as Box<dyn std::error::Error + Send + Sync>
            })
        })
        .await;

    match result {
        Ok(state) => Ok(Json(state)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, {
            tracing::error!(error = %e, "status endpoint failed");
            Json(BlockbookError::new("Internal error"))
        })),
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
    /// Unix time (s) of the most recently indexed block; 0 if none recorded yet.
    pub last_block_time: i64,
    /// Age of the indexed tip in seconds. `None` while the index is still
    /// building; otherwise `now - last_block_time`. An uptime checker can alert
    /// on this directly (a frozen tip grows unbounded even when `synced` reads true).
    pub tip_age_seconds: Option<i64>,
    pub warnings: Vec<String>,
}

/// GET /api/v2/health
/// Health check endpoint - shows database and sync status.
///
/// **NO CACHE**: Real-time health checks should not be cached
pub async fn health_check_v2(
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<HealthStatus>, StatusCode> {
    // O(1) health check. The previous implementation iterated the ENTIRE
    // transactions and addr_index column families on every call — it was both the
    // Docker healthcheck target (every 30s) and an unauthenticated remote DoS
    // vector on a synced chain. Counts now come from RocksDB key estimates and
    // metrics persisted by the sync pipeline.
    let result = tokio::task::spawn_blocking(move || -> Result<HealthStatus, Box<dyn std::error::Error + Send + Sync>> {
        let mut warnings = Vec::new();

        let tx_cf = db.cf_handle("transactions").ok_or("transactions CF not found")?;
        let total_txs = db
            .property_int_value_cf(tx_cf, "rocksdb.estimate-num-keys")?
            .unwrap_or(0);

        // Cheap liveness probe: a point read against chain_state
        let state_cf = db.cf_handle("chain_state").ok_or("chain_state CF not found")?;
        // Align /health with the data endpoints' 503 gate (LSM-2): "complete" means
        // complete AND in the current (v2) on-disk format, not just the raw marker —
        // otherwise /health reports healthy during the v1 pre-wipe window (when every
        // data endpoint 503s) and during a v2 re-enrich.
        let raw_complete = db
            .get_cf(state_cf, b"address_index_complete")?
            .map(|v| v.first() == Some(&1u8))
            .unwrap_or(false);
        let addr_version = crate::chain_state::get_addr_index_version(&db);
        let address_index_complete = crate::chain_state::addr_index_ready(&db);

        // Address count persisted by the sync pipeline (metric_total_addresses)
        let indexed_addresses = db
            .get_cf(state_cf, b"metric_total_addresses")?
            .filter(|b| b.len() >= 8)
            .map(|b| u64::from_le_bytes(b[0..8].try_into().unwrap_or([0u8; 8])))
            .unwrap_or(0);

        if !address_index_complete && total_txs > 0 {
            if raw_complete && addr_version != crate::parser::ADDR_INDEX_FORMAT_VERSION {
                warnings.push(format!(
                    "Address index is a legacy format (v{addr_version}); it will be wiped and rebuilt to v{} automatically on the next sync. Data endpoints return 503 until then.",
                    crate::parser::ADDR_INDEX_FORMAT_VERSION
                ));
            } else {
                warnings.push(
                    "Address index is (re)building; data endpoints return 503 until it completes."
                        .to_string(),
                );
            }
        }
        if indexed_addresses == 0 && total_txs > 100_000 {
            warnings.push("Address index is empty but many transactions exist. Run rebuild_address_index.".to_string());
        }

        // Frozen-tip detection. When RPC dies mid-loop the monitor stops
        // connecting blocks AND stops advancing network_height, so `synced` still
        // reads true — the only honest signal is that the last indexed block's
        // header time stops moving. Only judged once the index is complete.
        let last_block_time = db
            .get_cf(state_cf, b"tip_block_time")?
            .filter(|b| b.len() >= 8)
            .map(|b| i64::from_le_bytes(b[0..8].try_into().unwrap_or([0u8; 8])))
            .unwrap_or(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let tip_age_seconds =
            crate::chain_state::tip_age_seconds(last_block_time, now, address_index_complete);
        // Option<i64> is Copy, so filtering here doesn't consume the field below.
        if let Some(age) =
            tip_age_seconds.filter(|a| *a > crate::chain_state::STALE_TIP_THRESHOLD_SECS)
        {
            warnings.push(format!(
                "No new block indexed in {age}s (~{} min); tip may be frozen — check pivxd RPC.",
                age / 60
            ));
        }

        let status = if warnings.is_empty() { "healthy" } else { "degraded" };

        Ok(HealthStatus {
            status: status.to_string(),
            database_ok: true,
            address_index_complete,
            // Estimated via rocksdb.estimate-num-keys (exact counts would require a
            // full CF scan — see DoS note above). Orphan breakdown is not tracked here.
            total_transactions: total_txs,
            valid_transactions: total_txs,
            orphaned_transactions: 0,
            indexed_addresses,
            last_block_time,
            tip_age_seconds,
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
        .get_or_compute("supply:latest", Duration::from_secs(300), || async {
            compute_money_supply().await
        })
        .await;

    match result {
        Ok(supply) => Ok(Json(supply)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Compute money supply from PIVX RPC (async, non-blocking)
pub async fn compute_money_supply() -> Result<MoneySupply, Box<dyn std::error::Error + Send + Sync>>
{
    // Unforced (fForceUpdate=false): the node maintains supply as it syncs, so this
    // returns the up-to-date figures in ~20ms. Passing [true] forces a full
    // chainstate recomputation (~17s) on every call and blew the RPC timeout.
    let result = super::helpers::rpc_call_json("getsupplyinfo", serde_json::json!([false])).await?;
    Ok(MoneySupply {
        moneysupply: result
            .get("totalsupply")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        transparentsupply: result
            .get("transparentsupply")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        shieldsupply: result
            .get("shieldsupply")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    })
}

/// Blocking version of compute_money_supply for use in spawn_blocking contexts
pub fn compute_money_supply_blocking(
) -> Result<MoneySupply, Box<dyn std::error::Error + Send + Sync>> {
    let config = get_global_config();
    let rpc_host = config
        .get_string("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    // Fail closed: never embed a credential in the binary. Missing config
    // yields empty creds → the node rejects auth and this errors, rather than
    // silently using a known password.
    let rpc_user = config.get_string("rpc.user").unwrap_or_default();
    let rpc_pass = config.get_string("rpc.pass").unwrap_or_default();

    let host_port = rpc_host.replace("http://", "").replace("https://", "");

    let mut stream = TcpStream::connect(&host_port)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;

    let json_body = r#"{"jsonrpc":"1.0","id":"1","method":"getsupplyinfo","params":[false]}"#;
    let auth = format!("{rpc_user}:{rpc_pass}");
    let auth_b64 = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(auth)
    };

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
        host_port,
        auth_b64,
        json_body.len(),
        json_body
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
        moneysupply: result
            .get("totalsupply")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        transparentsupply: result
            .get("transparentsupply")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        shieldsupply: result
            .get("shieldsupply")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    })
}

/// GET /api/v2/cache/stats
/// Returns cache statistics for monitoring
pub async fn cache_stats_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Json<crate::cache::CacheStats> {
    Json(cache.get_stats().await)
}
