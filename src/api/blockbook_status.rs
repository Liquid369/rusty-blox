// Blockbook drop-in surface: the root status envelope, the JSON 404 catch-all,
// /estimatefee and /tx-specific. Shapes verified against a live Blockbook 0.6.0
// (btc1.trezor.io, 2026-08-31): root = {"blockbook":{...},"backend":{...}},
// errors = {"error":"<string>"}.

use axum::{http::StatusCode, Extension, Json};
use rocksdb::DB;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use super::helpers::rpc_call_json;
use crate::cache::CacheManager;
use crate::mempool::MempoolState;

pub use axum::extract::Path as AxumPath;

/// Unix seconds to RFC3339 UTC (composes the existing civil-from-days date).
fn rfc3339(ts: u64) -> String {
    let (h, m, s) = ((ts % 86_400) / 3600, (ts % 3600) / 60, ts % 60);
    format!(
        "{}T{h:02}:{m:02}:{s:02}Z",
        crate::enrich_addresses::unix_to_date(ts)
    )
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// GET /api, /api/v2 (and trailing-slash forms): the status envelope every
/// Blockbook client health-checks first. These paths previously fell through
/// to the SPA and answered with HTML.
pub async fn blockbook_root_v2(
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
    Extension(mempool): Extension<Arc<MempoolState>>,
) -> Json<serde_json::Value> {
    let (height, in_sync) = match crate::chain_state::get_chain_state(&db) {
        Ok(cs) => (cs.height, cs.synced),
        Err(_) => (0, false),
    };

    // Tip header nTime for lastBlockTime (chain_metadata height -> hash,
    // blocks CF hash -> header, nTime at [68..72]).
    let last_block_time = (|| -> Option<u64> {
        let cf_meta = db.cf_handle("chain_metadata")?;
        let cf_blocks = db.cf_handle("blocks")?;
        let display = db.get_cf(&cf_meta, height.to_le_bytes()).ok()??;
        let internal: Vec<u8> = display.iter().rev().cloned().collect();
        let header = db.get_cf(&cf_blocks, &internal).ok()??;
        if header.len() >= 72 {
            Some(u32::from_le_bytes(header[68..72].try_into().ok()?) as u64)
        } else {
            None
        }
    })()
    .unwrap_or_else(now_secs);

    let mempool_size = mempool.get_info().await.size;

    // backend{} needs two node RPCs; cache 15s (single-flight) and degrade to
    // what the index knows if the node is unreachable, so the health probe
    // itself never 500s.
    let backend = cache
        .get_or_compute("bb:backend", Duration::from_secs(15), || async {
            let chain_info = rpc_call_json("getblockchaininfo", serde_json::json!([])).await?;
            let net_info = rpc_call_json("getnetworkinfo", serde_json::json!([])).await?;
            Ok::<serde_json::Value, Box<dyn std::error::Error + Send + Sync>>(serde_json::json!({
                "chain": chain_info.get("chain").cloned().unwrap_or("main".into()),
                "blocks": chain_info.get("blocks").cloned().unwrap_or(0.into()),
                "headers": chain_info.get("headers").cloned().unwrap_or(0.into()),
                "bestBlockHash": chain_info.get("bestblockhash").cloned().unwrap_or("".into()),
                "difficulty": chain_info.get("difficulty").map(|d| d.to_string()).unwrap_or_default(),
                "sizeOnDisk": chain_info.get("size_on_disk").cloned().unwrap_or(0.into()),
                "version": net_info.get("version").map(|v| v.to_string()).unwrap_or_default(),
                "subversion": net_info.get("subversion").cloned().unwrap_or("".into()),
                "protocolVersion": net_info.get("protocolversion").map(|v| v.to_string()).unwrap_or_default(),
            }))
        })
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "blockbook root: backend RPC unavailable, serving degraded");
            serde_json::json!({ "chain": "main", "blocks": height, "error": "backend unavailable" })
        });

    Json(serde_json::json!({
        "blockbook": {
            "coin": "PIVX",
            "host": std::env::var("HOSTNAME").unwrap_or_else(|_| "rustyblox".to_string()),
            "version": env!("CARGO_PKG_VERSION"),
            "syncMode": true,
            "initialSync": false,
            "inSync": in_sync,
            "bestHeight": height,
            "lastBlockTime": rfc3339(last_block_time),
            "inSyncMempool": true,
            "lastMempoolTime": rfc3339(now_secs()),
            "mempoolSize": mempool_size,
            "decimals": 8,
            "about": "rusty-blox: PIVX block explorer with a Blockbook v2 compatible API",
        },
        "backend": backend,
    }))
}

/// Catch-all for unrouted /api paths. These used to fall through to the SPA
/// fallback and answer HTTP 200 with HTML, which told API clients a missing
/// endpoint existed and then fed them a webpage.
pub async fn api_not_found() -> (StatusCode, Json<super::types::BlockbookError>) {
    (
        StatusCode::NOT_FOUND,
        Json(super::types::BlockbookError::new("Not found")),
    )
}

/// GET /api/v2/estimatefee/{blocks}: {"result":"<PIV per kB>"} as an 8-decimal
/// string (Blockbook shape). estimatesmartfee first, estimatefee fallback, and
/// the 0.0001 PIV/kB relay minimum when the node has no estimate (a fresh or
/// quiet node returns -1) so wallets always get a spendable rate.
pub async fn estimate_fee_v2(
    AxumPath(blocks): AxumPath<u32>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Json<serde_json::Value> {
    const RELAY_FLOOR: f64 = 0.0001;
    let n = blocks.clamp(1, 1008);
    let rate = cache
        .get_or_compute(
            &format!("bb:fee:{n}"),
            Duration::from_secs(15),
            || async move {
                let smart = rpc_call_json("estimatesmartfee", serde_json::json!([n])).await;
                let rate = match smart {
                    Ok(v) => v.get("feerate").and_then(|f| f.as_f64()),
                    Err(_) => rpc_call_json("estimatefee", serde_json::json!([n]))
                        .await
                        .ok()
                        .and_then(|v| v.as_f64()),
                };
                Ok::<f64, Box<dyn std::error::Error + Send + Sync>>(match rate {
                    Some(r) if r > 0.0 => r,
                    _ => RELAY_FLOOR,
                })
            },
        )
        .await
        .unwrap_or(RELAY_FLOOR);
    Json(serde_json::json!({ "result": format!("{rate:.8}") }))
}

/// GET /api/v2/tx-specific/{txid}: the node's verbose getrawtransaction,
/// passed through untouched (Blockbook's escape hatch for coin-specific data).
pub async fn tx_specific_v2(
    AxumPath(txid): AxumPath<String>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<super::types::BlockbookError>)> {
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(super::types::BlockbookError::new("Invalid txid")),
        ));
    }
    let txid_clone = txid.clone();
    let result = cache
        .get_or_compute(
            &format!("bb:txspec:{txid}"),
            Duration::from_secs(60),
            || async move {
                rpc_call_json("getrawtransaction", serde_json::json!([txid_clone, 1])).await
            },
        )
        .await;
    match result {
        Ok(v) => Ok(Json(v)),
        Err(_) => Err((
            StatusCode::NOT_FOUND,
            Json(super::types::BlockbookError::new(format!(
                "Transaction '{txid}' not found"
            ))),
        )),
    }
}
