// Fiat rate store + Blockbook tickers endpoints.
//
// Daily PIVX rates persist in chain_state as `fiatrate:<10-digit day-start ts>`
// -> JSON {"usd":..,"eur":..,"btc":..}. A background sampler self-backfills
// from CoinGecko market_chart when the store is empty (RocksDB is
// single-writer, so a separate backfill binary could never run beside the
// service), then upserts today's key from the live simple-price fetch every
// 15 minutes. Serving: /tickers, /tickers-list, balancehistory rates, and the
// websocket fiat methods all read the same store.

use axum::{extract::Query, Extension, Json};
use rocksdb::DB;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::cache::CacheManager;

const RATE_PREFIX: &[u8] = b"fiatrate:";
pub(crate) const CURRENCIES: [&str; 3] = ["btc", "eur", "usd"];

#[derive(Debug, Clone, Copy)]
pub(crate) struct DayRates {
    pub usd: f64,
    pub eur: f64,
    pub btc: f64,
}

impl DayRates {
    pub(crate) fn get(&self, cur: &str) -> Option<f64> {
        match cur {
            "usd" => Some(self.usd),
            "eur" => Some(self.eur),
            "btc" => Some(self.btc),
            _ => None,
        }
    }
    fn to_json(self, currency: Option<&str>) -> serde_json::Value {
        match currency {
            Some(c) => match self.get(c) {
                Some(v) => serde_json::json!({ c: v }),
                None => serde_json::json!({}),
            },
            None => serde_json::json!({ "usd": self.usd, "eur": self.eur, "btc": self.btc }),
        }
    }
}

fn day_start(ts: u64) -> u64 {
    ts - ts % 86_400
}

fn rate_key(day_ts: u64) -> Vec<u8> {
    format!("fiatrate:{day_ts:010}").into_bytes()
}

/// Load the whole daily series (a few KB per year; callers cache).
pub(crate) fn load_rate_series(db: &Arc<DB>) -> BTreeMap<u64, DayRates> {
    let mut out = BTreeMap::new();
    let Some(cf) = db.cf_handle("chain_state") else {
        return out;
    };
    for item in db.prefix_iterator_cf(&cf, RATE_PREFIX) {
        let Ok((k, v)) = item else { break };
        if !k.starts_with(RATE_PREFIX) {
            break;
        }
        let Some(ts) = std::str::from_utf8(&k[RATE_PREFIX.len()..])
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
        else {
            continue;
        };
        let Ok(j) = serde_json::from_slice::<serde_json::Value>(&v) else {
            continue;
        };
        let g = |c: &str| j.get(c).and_then(|x| x.as_f64()).unwrap_or(0.0);
        out.insert(
            ts,
            DayRates {
                usd: g("usd"),
                eur: g("eur"),
                btc: g("btc"),
            },
        );
    }
    out
}

/// Rate entry Blockbook-style for a requested timestamp: first at-or-after
/// the RAW timestamp (matching Blockbook's findTicker), else the latest
/// available. None only when the store is empty.
pub(crate) fn rate_at(
    series: &BTreeMap<u64, DayRates>,
    ts: Option<u64>,
) -> Option<(u64, DayRates)> {
    match ts {
        Some(t) => series
            .range(t..)
            .next()
            .or_else(|| series.iter().next_back())
            .map(|(k, v)| (*k, *v)),
        None => series.iter().next_back().map(|(k, v)| (*k, *v)),
    }
}

/// Nearest rate at-or-before, for balance-history buckets (a historical bucket
/// must never get a FUTURE day's price).
pub(crate) fn rate_at_or_before(
    series: &BTreeMap<u64, DayRates>,
    ts: u64,
) -> Option<(u64, DayRates)> {
    series
        .range(..=day_start(ts))
        .next_back()
        .map(|(k, v)| (*k, *v))
}

fn write_day(db: &Arc<DB>, day_ts: u64, rates: DayRates) -> Result<(), String> {
    let cf = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    let body = serde_json::json!({ "usd": rates.usd, "eur": rates.eur, "btc": rates.btc });
    db.put_cf(&cf, rate_key(day_ts), body.to_string().as_bytes())
        .map_err(|e| e.to_string())
}

