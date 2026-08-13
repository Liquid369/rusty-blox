// Masternode API Endpoints
//
// Endpoints that proxy to PIVX RPC for masternode information.

use super::helpers::rpc_call_json;
use axum::{extract::Path as AxumPath, http::StatusCode, Extension, Json};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use super::types::MNCount;
use crate::cache::CacheManager;

/// GET /api/v2/mncount
/// Returns masternode count statistics from RPC.
///
/// **CACHED**: 60 second TTL
pub async fn mn_count_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<MNCount>, StatusCode> {
    let result = cache
        .get_or_compute("mn:count", Duration::from_secs(60), || async {
            compute_mn_count().await
        })
        .await;

    match result {
        Ok(count) => Ok(Json(count)),
        Err(e) => {
            warn!(error = %e, "mncount failed");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn compute_mn_count() -> Result<MNCount, Box<dyn std::error::Error + Send + Sync>> {
    let result = rpc_call_json("getmasternodecount", serde_json::json!([])).await?;
    Ok(serde_json::from_value(result)?)
}

/// GET /api/v2/mnlist
/// Returns full masternode list from RPC.
///
/// **CACHED**: 60 second TTL
pub async fn mn_list_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let result = cache
        .get_or_compute("mn:list", Duration::from_secs(60), || async {
            compute_mn_list().await
        })
        .await;

    match result {
        Ok(list) => Ok(Json(list)),
        Err(e) => {
            warn!(error = %e, "mnlist failed");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Pure proxy: serve pivxd's rows UNTYPED. The old strict
/// `Vec<pivx_rpc_rs::MasternodeList>` round-trip (rigid schema, `outidx: i8`,
/// no optional fields) 500'd the ENTIRE list the day one row's shape moved —
/// and the endpoint only re-serializes to JSON, so the typing bought nothing.
/// The frontend renders row fields defensively.
async fn compute_mn_list() -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let result = rpc_call_json("listmasternodes", serde_json::json!([])).await?;
    if !result.is_array() {
        return Err(format!(
            "listmasternodes returned non-array JSON ({})",
            match &result {
                serde_json::Value::Object(_) => "object",
                other => other.as_str().unwrap_or("scalar"),
            }
        )
        .into());
    }
    Ok(result)
}

/// GET /api/v2/relaymnb/{hex}
/// Relay masternode broadcast message.
///
/// **NO CACHE**: Write operation
pub async fn relay_mnb_v2(AxumPath(param): AxumPath<String>) -> Result<Json<String>, StatusCode> {
    // Validate hex before touching the node (was: arbitrary input forwarded on a
    // freshly spawned OS thread per request)
    let param = param.trim().to_string();
    if param.is_empty()
        || param.len() > 100_000
        || param.len() % 2 != 0
        || !param.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    match rpc_call_json("relaymasternodebroadcast", serde_json::json!([param])).await {
        Ok(result) => {
            let msg = result
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| result.to_string());
            Ok(Json(msg))
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}
