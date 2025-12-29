// Governance and Budget API Endpoints
//
// Endpoints that proxy to PIVX RPC for governance/budget information.

use axum::{Json, Extension, extract::Path as AxumPath, http::StatusCode};
use pivx_rpc_rs::BudgetInfo as RpcBudgetInfo;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use crate::cache::CacheManager;
use crate::config::get_global_config;

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
    let config = get_global_config();
    let rpc_host = config.get_string("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config.get_string("rpc.user")
        .unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config.get_string("rpc.pass")
        .unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    let host_port = rpc_host.replace("http://", "").replace("https://", "");
    
    let mut stream = TcpStream::connect(&host_port)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    
    let json_body = r#"{"jsonrpc":"1.0","id":"1","method":"getbudgetinfo","params":[]}"#;
    let auth_b64 = base64::encode(format!("{}:{}", rpc_user, rpc_pass));
    
    let request = format!(
        "POST / HTTP/1.1\r\n\
         Host: {}\r\n\
         Authorization: Basic {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        host_port, auth_b64, json_body.len(), json_body
    );
    
    stream.write_all(request.as_bytes())?;
    
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    
    let response_str = String::from_utf8_lossy(&response);
    let pos = response_str.find("\r\n\r\n").ok_or("Invalid response")?;
    let body = &response_str[pos + 4..].trim();
    
    let value: serde_json::Value = serde_json::from_str(body)?;
    let result = value.get("result").ok_or("No result in response")?;
    
    Ok(serde_json::from_value(result.clone())?)
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
    let config = get_global_config();
    let rpc_host = config.get_string("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config.get_string("rpc.user")
        .unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config.get_string("rpc.pass")
        .unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    let host_port = rpc_host.replace("http://", "").replace("https://", "");
    
    let mut stream = TcpStream::connect(&host_port)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    
    let json_body = format!(r#"{{"jsonrpc":"1.0","id":"1","method":"getbudgetvotes","params":["{}"]}}"#, proposal_name);
    let auth_b64 = base64::encode(format!("{}:{}", rpc_user, rpc_pass));
    
    let request = format!(
        "POST / HTTP/1.1\r\n\
         Host: {}\r\n\
         Authorization: Basic {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        host_port, auth_b64, json_body.len(), json_body
    );
    
    stream.write_all(request.as_bytes())?;
    
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    
    let response_str = String::from_utf8_lossy(&response);
    let pos = response_str.find("\r\n\r\n").ok_or("Invalid response")?;
    let body = &response_str[pos + 4..].trim();
    
    let value: serde_json::Value = serde_json::from_str(body)?;
    let result = value.get("result").ok_or("No result in response")?.clone();
    
    Ok(result)
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
    let config = get_global_config();
    let rpc_host = config.get_string("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config.get_string("rpc.user")
        .unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config.get_string("rpc.pass")
        .unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    let host_port = rpc_host.replace("http://", "").replace("https://", "");
    
    let mut stream = TcpStream::connect(&host_port)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    
    let json_body = r#"{"jsonrpc":"1.0","id":"1","method":"getbudgetprojection","params":[]}"#;
    let auth_b64 = base64::encode(format!("{}:{}", rpc_user, rpc_pass));
    
    let request = format!(
        "POST / HTTP/1.1\r\n\
         Host: {}\r\n\
         Authorization: Basic {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        host_port, auth_b64, json_body.len(), json_body
    );
    
    stream.write_all(request.as_bytes())?;
    
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    
    let response_str = String::from_utf8_lossy(&response);
    let pos = response_str.find("\r\n\r\n").ok_or("Invalid response")?;
    let body = &response_str[pos + 4..].trim();
    
    let value: serde_json::Value = serde_json::from_str(body)?;
    let result = value.get("result").ok_or("No result in response")?.clone();
    
    Ok(result)
}
