// Search and Mempool API Endpoints
//
// Real-time endpoints that should NOT be cached.

use axum::{extract::Path as AxumPath, http::StatusCode, Extension, Json};
use rocksdb::DB;
use std::sync::Arc;

use super::types::BlockbookError;
use crate::mempool::{MempoolInfo, MempoolState};
use crate::search::{search, SearchResult};

/// GET /api/v2/search/{query}
/// Universal search for blocks, transactions, or addresses.
///
/// **NO CACHE**: Search results are real-time
pub async fn search_v2(
    AxumPath(query): AxumPath<String>,
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<SearchResult>, (StatusCode, Json<BlockbookError>)> {
    match search(&db, &query) {
        Ok(result) => Ok(Json(result)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, {
            tracing::error!(error = %e, "search failed");
            Json(BlockbookError::new("Search failed"))
        })),
    }
}

/// GET /api/v2/mempool
/// Returns current mempool information.
///
/// **CACHED 2s**: the snapshot clones every pending tx; a 2s TTL keeps it
/// effectively real-time (10s poll cadence) while bounding clone work under
/// a request burst.
pub async fn mempool_v2(
    Extension(mempool_state): Extension<Arc<MempoolState>>,
    Extension(cache): Extension<Arc<crate::cache::CacheManager>>,
) -> Result<Json<MempoolInfo>, (StatusCode, Json<BlockbookError>)> {
    let info = cache
        .get_or_compute(
            "mempool:snapshot",
            std::time::Duration::from_secs(2),
            || async move {
                Ok::<MempoolInfo, Box<dyn std::error::Error + Send + Sync>>(
                    mempool_state.get_info().await,
                )
            },
        )
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(BlockbookError::new("Internal error reading mempool")),
            )
        })?;
    Ok(Json(info))
}

/// GET /api/v2/mempool/{txid}
/// Returns specific mempool transaction.
///
/// **NO CACHE**: Mempool data is ephemeral
pub async fn mempool_tx_v2(
    AxumPath(txid): AxumPath<String>,
    Extension(mempool_state): Extension<Arc<MempoolState>>,
) -> Result<Json<crate::mempool::MempoolTransaction>, (StatusCode, Json<BlockbookError>)> {
    match mempool_state.get_transaction(&txid).await {
        Some(tx) => Ok(Json(tx)),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(BlockbookError::new(format!(
                "Transaction {txid} not found in mempool"
            ))),
        )),
    }
}
