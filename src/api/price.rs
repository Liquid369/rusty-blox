// Price API Endpoints
//
// Proxies CoinGecko API for PIVX price data with caching to avoid rate limits.
// Provides multi-currency support (USD, EUR, BTC).

use axum::{Json, Extension, http::StatusCode};
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use std::time::Duration;

use crate::cache::CacheManager;
use super::types::BlockbookError;

/// Price data response format
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PriceData {
    pub usd: f64,
    pub eur: f64,
    pub btc: f64,
    pub last_updated: u64, // Unix timestamp
}

/// GET /api/v2/price
/// Returns current PIVX price in USD, EUR, and BTC
/// 
/// **CACHED**: 60 second TTL (CoinGecko rate limit protection)
pub async fn price_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<PriceData>, (StatusCode, Json<BlockbookError>)> {
    let result = cache
        .get_or_compute(
            "price:latest",
            Duration::from_secs(60),
            || async move {
                fetch_coingecko_price()
                    .await
                    .map_err(|e| Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Failed to fetch price: {}", e)
                    )) as Box<dyn std::error::Error + Send + Sync>)
            }
        )
        .await;
    
    match result {
        Ok(price) => Ok(Json(price)),
        Err(e) => {
            // Return last known price or zero values if fetch fails
            tracing::warn!(error = %e, "Failed to fetch PIVX price, returning fallback");
            Ok(Json(PriceData {
                usd: 0.0,
                eur: 0.0,
                btc: 0.0,
                last_updated: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            }))
        }
    }
}

/// Fetch price data from CoinGecko API
async fn fetch_coingecko_price() -> Result<PriceData, Box<dyn std::error::Error + Send + Sync>> {
    let url = "https://api.coingecko.com/api/v3/simple/price?ids=pivx&vs_currencies=usd,eur,btc";
    
    // Use reqwest for async HTTP with proper headers
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("PIVX-Explorer/1.0")
        .build()?;
    
    let response = client.get(url).send().await?;
    
    if !response.status().is_success() {
        return Err(format!("CoinGecko API returned status: {}", response.status()).into());
    }
    
    let body = response.text().await?;
    
    // Parse CoinGecko response format:
    // { "pivx": { "usd": 0.42, "eur": 0.39, "btc": 0.00001234 } }
    let json: serde_json::Value = serde_json::from_str(&body)?;
    
    let pivx_data = json.get("pivx")
        .ok_or("Missing 'pivx' key in CoinGecko response")?;
    
    let usd = pivx_data.get("usd")
        .and_then(|v| v.as_f64())
        .ok_or("Missing or invalid 'usd' price")?;
    
    let eur = pivx_data.get("eur")
        .and_then(|v| v.as_f64())
        .ok_or("Missing or invalid 'eur' price")?;
    
    let btc = pivx_data.get("btc")
        .and_then(|v| v.as_f64())
        .ok_or("Missing or invalid 'btc' price")?;
    
    let last_updated = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    tracing::debug!(
        usd = %usd,
        eur = %eur,
        btc = %btc,
        "Fetched PIVX price from CoinGecko"
    );
    
    Ok(PriceData {
        usd,
        eur,
        btc,
        last_updated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    #[ignore] // Ignore by default to avoid hitting API in CI
    async fn test_fetch_coingecko_price() {
        let result = fetch_coingecko_price().await;
        assert!(result.is_ok(), "Failed to fetch price: {:?}", result.err());
        
        let price = result.unwrap();
        assert!(price.usd > 0.0, "USD price should be positive");
        assert!(price.eur > 0.0, "EUR price should be positive");
        assert!(price.btc > 0.0, "BTC price should be positive");
    }
}
