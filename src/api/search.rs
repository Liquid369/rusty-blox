// Search and Mempool API Endpoints
//
// Real-time endpoints that should NOT be cached.

use axum::{Json, Extension, extract::Path as AxumPath, http::StatusCode};
use rocksdb::DB;
use std::sync::Arc;

use crate::search::{search, SearchResult};
use crate::mempool::{MempoolState, MempoolInfo};
use super::types::BlockbookError;

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
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(BlockbookError::new(format!("Search failed: {}", e)))
        ))
    }
}

/// GET /api/v2/mempool
/// Returns current mempool information.
/// 
/// **NO CACHE**: Mempool is real-time data
pub async fn mempool_v2(
    Extension(mempool_state): Extension<Arc<MempoolState>>,
) -> Result<Json<MempoolInfo>, (StatusCode, Json<BlockbookError>)> {
    let info = mempool_state.get_info().await;
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
            Json(BlockbookError::new(format!("Transaction {} not found in mempool", txid)))
        )),
    }
}
