// Price API Endpoints
//
// Live PIVX price (USD, EUR, BTC). CoinGecko first (keyless /simple/price was
// closed ~2026-09-29; set price.coingecko_api_key to a free demo key), then
// CoinPaprika (keyless). On total failure /price serves the tickers store's
// latest day flagged stale, never zeros.

use axum::{http::StatusCode, Extension, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use super::types::BlockbookError;
use crate::cache::CacheManager;

/// Price data response format
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PriceData {
    pub usd: f64,
    pub eur: f64,
    pub btc: f64,
    pub last_updated: u64, // Unix timestamp
    // Present and true only when the live sources failed and this is the
    // latest stored daily rate.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
}

/// GET /api/v2/price
/// Returns current PIVX price in USD, EUR, and BTC
///
/// **CACHED**: 60 second TTL (CoinGecko rate limit protection)
pub async fn price_v2(
    Extension(cache): Extension<Arc<CacheManager>>,
    Extension(db): Extension<Arc<rocksdb::DB>>,
) -> Result<Json<PriceData>, (StatusCode, Json<BlockbookError>)> {
    let result = cache
        .get_or_compute("price:latest", Duration::from_secs(60), || async move {
            fetch_price().await.map_err(|e| {
                Box::new(std::io::Error::other(format!("Failed to fetch price: {e}")))
                    as Box<dyn std::error::Error + Send + Sync>
            })
        })
        .await;

    match result {
        Ok(price) => Ok(Json(price)),
        Err(e) => {
            // Zeros with a fresh timestamp read as "price is 0 right now"; the
            // stored daily rate is an honest, labeled substitute. 503 only
            // when nothing has ever been stored.
            tracing::warn!(error = %e, "live PIVX price unavailable; serving stored rate");
            let series = super::tickers::cached_series_pub(&db, &cache).await;
            match series.iter().next_back() {
                Some((ts, r)) => Ok(Json(PriceData {
                    usd: r.usd,
                    eur: r.eur,
                    btc: r.btc,
                    last_updated: *ts,
                    stale: true,
                })),
                None => Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(BlockbookError::new("Price unavailable")),
                )),
            }
        }
    }
}

/// Live price: CoinGecko, then CoinPaprika. Shared by /price and the fiat
/// sampler so one source outage cannot blank both.
pub(crate) async fn fetch_price() -> Result<PriceData, Box<dyn std::error::Error + Send + Sync>> {
    match fetch_coingecko_price().await {
        Ok(p) => Ok(p),
        Err(cg) => fetch_coinpaprika_price()
            .await
            .map_err(|pp| format!("coingecko: {cg}; coinpaprika: {pp}").into()),
    }
}

/// Free CoinGecko demo key from config, sent as x-cg-demo-api-key.
pub(crate) fn coingecko_api_key() -> Option<String> {
    crate::config::try_global_config()?
        .get_string("price.coingecko_api_key")
        .ok()
        .filter(|k| !k.trim().is_empty())
}

async fn fetch_coinpaprika_price() -> Result<PriceData, Box<dyn std::error::Error + Send + Sync>> {
    let url = "https://api.coinpaprika.com/v1/tickers/pivx-pivx?quotes=USD,EUR,BTC";
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("PIVX-Explorer/1.0")
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("CoinPaprika API returned status: {}", resp.status()).into());
    }
    let json: serde_json::Value = resp.json().await?;
    let q = |c: &str| {
        json.get("quotes")
            .and_then(|q| q.get(c))
            .and_then(|v| v.get("price"))
            .and_then(|v| v.as_f64())
            .filter(|p| *p > 0.0)
            .ok_or(format!("CoinPaprika missing {c} price"))
    };
    Ok(PriceData {
        usd: q("USD")?,
        eur: q("EUR")?,
        btc: q("BTC")?,
        last_updated: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        stale: false,
    })
}

/// Fetch price data from CoinGecko API
async fn fetch_coingecko_price() -> Result<PriceData, Box<dyn std::error::Error + Send + Sync>> {
    let url = "https://api.coingecko.com/api/v3/simple/price?ids=pivx&vs_currencies=usd,eur,btc";

    // Use reqwest for async HTTP with proper headers
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("PIVX-Explorer/1.0")
        .build()?;

    let mut req = client.get(url);
    if let Some(key) = coingecko_api_key() {
        req = req.header("x-cg-demo-api-key", key);
    }
    let response = req.send().await?;

    if !response.status().is_success() {
        return Err(format!("CoinGecko API returned status: {}", response.status()).into());
    }

    let body = response.text().await?;

    // Parse CoinGecko response format:
    // { "pivx": { "usd": 0.42, "eur": 0.39, "btc": 0.00001234 } }
    let json: serde_json::Value = serde_json::from_str(&body)?;

    let pivx_data = json
        .get("pivx")
        .ok_or("Missing 'pivx' key in CoinGecko response")?;

    let usd = pivx_data
        .get("usd")
        .and_then(|v| v.as_f64())
        .ok_or("Missing or invalid 'usd' price")?;

    let eur = pivx_data
        .get("eur")
        .and_then(|v| v.as_f64())
        .ok_or("Missing or invalid 'eur' price")?;

    let btc = pivx_data
        .get("btc")
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
        stale: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Ignore by default to avoid hitting API in CI
    async fn test_fetch_price() {
        let result = fetch_price().await;
        assert!(result.is_ok(), "Failed to fetch price: {:?}", result.err());

        let price = result.unwrap();
        assert!(price.usd > 0.0, "USD price should be positive");
        assert!(price.eur > 0.0, "EUR price should be positive");
        assert!(price.btc > 0.0, "BTC price should be positive");
    }
}
