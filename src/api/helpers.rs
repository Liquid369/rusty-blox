// API Helper Functions
//
// Shared utilities used across API modules.

use super::types::BlockbookError;
use crate::config::get_global_config;
use axum::{http::StatusCode, Json};
use pivx_rpc_rs::PivxRpcClient;
use std::sync::Arc;

/// Format atomic PIVX units (satoshis) to human-readable PIV with 8 decimals.
///
/// # Examples
/// ```
/// use rustyblox::api::helpers::format_piv_amount;
/// assert_eq!(format_piv_amount(100_000_000), "1.00000000");
/// assert_eq!(format_piv_amount(-50_000_000), "-0.50000000");
/// ```
pub fn format_piv_amount(amount: i64) -> String {
    let neg = amount < 0;
    let abs = if neg { (-amount) as u64 } else { amount as u64 };
    let whole = abs / 100_000_000u64;
    let frac = abs % 100_000_000u64;
    if neg {
        format!("-{whole}.{frac:08}")
    } else {
        format!("{whole}.{frac:08}")
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
    let rpc_user = config.get_string("rpc.user").unwrap_or_default();
    let rpc_pass = config.get_string("rpc.pass").unwrap_or_default();

    Ok(PivxRpcClient::new(
        rpc_host,
        Some(rpc_user),
        Some(rpc_pass),
        3,    // timeout_seconds
        10,   // max_retries
        1000, // retry_delay_ms
    ))
}

lazy_static::lazy_static! {
    /// Shared async HTTP client for node RPC proxy calls. A hard timeout is
    /// essential: without one a hung node would pin requests forever.
    static ref RPC_HTTP_CLIENT: reqwest::Client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("failed to build RPC HTTP client");
}

/// Flatten a reqwest transport error to a client-safe message. reqwest's Display
/// embeds the target URL, which would leak the internal RPC host:port to anonymous
/// callers (e.g. /sendtx echoes this error verbatim). Node *rejection* reasons do
/// NOT pass through here — they're parsed from the JSON body and forwarded intact.
fn rpc_transport_error(e: reqwest::Error) -> Box<dyn std::error::Error + Send + Sync> {
    // Full detail (incl. URL) to the operator log only; a fixed, URL-free string
    // to the caller. Never interpolate `e` into the returned message — its Display
    // carries the URL.
    tracing::debug!(error = %e, "RPC transport error (sanitized for client)");
    if e.is_timeout() {
        "node RPC request timed out".into()
    } else if e.is_connect() {
        "node RPC connection failed".into()
    } else if e.is_decode() {
        "node RPC returned an unreadable response".into()
    } else {
        "node RPC request failed".into()
    }
}

/// Async JSON-RPC call to the PIVX node.
///
/// Replaces the previous per-endpoint synchronous `TcpStream` implementations,
/// which performed blocking reads (15s timeouts) directly on tokio worker
/// threads — a handful of concurrent requests could stall the entire server.
/// This version is fully async and JSON-injection-safe (params are serialized,
/// never string-interpolated).
pub async fn rpc_call_json(
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let config = get_global_config();
    let rpc_host = config
        .get_string("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    // Fail closed: no hardcoded credential fallbacks.
    let rpc_user = config
        .get_string("rpc.user")
        .map_err(|_| "rpc.user not configured")?;
    let rpc_pass = config
        .get_string("rpc.pass")
        .map_err(|_| "rpc.pass not configured")?;
    let url = if rpc_host.starts_with("http://") || rpc_host.starts_with("https://") {
        rpc_host
    } else {
        format!("http://{rpc_host}")
    };

    let body = serde_json::json!({
        "jsonrpc": "1.0",
        "id": "rustyblox",
        "method": method,
        "params": params,
    });
    let resp = RPC_HTTP_CLIENT
        .post(&url)
        .basic_auth(&rpc_user, Some(&rpc_pass))
        .json(&body)
        .send()
        .await
        .map_err(rpc_transport_error)?;
    let value: serde_json::Value = resp.json().await.map_err(rpc_transport_error)?;
    if let Some(err) = value.get("error").filter(|e| !e.is_null()) {
        return Err(format!("RPC error: {err}").into());
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| "No result in RPC response".into())
}

/// Standard error result type for API handlers
pub type ApiResult<T> = Result<Json<T>, (StatusCode, Json<BlockbookError>)>;

/// Helper to create a 404 Not Found error response
pub fn not_found(message: impl Into<String>) -> (StatusCode, Json<BlockbookError>) {
    (StatusCode::NOT_FOUND, Json(BlockbookError::new(message)))
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
    (StatusCode::BAD_REQUEST, Json(BlockbookError::new(message)))
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

    /// P3: /sendtx echoes rpc_call_json's error verbatim. A transport failure
    /// must not leak the internal RPC URL (host:port + path) to anonymous callers.
    #[tokio::test]
    async fn rpc_transport_error_does_not_leak_url() {
        // Connection refused to port 1 yields a reqwest error whose Display
        // includes the URL; the sanitizer must strip it.
        let err = reqwest::Client::new()
            .get("http://127.0.0.1:1/internal-rpc-path")
            .send()
            .await
            .expect_err("connect to port 1 must fail");
        let msg = rpc_transport_error(err).to_string();
        assert!(!msg.contains("127.0.0.1"), "leaked host: {msg}");
        assert!(!msg.contains("internal-rpc-path"), "leaked path: {msg}");
        assert!(!msg.contains("://"), "leaked URL: {msg}");
    }
}
