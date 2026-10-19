// Governance and Budget API Endpoints
//
// Endpoints that proxy to PIVX RPC for governance/budget information.

use super::helpers::rpc_call_json;
use axum::{extract::Path as AxumPath, http::StatusCode, Extension, Json};
use pivx_rpc_rs::BudgetInfo as RpcBudgetInfo;
use std::sync::Arc;
use std::time::Duration;

use crate::cache::CacheManager;

/// GET /api/v2/budgetinfo
/// Returns budget proposal information from RPC.
///
/// **CACHED**: 120 second TTL
pub async fn budget_info_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<Vec<RpcBudgetInfo>>, StatusCode> {
    let result = cache
        .get_or_compute("budget:info", Duration::from_secs(120), || async {
            compute_budget_info().await
        })
        .await;

    match result {
        Ok(info) => Ok(Json(info)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn compute_budget_info(
) -> Result<Vec<RpcBudgetInfo>, Box<dyn std::error::Error + Send + Sync>> {
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
    let cache_key = format!("budget:votes:{proposal_name}");
    let proposal_clone = proposal_name.clone();

    let result = cache
        .get_or_compute(&cache_key, Duration::from_secs(120), || async move {
            compute_budget_votes(&proposal_clone).await
        })
        .await;

    match result {
        Ok(votes) => Ok(Json(votes)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn compute_budget_votes(
    proposal_name: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    // params serialized via serde_json; no string interpolation (JSON-injection safe)
    match rpc_call_json("getbudgetvotes", serde_json::json!([proposal_name])).await {
        Ok(votes) => Ok(votes),
        Err(e) => {
            // P2-8: an unknown/empty proposal makes the node return an RPC error
            // ("Unknown proposal name"). That is a 200-with-empty-array case, not a
            // 500. Genuine RPC/transport failures still propagate as errors.
            let msg = e.to_string().to_lowercase();
            if msg.contains("unknown proposal") || msg.contains("invalid proposal") {
                Ok(serde_json::json!([]))
            } else {
                Err(e)
            }
        }
    }
}

/// GET /api/v2/budgetprojection
/// Returns budget projection from RPC.
///
/// **CACHED**: 120 second TTL
pub async fn budget_projection_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let result = cache
        .get_or_compute("budget:projection", Duration::from_secs(120), || async {
            compute_budget_projection().await
        })
        .await;

    match result {
        Ok(projection) => Ok(Json(projection)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn compute_budget_projection(
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    rpc_call_json("getbudgetprojection", serde_json::json!([])).await
}

/// GET /api/v2/finalizedbudgets
/// Finalized budgets the node currently tracks (RPC `mnfinalbudget show`): an
/// object keyed "Name (hash)", each with FeeTX (the collateral txid), BlockStart/
/// End, Proposals (names), VoteCount, Status, IsValid. Lets a budget-finalization
/// collateral tx be resolved to its budget by matching FeeTX == txid. NOTE: the
/// node prunes old finalized budgets, so historical ones may be absent.
///
/// **CACHED**: 120 second TTL
pub async fn finalized_budgets_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let result = cache
        .get_or_compute("budget:finalized", Duration::from_secs(120), || async {
            compute_finalized_budgets().await
        })
        .await;

    match result {
        Ok(v) => Ok(Json(v)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn compute_finalized_budgets(
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    rpc_call_json("mnfinalbudget", serde_json::json!(["show"])).await
}

fn node_rpc(m: &'static str, p: serde_json::Value) -> crate::gov::poller::RpcFut {
    Box::pin(async move { rpc_call_json(m, p).await.map_err(|e| e.to_string()) })
}

type JsonErr = (StatusCode, Json<serde_json::Value>);

fn json_err(code: StatusCode, msg: impl Into<String>) -> JsonErr {
    (code, Json(serde_json::json!({ "error": msg.into() })))
}

/// GET /api/v2/budgetvotecounts
/// Per listed proposal: votes the node holds vs votes Core counts (valid,
/// i.e. from masternodes the node sees as enabled).
///
/// **CACHED**: 120 second TTL
pub async fn budget_vote_counts_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, JsonErr> {
    cache
        .get_or_compute("budget:votecounts", Duration::from_secs(120), || async {
            let rows = crate::gov::live::vote_counts(&node_rpc).await?;
            let as_of = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Ok::<_, String>(serde_json::json!({ "as_of": as_of, "rows": rows }))
        })
        .await
        .map(Json)
        .map_err(|e| json_err(StatusCode::INTERNAL_SERVER_ERROR, e))
}

#[derive(serde::Deserialize)]
pub struct PayoutQuery {
    sb: Option<u32>,
}

/// GET /api/v2/budgetpayouts?sb=H
/// Budget payments found in blocks sb..=sb+99 (default: latest superblock).
///
/// **CACHED**: 120 second TTL
pub async fn budget_payouts_v2(
    axum::extract::Query(q): axum::extract::Query<PayoutQuery>,
    Extension(db): Extension<Arc<rocksdb::DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, JsonErr> {
    use crate::gov::live::{CYCLE, FIRST_SUPPORTED_SUPERBLOCK, MAX_PAYMENTS};
    let tip = crate::chain_state::get_chain_state(&db)
        .map_err(|e| json_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .height
        .max(0) as u32;
    let sb = q.sb.unwrap_or(tip - tip % CYCLE);
    if sb % CYCLE != 0 || sb > tip {
        return Err(json_err(
            StatusCode::BAD_REQUEST,
            format!("sb must be a multiple of {CYCLE} at or below the tip {tip}"),
        ));
    }
    if sb < FIRST_SUPPORTED_SUPERBLOCK {
        return Err(json_err(
            StatusCode::BAD_REQUEST,
            format!(
                "superblocks before {FIRST_SUPPORTED_SUPERBLOCK} paid with zerocoin stakes or fee-deducted coinstakes this scan cannot classify"
            ),
        ));
    }
    let scanned_to = tip.min(sb + MAX_PAYMENTS - 1);
    cache
        .get_or_compute(
            &format!("budget:payouts:{sb}:{scanned_to}"),
            Duration::from_secs(120),
            || async move {
                let scan_db = Arc::clone(&db);
                let mut payouts = tokio::task::spawn_blocking(move || {
                    crate::gov::live::scan_payouts(&scan_db, sb, tip)
                })
                .await
                .map_err(|e| e.to_string())??;
                let info = node_rpc("getbudgetinfo", serde_json::json!([])).await?;
                let mut gaps = Vec::new();
                let proposals = crate::gov::poller::rows(&info, "getbudgetinfo", &mut gaps);
                if !gaps.is_empty() {
                    return Err(gaps.join("; "));
                }
                crate::gov::live::name_payouts(&mut payouts, &proposals, sb);
                let paid: i64 = payouts
                    .iter()
                    .filter(|p| p.kind == "paid")
                    .map(|p| p.amount_sats)
                    .sum();
                let unclassified = payouts.iter().filter(|p| p.kind != "paid").count();
                Ok::<_, String>(serde_json::json!({
                    "sb": sb,
                    "scanned_to": scanned_to,
                    "complete": tip >= sb + MAX_PAYMENTS - 1,
                    "payouts": payouts,
                    "total_paid_sats": paid.to_string(),
                    "unclassified": unclassified,
                }))
            },
        )
        .await
        .map(Json)
        .map_err(|e| json_err(StatusCode::INTERNAL_SERVER_ERROR, e))
}
