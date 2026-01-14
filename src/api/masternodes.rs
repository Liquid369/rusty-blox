// Masternode API Endpoints
//
// Endpoints that proxy to PIVX RPC for masternode information.

use axum::{Json, Extension, extract::Path as AxumPath, http::StatusCode};
use pivx_rpc_rs::{PivxRpcClient, MasternodeList};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use crate::cache::CacheManager;
use crate::config::get_global_config;
use super::types::MNCount;

/// GET /api/v2/mncount
/// Returns masternode count statistics from RPC.
/// 
/// **CACHED**: 60 second TTL
pub async fn mn_count_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<MNCount>, StatusCode> {
    let result = cache
        .get_or_compute(
            "mn:count",
            Duration::from_secs(60),
            || async {
                compute_mn_count().await
            }
        )
        .await;
    
    match result {
        Ok(count) => Ok(Json(count)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn compute_mn_count() -> Result<MNCount, Box<dyn std::error::Error + Send + Sync>> {
    let config = get_global_config();
    let rpc_host = config.get_string("rpc.host")
        .unwrap_or_else(|_| "http://127.0.0.1:51472".to_string());
    let rpc_user = config.get_string("rpc.user")
        .unwrap_or_else(|_| "explorer".to_string());
    let rpc_pass = config.get_string("rpc.pass")
        .unwrap_or_else(|_| "explorer_test_pass".to_string());
    
    let host_port = rpc_host.replace("http://", "").replace("https://", "");
    
    let mut stream = TcpStream::connect(&host_port)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    
    let json_body = r#"{"jsonrpc":"1.0","id":"1","method":"getmasternodecount","params":[]}"#;
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

/// GET /api/v2/mnlist
/// Returns full masternode list from RPC.
/// 
/// **CACHED**: 60 second TTL
pub async fn mn_list_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<Vec<MasternodeList>>, StatusCode> {
    let result = cache
        .get_or_compute(
            "mn:list",
            Duration::from_secs(60),
            || async {
                compute_mn_list().await
            }
        )
        .await;
    
    match result {
        Ok(list) => Ok(Json(list)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn compute_mn_list() -> Result<Vec<MasternodeList>, Box<dyn std::error::Error + Send + Sync>> {
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
    
    let json_body = r#"{"jsonrpc":"1.0","id":"1","method":"listmasternodes","params":[]}"#;
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

/// GET /api/v2/relaymnb/{hex}
/// Relay masternode broadcast message.
/// 
/// **NO CACHE**: Write operation
pub async fn relay_mnb_v2(
    AxumPath(param): AxumPath<String>
) -> Result<Json<String>, StatusCode> {
    let config = get_global_config();
    let rpc_host = config.get::<String>("rpc.host");
    let rpc_user = config.get::<String>("rpc.user");
    let rpc_pass = config.get::<String>("rpc.pass");

    let client = PivxRpcClient::new(
        rpc_host.unwrap_or_else(|_| "127.0.0.1:51472".to_string()),
        Some(rpc_user.unwrap_or_default()),
        Some(rpc_pass.unwrap_or_default()),
        3, 10, 1000,
    );

    match client.relaymasternodebroadcast(&param) {
        Ok(mnb_relay) => Ok(Json(mnb_relay)),
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}
