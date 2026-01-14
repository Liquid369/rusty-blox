// API Helper Functions
//
// Shared utilities used across API modules.

use pivx_rpc_rs::PivxRpcClient;
use crate::config::get_global_config;
use axum::{Json, http::StatusCode};
use std::sync::Arc;
use super::types::BlockbookError;

/// Format atomic PIVX units (satoshis) to human-readable PIV with 8 decimals.
/// 
/// # Examples
/// ```
/// assert_eq!(format_piv_amount(100_000_000), "1.00000000");
/// assert_eq!(format_piv_amount(-50_000_000), "-0.50000000");
/// ```
pub fn format_piv_amount(amount: i64) -> String {
    let neg = amount < 0;
    let abs = if neg { (-amount) as u64 } else { amount as u64 };
    let whole = abs / 100_000_000u64;
    let frac = abs % 100_000_000u64;
    if neg {
        format!("-{}.{:08}", whole, frac)
    } else {
        format!("{}.{:08}", whole, frac)
    }
}

/// Create an RPC client from global configuration.
/// 
/// Reads from config keys:
/// - `rpc.host` (default: "127.0.0.1:51472")
/// - `rpc.user`
/// - `rpc.pass`
pub fn create_rpc_client() -> Result<Arc<PivxRpcClient>, String> {
    let config = get_global_config();
    
    let rpc_host = config
        .get_string("rpc.host")
        .unwrap_or_else(|_| "127.0.0.1:51472".to_string());
    let rpc_user = config
        .get_string("rpc.user")
        .unwrap_or_default();
    let rpc_pass = config
        .get_string("rpc.pass")
        .unwrap_or_default();
    
    Ok(PivxRpcClient::new(
        rpc_host,
        Some(rpc_user),
        Some(rpc_pass),
        3,    // timeout_seconds
        10,   // max_retries
        1000, // retry_delay_ms
    ))
}

/// Standard error result type for API handlers
pub type ApiResult<T> = Result<Json<T>, (StatusCode, Json<BlockbookError>)>;

/// Helper to create a 404 Not Found error response
pub fn not_found(message: impl Into<String>) -> (StatusCode, Json<BlockbookError>) {
    (
        StatusCode::NOT_FOUND,
        Json(BlockbookError::new(message)),
    )
}

/// Helper to create a 500 Internal Server Error response
pub fn internal_error(message: impl Into<String>) -> (StatusCode, Json<BlockbookError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(BlockbookError::new(message)),
    )
}

/// Helper to create a 400 Bad Request error response
pub fn bad_request(message: impl Into<String>) -> (StatusCode, Json<BlockbookError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(BlockbookError::new(message)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_piv_amount() {
        assert_eq!(format_piv_amount(0), "0.00000000");
        assert_eq!(format_piv_amount(1), "0.00000001");
        assert_eq!(format_piv_amount(100_000_000), "1.00000000");
        assert_eq!(format_piv_amount(123_456_789), "1.23456789");
        assert_eq!(format_piv_amount(-100_000_000), "-1.00000000");
        assert_eq!(format_piv_amount(-50_000_000), "-0.50000000");
    }
}
