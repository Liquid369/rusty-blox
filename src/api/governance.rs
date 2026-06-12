// Governance and Budget API Endpoints
//
// Endpoints that proxy to PIVX RPC for governance/budget information.

use axum::{Json, Extension, extract::Path as AxumPath, http::StatusCode};
use pivx_rpc_rs::BudgetInfo as RpcBudgetInfo;
use std::sync::Arc;
use std::time::Duration;
use super::helpers::rpc_call_json;

use crate::cache::CacheManager;

/// GET /api/v2/budgetinfo
/// Returns budget proposal information from RPC.
/// 
/// **CACHED**: 120 second TTL
pub async fn budget_info_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<Vec<RpcBudgetInfo>>, StatusCode> {
    let result = cache
        .get_or_compute(
            "budget:info",
            Duration::from_secs(120),
            || async {
                compute_budget_info().await
            }
        )
        .await;
    
    match result {
        Ok(info) => Ok(Json(info)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn compute_budget_info() -> Result<Vec<RpcBudgetInfo>, Box<dyn std::error::Error + Send + Sync>> {
    let result = rpc_call_json("getbudgetinfo", serde_json::json!([])).await?;
    Ok(serde_json::from_value(result)?)
}

/// GET /api/v2/budgetvotes/{proposal}
/// Returns votes for a specific budget proposal.
/// 
/// **CACHED**: 120 second TTL
pub async fn budget_votes_v2(
    AxumPath(proposal_name): AxumPath<String>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let cache_key = format!("budget:votes:{}", proposal_name);
    let proposal_clone = proposal_name.clone();
    
    let result = cache
        .get_or_compute(
            &cache_key,
            Duration::from_secs(120),
            || async move {
                compute_budget_votes(&proposal_clone).await
            }
        )
        .await;
    
    match result {
        Ok(votes) => Ok(Json(votes)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn compute_budget_votes(proposal_name: &str) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    // params serialized via serde_json — no string interpolation (JSON-injection safe)
    rpc_call_json("getbudgetvotes", serde_json::json!([proposal_name])).await
}

/// GET /api/v2/budgetprojection
/// Returns budget projection from RPC.
/// 
/// **CACHED**: 120 second TTL
pub async fn budget_projection_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let result = cache
        .get_or_compute(
            "budget:projection",
            Duration::from_secs(120),
            || async {
                compute_budget_projection().await
            }
        )
        .await;
    
    match result {
        Ok(projection) => Ok(Json(projection)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn compute_budget_projection() -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    rpc_call_json("getbudgetprojection", serde_json::json!([])).await
}