/// One market_chart call: daily [ts, price] pairs for a currency.
async fn fetch_history(currency: &str) -> Result<Vec<(u64, f64)>, String> {
    let url = format!(
        "https://api.coingecko.com/api/v3/coins/pivx/market_chart?vs_currency={currency}&days=max&interval=daily"
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("PIVX-Explorer/1.0")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("market_chart {currency}: HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let prices = body
        .get("prices")
        .and_then(|p| p.as_array())
        .ok_or("market_chart: no prices array")?;
    Ok(prices
        .iter()
        .filter_map(|pair| {
            let arr = pair.as_array()?;
            let ms = arr.first()?.as_f64()?;
            let price = arr.get(1)?.as_f64()?;
            Some((day_start((ms / 1000.0) as u64), price))
        })
        .collect())
}

/// Full-history backfill, one call per currency, merged by day.
async fn backfill(db: &Arc<DB>) -> Result<usize, String> {
    let mut merged: BTreeMap<u64, DayRates> = BTreeMap::new();
    for cur in CURRENCIES {
        for (day, price) in fetch_history(cur).await? {
            let e = merged.entry(day).or_insert(DayRates {
                usd: 0.0,
                eur: 0.0,
                btc: 0.0,
            });
            match cur {
                "usd" => e.usd = price,
                "eur" => e.eur = price,
                "btc" => e.btc = price,
                _ => {}
            }
        }
        // CoinGecko free tier is unhappy about burst calls.
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    // Never overwrite a live-sampled day: the sampler's simple-price points
    // beat market_chart's coarser series.
    let db2 = Arc::clone(db);
    tokio::task::spawn_blocking(move || -> Result<usize, String> {
        let existing = load_rate_series(&db2);
        let mut n = 0usize;
        for (day, rates) in merged {
            if existing.contains_key(&day) {
                continue;
            }
            write_day(&db2, day, rates)?;
            n += 1;
        }
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Background task: self-backfill once when the store is (near) empty, then
/// upsert today's rates every 15 minutes from the live price fetch.
pub async fn run_fiat_rate_sampler(db: Arc<DB>) {
    // Config gate: one instance per site should sample (a testnet twin polling
    // CoinGecko doubles the rate-limit load for identical data).
    let enabled = crate::config::get_global_config()
        .get_bool("price.fiat_sampler")
        .unwrap_or(true);
    if !enabled {
        info!("fiat sampler disabled (price.fiat_sampler = false)");
        return;
    }
    // Mainnet only: CoinGecko rates describe mainnet PIVX; a testnet twin
    // sampling them writes irrelevant data and doubles the API quota use.
    // Fail CLOSED: no sampling until the node confirms the chain (an RPC-down
    // boot on a testnet box must not start a mainnet backfill).
    loop {
        match crate::api::helpers::rpc_call_json("getblockchaininfo", serde_json::json!([])).await {
            Ok(v) => {
                let chain = v.get("chain").and_then(|c| c.as_str()).unwrap_or("");
                if chain == "main" {
                    break;
                }
                info!(chain = %chain, "fiat sampler disabled on non-mainnet chain");
                return;
            }
            Err(e) => {
                warn!(error = %e, "fiat sampler: chain unknown (RPC down); retrying in 60s");
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }
    }
    if load_rate_series(&db).len() < 30 {
        info!("fiat sampler: store empty, backfilling from CoinGecko market_chart");
        match backfill(&db).await {
            Ok(n) => info!(days = n, "fiat sampler: backfill complete"),
            Err(e) => warn!(error = %e, "fiat sampler: backfill failed; will accumulate live"),
        }
    }
    let mut tick = tokio::time::interval(Duration::from_secs(900));
    loop {
        tick.tick().await;
        match super::price::fetch_coingecko_price().await {
            Ok(p) => {
                let day = day_start(p.last_updated);
                let rates = DayRates {
                    usd: p.usd,
                    eur: p.eur,
                    btc: p.btc,
                };
                let db2 = Arc::clone(&db);
                let write = tokio::task::spawn_blocking(move || write_day(&db2, day, rates))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                if let Err(e) = write {
                    warn!(error = %e, "fiat sampler: write failed");
                }
            }
            Err(e) => warn!(error = %e, "fiat sampler: live fetch failed"),
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct TickersQuery {
    pub currency: Option<String>,
    pub timestamp: Option<u64>,
}

/// Cached series load (60s): the store only changes a few times a day.
async fn cached_series(db: &Arc<DB>, cache: &Arc<CacheManager>) -> BTreeMap<u64, DayRates> {
    let db = Arc::clone(db);
    cache
        .get_or_compute("tickers:series", Duration::from_secs(60), || async move {
            let flat: Vec<(u64, f64, f64, f64)> =
                tokio::task::spawn_blocking(move || load_rate_series(&db))
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(t, r)| (t, r.usd, r.eur, r.btc))
                    .collect();
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(flat)
        })
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(t, usd, eur, btc)| (t, DayRates { usd, eur, btc }))
        .collect()
}

/// Shared cached-series accessor for other modules (balance history, ws).
pub(crate) async fn cached_series_pub(
    db: &Arc<DB>,
    cache: &Arc<CacheManager>,
) -> BTreeMap<u64, DayRates> {
    cached_series(db, cache).await
}

/// GET /api/v2/tickers?currency=&timestamp=
pub async fn tickers_v2(
    Query(q): Query<TickersQuery>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<super::types::BlockbookError>)> {
    let series = cached_series(&db, &cache).await;
    let currency = q.currency.as_deref().map(str::to_lowercase);
    match rate_at(&series, q.timestamp) {
        Some((ts, rates)) => Ok(Json(serde_json::json!({
            "ts": ts,
            "rates": rates.to_json(currency.as_deref()),
        }))),
        None => Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(super::types::BlockbookError::new("No tickers available")),
        )),
    }
}

/// GET /api/v2/tickers-list?timestamp=
pub async fn tickers_list_v2(
    Query(q): Query<TickersQuery>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<super::types::BlockbookError>)> {
    let series = cached_series(&db, &cache).await;
    match rate_at(&series, q.timestamp) {
        Some((ts, _)) => Ok(Json(serde_json::json!({
            "ts": ts,
            "available_currencies": CURRENCIES,
        }))),
        None => Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(super::types::BlockbookError::new("No tickers available")),
        )),
    }
}
