// Analytics API Endpoints
//
// Provides comprehensive blockchain analytics:
// - Money supply metrics
// - Transaction statistics
// - Staking analytics
// - Network health indicators
// - Rich list and wealth distribution

use axum::{Json, Extension, extract::Query, http::StatusCode};
use rocksdb::DB;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::collections::{HashMap, HashSet, BinaryHeap};
use std::cmp::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cache::CacheManager;
use crate::chain_state::get_chain_state;
use crate::parser::deserialize_transaction_blocking;
use super::helpers::format_piv_amount;
use crate::tx_type::{detect_transaction_type, TransactionType};

// ========================================
// Query Parameters
// ========================================

#[derive(Deserialize, Debug)]
pub struct TimeRangeQuery {
    #[serde(default = "default_range")]
    pub range: String,  // 24h, 7d, 30d, 90d, 1y, all
}

fn default_range() -> String {
    "30d".to_string()
}

#[derive(Deserialize, Debug)]
pub struct RichListQuery {
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    100
}

// ========================================
// Response Types
// ========================================

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SupplyAnalytics {
    pub current: SupplySnapshot,
    pub historical: Vec<SupplyDataPoint>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SupplySnapshot {
    pub total_supply: String,
    pub transparent_supply: String,
    pub shielded_supply: String,
    pub shield_adoption_percentage: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SupplyDataPoint {
    pub date: String,
    pub total: String,
    pub transparent: String,
    pub shielded: String,
}

#[derive(Serialize, Debug)]
pub struct TransactionDataPoint {
    pub date: String,
    pub count: u64,
    pub volume: String,
    pub payment_count: u64,
    pub stake_count: u64,
    /// Coinbase transactions this day. On PoS this equals the block count
    /// (one coinbase per block, paired with one coinstake). Surfaced as the
    /// "Coinbase" slice — NOT a residual "other" bucket, which would double-
    /// count the coinstakes already in `stake_count`.
    pub coinbase_count: u64,
    /// Average transaction VALUE for the day, in satoshis (string).
    /// Not byte size — this is volume / tx_count.
    pub avg_value: String,
    pub avg_fee: String,
    /// Average fee per byte across the day's Normal txs (sats/byte).
    pub avg_fee_per_byte: f64,
    /// Unique addresses appearing in any output this day.
    pub active_addresses: u64,
    /// Addresses seen for the first time ever on this day.
    pub new_addresses: u64,
    /// Transactions carrying Sapling (shield) data this day.
    pub sapling_txs: u64,
    /// Coinstakes staking a P2CS (cold-staking) delegation this day —
    /// subset of `stake_count`.
    pub coldstake_txs: u64,
    /// Coin days destroyed this day (PIV * days).
    pub coin_days_destroyed: f64,
}

#[derive(Serialize, Debug)]
pub struct StakingDataPoint {
    pub date: String,
    pub participation_rate: f64,
    pub total_staked: String,
    pub active_stakers: u64,
    pub rewards_distributed: String,
    pub avg_block_time: f64,
    /// Average size of the day's actual coinstakes (stake turnover / count), PIV.
    pub avg_stake_size: String,
    /// Honest staker APY: staker-share rewards (era emission minus masternode
    /// payment, per the PIVX Core v5.6.1 schedule) * 365 / network weight, %.
    pub apy_estimate: f64,
    /// Gross minted yield: TOTAL coinstake emission (staker + masternode
    /// share) * 365 / network weight, %. Overstates what a staker earns
    /// (~2.5x in the 10 PIV era where the staker keeps only 4 of 10).
    pub gross_yield_estimate: f64,
    /// Share of the day's blocks won by its top-10 stakers, %.
    pub top10_dominance: f64,
}

#[derive(Serialize, Debug)]
pub struct NetworkHealthDataPoint {
    pub date: String,
    pub difficulty: String,
    pub orphan_rate: f64,
    pub blocks_per_day: u64,
    pub avg_block_size: u64,
    /// 95th-percentile block interval (seconds).
    pub interval_p95_secs: u64,
    /// Longest block interval (seconds).
    pub interval_max_secs: u64,
}

#[derive(Serialize, Debug)]
pub struct RichListEntry {
    pub rank: u32,
    pub address: String,
    pub balance: String,
    pub percentage: f64,
    #[serde(rename = "txCount")]
    pub tx_count: u64,
}

#[derive(Serialize, Debug)]
pub struct WealthDistribution {
    pub top_10: f64,
    pub top_50: f64,
    pub top_100: f64,
    pub top_1000: f64,
    pub histogram: Vec<WealthBucket>,
    /// Gini coefficient over all positive balances (0 = equal, 1 = one holder).
    pub gini: f64,
    /// Minimum holders whose balances sum to >50% of the total.
    pub nakamoto_coefficient: u32,
}

#[derive(Serialize, Debug)]
pub struct WealthBucket {
    pub range: String,
    pub count: u64,
    pub percentage: f64,
}

// ========================================
// Endpoint Handlers
// ========================================

/// GET /api/v2/analytics/supply?range={timeRange}
/// Returns money supply analytics with historical data
///
/// **CACHED**: 300 second TTL (supply changes slowly). On an RPC error we serve
/// the last cached value if one exists (stale-while-revalidate) so a transient
/// node stall doesn't surface as a user-facing 500.
pub async fn supply_analytics(
    Query(params): Query<TimeRangeQuery>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<SupplyAnalytics>, StatusCode> {
    // The response shape (SupplyAnalytics) differs from the moneysupply endpoint
    // (MoneySupply) and is range-dependent, so use a dedicated key per range
    // rather than reusing "supply:latest".
    let cache_key = format!("analytics:supply:{}", params.range);
    // Separate long-lived backup so a failed recompute can fall back to the last
    // good value even after the 300s primary key has expired (get_json evicts on
    // TTL, so the primary key alone can't back stale-while-revalidate).
    let stale_key = format!("{cache_key}:last");

    // Fresh cache hit: serve immediately.
    if let Some(cached) = cache.get_json::<SupplyAnalytics>(&cache_key).await {
        return Ok(Json(cached));
    }

    match compute_supply_analytics(&db, &params.range).await {
        Ok(data) => {
            cache.set_json(&cache_key, &data, Duration::from_secs(300)).await;
            // Refresh the long-lived backup used for stale-while-revalidate.
            cache.set_json(&stale_key, &data, Duration::from_secs(86_400)).await;
            Ok(Json(data))
        }
        Err(e) => {
            // RPC stall / error: serve the last good value if we still have one.
            match cache.get_json::<SupplyAnalytics>(&stale_key).await {
                Some(stale) => {
                    tracing::warn!(error = %e, "supply analytics compute failed; serving stale cache");
                    Ok(Json(stale))
                }
                None => {
                    tracing::error!(error = %e, "supply analytics endpoint failed");
                    Err(StatusCode::INTERNAL_SERVER_ERROR)
                }
            }
        }
    }
}

/// GET /api/v2/analytics/transactions?range={timeRange}
/// Returns transaction analytics over time
pub async fn transaction_analytics(
    Query(params): Query<TimeRangeQuery>,
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<Vec<TransactionDataPoint>>, StatusCode> {
    let db_clone = db.clone();
    let range = params.range.clone();
    
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<TransactionDataPoint>, Box<dyn std::error::Error + Send + Sync>> {
        // Serve the precomputed daily series (real block-time dates); fall back
        // to the live scan only if it hasn't been built yet.
        match read_tx_daily_series(&db_clone, &range) {
            Some(series) => Ok(series),
            None => compute_transaction_analytics(&db_clone, &range),
        }
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    match result {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Read the precomputed transaction daily series from chain_state, filtered to
/// the requested range. Returns None if the series hasn't been built.
/// PIVX produces ~1440 blocks/day; a TRAILING day well below this is a partial /
/// stale-enrichment tail that renders as a cliff (counts dip) or a spike
/// (block-time = 86400/blocks shoots up). Drop such trailing days — and the
/// current day — so series end on a genuinely complete day. Interior low-block
/// days (real chain stalls) are kept. `dates` must be sorted ascending.
const MIN_COMPLETE_DAY_BLOCKS: u64 = 1000;

fn drop_incomplete_trailing_days(db: &Arc<DB>, dates: &mut Vec<String>) {
    let cf_state = match db.cf_handle("chain_state") {
        Some(c) => c,
        None => return,
    };
    // Anchor "today" to the chain TIP, not wall-clock. Under node-lag the freshest
    // data lives in the tip's calendar day (a partial day), while wall-clock today
    // may have no blocks at all; dropping wall-clock-today would then keep a stale
    // partial tip day (the spike/cliff). Fall back to wall-clock only if the tip
    // header time is unreadable.
    let tip = crate::chain_state::get_sync_height(db).unwrap_or(0);
    let today = block_time_at_height(db, tip)
        .map(|t| crate::enrich_addresses::unix_to_date(t as u64))
        .unwrap_or_else(|| {
            crate::enrich_addresses::unix_to_date(
                SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            )
        });
    dates.retain(|d| *d != today);
    while let Some(last) = dates.last() {
        let mut k = b"analytics_tx_day:".to_vec();
        k.extend_from_slice(last.as_bytes());
        let blocks = db
            .get_cf(&cf_state, &k)
            .ok()
            .flatten()
            .and_then(|b| bincode::deserialize::<crate::enrich_addresses::TxDayAgg>(&b).ok())
            .map(|a| a.blocks)
            .unwrap_or(0);
        if blocks < MIN_COMPLETE_DAY_BLOCKS {
            dates.pop();
        } else {
            break;
        }
    }
}

fn read_tx_daily_series(db: &Arc<DB>, range: &str) -> Option<Vec<TransactionDataPoint>> {
    let cf_state = db.cf_handle("chain_state")?;
    let idx_bytes = db.get_cf(&cf_state, b"analytics_tx_days").ok()??;
    let mut dates: Vec<String> = bincode::deserialize(&idx_bytes).ok()?;
    dates.sort();

    // Keep only the last `days` calendar days of the series.
    let days = parse_time_range(range) as usize;
    if dates.len() > days {
        dates = dates.split_off(dates.len() - days);
    }
    // Drop the partial current day AND any incomplete (stale-enrichment) trailing
    // days so series/headline tiles read genuinely complete days.
    drop_incomplete_trailing_days(db, &mut dates);

    let mut out = Vec::with_capacity(dates.len());
    for date in dates {
        let mut k = b"analytics_tx_day:".to_vec();
        k.extend_from_slice(date.as_bytes());
        let agg: crate::enrich_addresses::TxDayAgg = match db.get_cf(&cf_state, &k).ok()? {
            Some(b) => bincode::deserialize(&b).ok()?,
            None => continue,
        };
        // avg_value = payment volume / PAYMENT count, not total tx_count: the
        // count is dominated by coinstakes/coinbases that carry no payment
        // value, which dragged this ~26x too low. Matches the avg_fee
        // denominator below (agg.payment).
        let avg_value = if agg.payment > 0 {
            (agg.volume / agg.payment as i64).to_string()
        } else {
            "0".to_string()
        };
        out.push(TransactionDataPoint {
            date,
            count: agg.tx_count,
            volume: format_piv_amount(agg.volume),
            payment_count: agg.payment,
            stake_count: agg.coinstake,
            coinbase_count: agg.coinbase,
            avg_value,
            // Real per-payment average fee from the Pass 2b prevout joins.
            avg_fee: format_piv_amount(agg.fees_total / agg.payment.max(1) as i64),
            avg_fee_per_byte: if agg.normal_tx_bytes > 0 {
                agg.fees_total as f64 / agg.normal_tx_bytes as f64
            } else {
                0.0
            },
            active_addresses: agg.active_addresses,
            new_addresses: agg.new_addresses,
            sapling_txs: agg.sapling_txs,
            coldstake_txs: agg.coldstake_txs,
            coin_days_destroyed: agg.coin_days_destroyed,
        });
    }
    Some(out)
}

/// GET /api/v2/analytics/staking?range={timeRange}
/// Returns staking participation and rewards metrics
pub async fn staking_analytics(
    Query(params): Query<TimeRangeQuery>,
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<Vec<StakingDataPoint>>, StatusCode> {
    let db_clone = db.clone();
    let range = params.range.clone();
    
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<StakingDataPoint>, Box<dyn std::error::Error + Send + Sync>> {
        // Serve from the precomputed daily series (real dates, O(1)); fall back
        // to the legacy sampled scan only if the series hasn't been built.
        match read_staking_daily_series(&db_clone, &range) {
            Some(series) => Ok(series),
            None => compute_staking_analytics(&db_clone, &range),
        }
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    match result {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Read the precomputed staking daily series. participation_rate relates the
/// day's stake turnover (coinstake output volume) to current total supply —
/// the same approximation the legacy sampled scan used. rewards_distributed
/// comes from the Pass 2b prevout joins (coinstake outputs - inputs, budget
/// mints excluded).
fn read_staking_daily_series(db: &Arc<DB>, range: &str) -> Option<Vec<StakingDataPoint>> {
    let cf_state = db.cf_handle("chain_state")?;
    let idx_bytes = db.get_cf(&cf_state, b"analytics_tx_days").ok()??;
    let mut dates: Vec<String> = bincode::deserialize(&idx_bytes).ok()?;
    dates.sort();
    let days = parse_time_range(range) as usize;
    if dates.len() > days {
        dates = dates.split_off(dates.len() - days);
    }
    // Drop the partial current day so the staking headline tiles (active
    // stakers, APY, block time) read the last COMPLETE day, not a few hours
    // of today that read as a cliff. Also drop any incomplete trailing days.
    drop_incomplete_trailing_days(db, &mut dates);

    // Real circulating supply from the wealth snapshot (sum of all positive
    // address balances, satoshis) — calculate_total_supply_at_height() is a
    // schedule-based estimate that overshoots by an order of magnitude.
    let total_supply = db
        .get_cf(&cf_state, b"analytics_wealth")
        .ok()
        .flatten()
        .and_then(|b| bincode::deserialize::<crate::enrich_addresses::WealthSnapshot>(&b).ok())
        .map(|w| w.total_balance)
        .unwrap_or(0);

    let mut out = Vec::with_capacity(dates.len());
    for date in dates {
        let mut k = b"analytics_tx_day:".to_vec();
        k.extend_from_slice(date.as_bytes());
        let agg: crate::enrich_addresses::TxDayAgg = match db.get_cf(&cf_state, &k).ok()? {
            Some(b) => bincode::deserialize(&b).ok()?,
            None => continue,
        };
        // A day's stake_volume of 0 with nonzero coinstakes means the series was
        // built before the field existed — fall back to the legacy scan so the
        // operator sees data until the next enrichment refresh.
        if agg.coinstake > 0 && agg.stake_volume == 0 {
            return None;
        }
        // Same staleness rule for the staker-share split: rewards without a
        // staker share means the blob predates staker_rewards_total.
        if agg.rewards_total > 0 && agg.staker_rewards_total == 0 {
            return None;
        }
        let blocks = if agg.blocks > 0 { agg.blocks } else { agg.coinstake.max(1) };
        // PoS network weight derived from difficulty:
        //   staked_sats = difficulty * 2^43 / 60
        // Empirically calibrated against a measurable anchor: a staking pool
        // with a known 3.506M PIV delegated balance staked 50/300 recent blocks
        // (16.7%), implying ~21M PIV total network weight at difficulty ~14-17k,
        // which matches difficulty * 2^43/60 (the naive diff1-convention
        // estimate of difficulty * 2^32/60 is low by PIVX's kernel weight
        // scaling). Turnover (sum of coinstake outputs) badly underestimates.
        let staked_sats = agg.avg_difficulty * 8_796_093_022_208.0 / 60.0;
        let staked = staked_sats as i64;
        out.push(StakingDataPoint {
            date,
            participation_rate: if total_supply > 0 && staked > 0 {
                (staked as f64 / total_supply as f64) * 100.0
            } else {
                0.0
            },
            total_staked: format_piv_amount(staked.max(0)),
            active_stakers: agg.unique_stakers,
            // Real minted rewards from the Pass 2b prevout joins
            // (coinstake outputs - staked inputs).
            rewards_distributed: format_piv_amount(agg.rewards_total),
            avg_block_time: 86_400.0 / blocks as f64,
            avg_stake_size: format_piv_amount(agg.stake_volume / agg.coinstake.max(1) as i64),
            // Honest staker APY: only the staker's share of the emission
            // (excludes the masternode payment — and rewards_total already
            // excludes budget/superblock mints).
            apy_estimate: if staked_sats > 0.0 {
                (agg.staker_rewards_total as f64 * 365.0 / staked_sats) * 100.0
            } else {
                0.0
            },
            // Former apy_estimate: gross minted emission incl. the MN share.
            gross_yield_estimate: if staked_sats > 0.0 {
                (agg.rewards_total as f64 * 365.0 / staked_sats) * 100.0
            } else {
                0.0
            },
            top10_dominance: (agg.top10_blocks as f64 / blocks as f64) * 100.0,
        });
    }
    Some(out)
}

/// Network health from the precomputed daily series: REAL per-day difficulty
/// (averaged from header nBits) and real block counts. Orphan rate and block
/// size are not tracked per day and report 0.
fn read_network_daily_series(db: &Arc<DB>, range: &str) -> Option<Vec<NetworkHealthDataPoint>> {
    let cf_state = db.cf_handle("chain_state")?;
    let idx_bytes = db.get_cf(&cf_state, b"analytics_tx_days").ok()??;
    let mut dates: Vec<String> = bincode::deserialize(&idx_bytes).ok()?;
    dates.sort();
    let days = parse_time_range(range) as usize;
    if dates.len() > days {
        dates = dates.split_off(dates.len() - days);
    }
    // Daily network metrics are only meaningful for COMPLETE days: a partial
    // current day under-reports blocks and over-reports orphan rate (recent
    // stale-vs-canonical classification lags the live tip). Also drop incomplete
    // trailing days (stale-enrichment partial tail).
    drop_incomplete_trailing_days(db, &mut dates);

    let mut out = Vec::with_capacity(dates.len());
    for date in dates {
        let mut k = b"analytics_tx_day:".to_vec();
        k.extend_from_slice(date.as_bytes());
        let agg: crate::enrich_addresses::TxDayAgg = match db.get_cf(&cf_state, &k).ok()? {
            Some(b) => bincode::deserialize(&b).ok()?,
            None => continue,
        };
        if agg.blocks == 0 {
            // Series predates the difficulty fields — rebuild pending; fall back.
            return None;
        }
        // Real average block size: the day's transaction bytes plus per-block
        // header overhead (112-byte v8+ header + ~1 byte tx-count varint).
        let avg_block_size = (agg.tx_bytes + agg.blocks * 113) / agg.blocks;
        let orphan_rate =
            (agg.orphan_blocks as f64 / (agg.orphan_blocks + agg.blocks) as f64) * 100.0;
        out.push(NetworkHealthDataPoint {
            date,
            difficulty: format!("{:.2}", agg.avg_difficulty),
            orphan_rate,
            blocks_per_day: agg.blocks,
            avg_block_size,
            interval_p95_secs: agg.interval_p95_secs,
            interval_max_secs: agg.interval_max_secs,
        });
    }
    Some(out)
}

/// GET /api/v2/analytics/network?range={timeRange}
/// Returns network health metrics
pub async fn network_health_analytics(
    Query(params): Query<TimeRangeQuery>,
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<Vec<NetworkHealthDataPoint>>, StatusCode> {
    let db_clone = db.clone();
    let range = params.range.clone();
    
    let result = tokio::task::spawn_blocking(move || -> Result<Vec<NetworkHealthDataPoint>, Box<dyn std::error::Error + Send + Sync>> {
        match read_network_daily_series(&db_clone, &range) {
            Some(series) => Ok(series),
            None => compute_network_health_analytics(&db_clone, &range),
        }
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    match result {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// GET /api/v2/analytics/richlist?limit={limit}
/// Returns top addresses by balance
pub async fn rich_list(
    Query(params): Query<RichListQuery>,
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<Vec<RichListEntry>>, StatusCode> {
    let db_clone = db.clone();
    let limit = params.limit.clamp(1, 1000);

    let result = tokio::task::spawn_blocking(move || -> Result<Vec<RichListEntry>, Box<dyn std::error::Error + Send + Sync>> {
        // Serve the precomputed snapshot (built during enrichment): O(1),
        // correct top-N. Falls back to the live scan only if the snapshot
        // hasn't been built yet (e.g. enrichment still running).
        match read_richlist_snapshot(&db_clone, limit) {
            Some(list) => Ok(list),
            None => compute_rich_list(&db_clone, limit),
        }
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match result {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Read the precomputed rich-list snapshot from chain_state and shape it into
/// Honest "% of supply" denominator: the HODL snapshot's deduped-outpoint
/// circulating total. The wealth snapshot's total_balance is a SUM of
/// per-address balances, which double-counts cold-staked coins (credited to
/// both staker and owner for per-address parity), inflating it ~7% and biasing
/// every share low. Falls back to total_balance if the HODL blob is absent.
fn supply_denominator(db: &Arc<DB>, fallback: i64) -> f64 {
    let from_hodl = db
        .cf_handle("chain_state")
        .and_then(|cf| db.get_cf(&cf, b"analytics_hodl").ok().flatten())
        .and_then(|b| bincode::deserialize::<crate::enrich_addresses::HodlSnapshot>(&b).ok())
        .map(|h| h.total)
        .filter(|t| *t > 0);
    let v = from_hodl.unwrap_or(fallback);
    if v > 0 { v as f64 } else { 1.0 }
}

/// the API response. Percentages are relative to the total tracked balance.
fn read_richlist_snapshot(db: &Arc<DB>, limit: u32) -> Option<Vec<RichListEntry>> {
    let cf_state = db.cf_handle("chain_state")?;
    let rl_bytes = db.get_cf(&cf_state, b"analytics_richlist").ok()??;
    let wealth_bytes = db.get_cf(&cf_state, b"analytics_wealth").ok()??;
    let entries: Vec<crate::enrich_addresses::RichListSnapshotEntry> =
        bincode::deserialize(&rl_bytes).ok()?;
    let wealth: crate::enrich_addresses::WealthSnapshot =
        bincode::deserialize(&wealth_bytes).ok()?;
    let denom = supply_denominator(db, wealth.total_balance);

    Some(
        entries
            .into_iter()
            .take(limit as usize)
            .enumerate()
            .map(|(i, e)| RichListEntry {
                rank: (i + 1) as u32,
                address: e.address,
                balance: e.balance.to_string(),
                percentage: (e.balance as f64 / denom) * 100.0,
                tx_count: e.tx_count,
            })
            .collect(),
    )
}

/// GET /api/v2/analytics/wealth-distribution
/// Returns wealth distribution statistics
pub async fn wealth_distribution(
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<WealthDistribution>, StatusCode> {
    let db_clone = db.clone();

    let result = tokio::task::spawn_blocking(move || -> Result<WealthDistribution, Box<dyn std::error::Error + Send + Sync>> {
        match read_wealth_snapshot(&db_clone) {
            Some(w) => Ok(w),
            None => compute_wealth_distribution(&db_clone),
        }
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match result {
        Ok(data) => Ok(Json(data)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Read the precomputed wealth snapshot and shape it into the API response.
fn read_wealth_snapshot(db: &Arc<DB>) -> Option<WealthDistribution> {
    let cf_state = db.cf_handle("chain_state")?;
    let bytes = db.get_cf(&cf_state, b"analytics_wealth").ok()??;
    let w: crate::enrich_addresses::WealthSnapshot = bincode::deserialize(&bytes).ok()?;
    let denom = supply_denominator(db, w.total_balance);
    let pct = |v: i64| (v as f64 / denom) * 100.0;
    let total_holders = if w.address_count > 0 { w.address_count as f64 } else { 1.0 };

    Some(WealthDistribution {
        top_10: pct(w.top_10),
        top_50: pct(w.top_50),
        top_100: pct(w.top_100),
        top_1000: pct(w.top_1000),
        gini: w.gini,
        nakamoto_coefficient: w.nakamoto_coefficient,
        histogram: w
            .histogram
            .into_iter()
            .map(|(range, count)| WealthBucket {
                range,
                count,
                percentage: (count as f64 / total_holders) * 100.0,
            })
            .collect(),
    })
}

// ========================================
// New Precomputed-Blob Endpoints
// ========================================

#[derive(Deserialize, Debug)]
pub struct SnapshotHoursQuery {
    #[serde(default = "default_hours")]
    pub hours: u64,
}

fn default_hours() -> u64 {
    24
}

#[derive(Serialize, Debug)]
pub struct HodlBand {
    pub band: String,
    /// Unspent value in this age band, PIV (formatted).
    pub value: String,
    pub percentage: f64,
}

#[derive(Serialize, Debug)]
pub struct HodlResponse {
    pub bands: Vec<HodlBand>,
    pub total: String,
}

/// GET /api/v2/analytics/hodl
/// Unspent value bucketed by coin age (HODL waves). Served from the
/// precomputed `analytics_hodl` blob; empty until enrichment has run.
pub async fn hodl_analytics(
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<HodlResponse>, StatusCode> {
    let db_clone = db.clone();
    let result = tokio::task::spawn_blocking(move || read_hodl_snapshot(&db_clone))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(result.unwrap_or(HodlResponse {
        bands: Vec::new(),
        total: "0".to_string(),
    })))
}

fn read_hodl_snapshot(db: &Arc<DB>) -> Option<HodlResponse> {
    let cf_state = db.cf_handle("chain_state")?;
    let bytes = db.get_cf(&cf_state, b"analytics_hodl").ok()??;
    let snap: crate::enrich_addresses::HodlSnapshot = bincode::deserialize(&bytes).ok()?;
    let denom = if snap.total > 0 { snap.total as f64 } else { 1.0 };
    Some(HodlResponse {
        bands: snap
            .bands
            .into_iter()
            .map(|(band, sats)| HodlBand {
                band,
                value: format_piv_amount(sats),
                percentage: (sats as f64 / denom) * 100.0,
            })
            .collect(),
        total: format_piv_amount(snap.total),
    })
}

/// GET /api/v2/analytics/snapshots?hours=N
/// Hourly forward-only network snapshots (mempool, masternodes, supply)
/// collected by the block monitor. Empty until the first snapshot is written.
pub async fn snapshots_analytics(
    Query(params): Query<SnapshotHoursQuery>,
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<Vec<crate::monitor::HourlySnapshot>>, StatusCode> {
    let db_clone = db.clone();
    let hours = params.hours.clamp(1, 8760);
    let result = tokio::task::spawn_blocking(move || -> Vec<crate::monitor::HourlySnapshot> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let cutoff = now.saturating_sub(hours.saturating_mul(3600));
        let mut series: Vec<crate::monitor::HourlySnapshot> = db_clone
            .cf_handle("chain_state")
            .and_then(|cf| db_clone.get_cf(&cf, b"analytics_snapshots").ok().flatten())
            .and_then(|b| bincode::deserialize(&b).ok())
            .unwrap_or_default();
        series.retain(|s| s.ts >= cutoff);
        series
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(result))
}

#[derive(Serialize, Debug)]
pub struct TreasuryEntry {
    pub height: i32,
    pub date: String,
    /// Total superblock payout, PIV (formatted).
    pub total_paid: String,
    pub n_outputs: u32,
}

/// GET /api/v2/analytics/treasury
/// Historical budget/treasury payouts — value minted in excess of the era
/// block reward (PoW era: extra coinbase outputs; PoS era: inside coinstakes
/// at/after budget-cycle heights) — from the precomputed `analytics_treasury`
/// blob, sorted by height.
pub async fn treasury_analytics(
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<Vec<TreasuryEntry>>, StatusCode> {
    let db_clone = db.clone();
    let result = tokio::task::spawn_blocking(move || -> Vec<TreasuryEntry> {
        let payouts: Vec<crate::enrich_addresses::TreasuryPayout> = db_clone
            .cf_handle("chain_state")
            .and_then(|cf| db_clone.get_cf(&cf, b"analytics_treasury").ok().flatten())
            .and_then(|b| bincode::deserialize(&b).ok())
            .unwrap_or_default();
        payouts
            .into_iter()
            .map(|p| TreasuryEntry {
                height: p.height,
                date: p.date,
                total_paid: format_piv_amount(p.total_paid_sats),
                n_outputs: p.n_outputs,
            })
            .collect()
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(result))
}

#[derive(Serialize, Debug)]
pub struct ColdStakingDataPoint {
    pub date: String,
    /// Value newly delegated into P2CS this day, PIV (formatted).
    pub created: String,
    /// P2CS value spent (undelegated/restaked) this day, PIV (formatted).
    pub spent: String,
    /// Cumulative net delegated value since the start of the series, PIV.
    pub net_cumulative: String,
}

/// GET /api/v2/analytics/coldstaking?range={timeRange}
/// Cold-staking adoption curve from the daily P2CS created/spent aggregates.
/// The cumulative sum runs over the FULL series, then only the requested
/// range is returned.
pub async fn coldstaking_analytics(
    Query(params): Query<TimeRangeQuery>,
    Extension(db): Extension<Arc<DB>>,
) -> Result<Json<Vec<ColdStakingDataPoint>>, StatusCode> {
    let db_clone = db.clone();
    let range = params.range.clone();
    let result =
        tokio::task::spawn_blocking(move || read_coldstaking_series(&db_clone, &range))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(result.unwrap_or_default()))
}

fn read_coldstaking_series(db: &Arc<DB>, range: &str) -> Option<Vec<ColdStakingDataPoint>> {
    let cf_state = db.cf_handle("chain_state")?;
    let idx_bytes = db.get_cf(&cf_state, b"analytics_tx_days").ok()??;
    let mut dates: Vec<String> = bincode::deserialize(&idx_bytes).ok()?;
    dates.sort();
    // Drop the partial current day + any incomplete trailing days; the cumulative
    // line should end on the last genuinely complete day.
    drop_incomplete_trailing_days(db, &mut dates);
    let days = parse_time_range(range) as usize;
    let start = dates.len().saturating_sub(days);

    let mut cumulative: i64 = 0;
    let mut out = Vec::with_capacity(dates.len().min(days));
    for (i, date) in dates.iter().enumerate() {
        let mut k = b"analytics_tx_day:".to_vec();
        k.extend_from_slice(date.as_bytes());
        let agg: crate::enrich_addresses::TxDayAgg = match db.get_cf(&cf_state, &k).ok()? {
            Some(b) => bincode::deserialize(&b).ok()?,
            None => continue,
        };
        cumulative = cumulative
            .saturating_add(agg.p2cs_created)
            .saturating_sub(agg.p2cs_spent);
        if i >= start {
            out.push(ColdStakingDataPoint {
                date: date.clone(),
                created: format_piv_amount(agg.p2cs_created),
                spent: format_piv_amount(agg.p2cs_spent),
                net_cumulative: format_piv_amount(cumulative),
            });
        }
    }
    Some(out)
}

// ========================================
// Computation Functions
// ========================================

async fn compute_supply_analytics(
    db: &Arc<DB>,
    _range: &str,
) -> Result<SupplyAnalytics, Box<dyn std::error::Error + Send + Sync>> {
    // Get current chain state for total supply
    let chain_state = get_chain_state(db).map_err(|e| e.to_string())?;
    let _current_height = chain_state.height;
    
    // Get real supply data from PIVX RPC
    let money_supply = super::network::compute_money_supply().await?;
    
    // Convert from PIV (f64) to satoshis (i64) for internal calculations
    let total_supply = (money_supply.moneysupply * 100_000_000.0) as i64;
    let transparent_supply = (money_supply.transparentsupply * 100_000_000.0) as i64;
    let shielded_supply = (money_supply.shieldsupply * 100_000_000.0) as i64;
    
    // Calculate current shielded percentage for historical estimates
    let shielded_percentage = if total_supply > 0 {
        shielded_supply as f64 / total_supply as f64
    } else {
        0.0
    };
    
    let current = SupplySnapshot {
        total_supply: format_piv_amount(total_supply),
        transparent_supply: format_piv_amount(transparent_supply),
        shielded_supply: format_piv_amount(shielded_supply),
        shield_adoption_percentage: shielded_percentage * 100.0,
    };
    
    // Note: Historical supply data requires tracking during sync or multiple RPC calls
    // For now, we only return current supply snapshot
    // PIVX has a complex reward schedule that makes accurate historical estimates difficult
    // without actually having the supply data at each historical height
    let historical = Vec::new();
    
    Ok(SupplyAnalytics { current, historical })
}

fn compute_transaction_analytics(
    db: &Arc<DB>,
    range: &str,
) -> Result<Vec<TransactionDataPoint>, Box<dyn std::error::Error + Send + Sync>> {
    let chain_state = get_chain_state(db).map_err(|e| e.to_string())?;
    let current_height = chain_state.height;
    let days = parse_time_range(range) as i32;
    
    let tx_cf = db.cf_handle("transactions").ok_or("transactions CF not found")?;
    let _metadata_cf = db.cf_handle("chain_metadata").ok_or("chain_metadata CF not found")?;
    
    let blocks_per_day = 1440i32;
    let start_height = std::cmp::max(1, current_height - (days * blocks_per_day));
    
    // Group transactions by day
    let mut daily_stats: HashMap<String, DailyTxStats> = HashMap::new();
    
    // Scan all transactions and filter by height range
    let iter = db.iterator_cf(tx_cf, rocksdb::IteratorMode::Start);
    let mut processed = 0;
    
    for item in iter {
        let (key, value) = item?;
        
        // Transaction keys are 't' + txid(32 bytes)
        if key.first() != Some(&b't') || key.len() != 33 {
            continue;
        }
        
        // Transaction value format: file_number(4) + block_height(4) + tx_data
        if value.len() < 8 {
            continue;
        }
        
        let height = i32::from_le_bytes(value[4..8].try_into().unwrap_or([0,0,0,0]));
        
        // Filter by height range
        if height < start_height || height > current_height {
            continue;
        }
        
        // Calculate date from height
        let blocks_ago = current_height - height;
        let days_ago = blocks_ago / blocks_per_day;
        let seconds_ago = days_ago as u64 * 86400;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let timestamp = now - seconds_ago;
        let date_key = format_timestamp(timestamp);
        
        let stats = daily_stats.entry(date_key.clone()).or_insert(DailyTxStats {
            date: date_key,
            count: 0,
            volume: 0,
            payment_count: 0,
            stake_count: 0,
            other_count: 0,
            total_size: 0,
            total_fee: 0,
        });
        
        stats.count += 1;
        
        // Approximate: first tx in block is usually coinbase/coinstake
        if height > 1 {
            // Try to determine if it's a coinstake by checking first byte patterns
            // This is a heuristic - coinstake txs typically have different patterns
            stats.stake_count += 1; // Count blocks, not individual stake txs
        }
        
        // All non-coinbase transactions are payment transactions
        stats.payment_count += 1;
        
        processed += 1;
        
        // Limit processing for performance (sample if too many)
        if processed > 100000 {
            break;
        }
    }
    
    // Convert to data points
    let mut data_points: Vec<TransactionDataPoint> = daily_stats.into_values().map(|stats| {
            // Legacy fallback path (used only before the daily series is built).
            // total_size is not populated here, so this averages to 0; the field
            // is average transaction VALUE, not byte size.
            let avg_value = if stats.count > 0 {
                format_piv_amount((stats.total_size / stats.count) as i64)
            } else {
                "0".to_string()
            };

            let avg_fee = if stats.count > 0 {
                format_piv_amount((stats.total_fee as i64) / (stats.count as i64))
            } else {
                "0".to_string()
            };

            TransactionDataPoint {
                date: stats.date,
                count: stats.count,
                volume: format_piv_amount(stats.volume),
                payment_count: stats.payment_count,
                stake_count: stats.stake_count,
                coinbase_count: stats.other_count,
                avg_value,
                avg_fee,
                // Prevout-joined metrics only exist in the precomputed series.
                avg_fee_per_byte: 0.0,
                active_addresses: 0,
                new_addresses: 0,
                sapling_txs: 0,
                coldstake_txs: 0,
                coin_days_destroyed: 0.0,
            }
        })
        .collect();
    
    // Sort by date
    data_points.sort_by(|a, b| a.date.cmp(&b.date));
    
    Ok(data_points)
}

fn compute_staking_analytics(
    db: &Arc<DB>,
    range: &str,
) -> Result<Vec<StakingDataPoint>, Box<dyn std::error::Error + Send + Sync>> {
    let chain_state = get_chain_state(db).map_err(|e| e.to_string())?;
    let current_height = chain_state.height;
    let days = parse_time_range(range) as i32;
    
    let blocks_per_day = 1440i32;
    let tx_cf = db.cf_handle("transactions").ok_or("transactions CF not found")?;
    
    // Get current supply for participation rate calculation
    let money_supply = super::network::compute_money_supply_blocking()?;
    let total_supply = (money_supply.moneysupply * 100_000_000.0) as i64;
    
    // Sample staking data by checking recent blocks only (last 1000 blocks max)
    let sample_blocks = std::cmp::min(1000, current_height);
    let start_height = current_height - sample_blocks;
    
    let mut coinstake_count = 0u64;
    let mut unique_stakers = HashSet::new();
    let mut processed = 0;
    let max_to_process = 2000; // Limit to prevent memory issues
    
    // Iterate transactions and sample coinstakes
    let iter = db.iterator_cf(tx_cf, rocksdb::IteratorMode::Start);
    
    for item in iter {
        if processed >= max_to_process {
            break;
        }
        
        let (key, value) = item?;
        
        // Filter transaction keys (prefix 't')
        if key.first() != Some(&b't') || key.len() != 33 {
            continue;
        }
        
        // Extract height from value (bytes 4-8)
        if value.len() < 8 {
            continue;
        }
        
        let height = i32::from_le_bytes(value[4..8].try_into()?);
        
        // Only process recent blocks for sampling
        if height < start_height || height > current_height {
            continue;
        }
        
        processed += 1;
        
        // Deserialize transaction to check if it's a coinstake.
        // The deserializer expects a 4-byte block_version prefix; the stored
        // value is version(4)+height(4)+raw_tx, so strip the 8-byte header and
        // prepend a 4-byte dummy (same convention as enrich_addresses). Passing
        // value[8..] directly misaligned the parser and read a corrupt script
        // length as an allocation size, aborting the process.
        let raw_tx = &value[8..];
        let mut tx_data = Vec::with_capacity(4 + raw_tx.len());
        tx_data.extend_from_slice(&[0u8; 4]);
        tx_data.extend_from_slice(raw_tx);

        if let Ok(tx) = deserialize_transaction_blocking(&tx_data) {
            // Check if this is a coinstake transaction
            if detect_transaction_type(&tx) == TransactionType::Coinstake {
                coinstake_count += 1;
                
                // Extract staker address from coinstake outputs (usually vout[1])
                if let Some(out) = tx.outputs.get(1) {
                    if let Some(addr) = out.address.first() {
                        unique_stakers.insert(addr.clone());
                    }
                }
            }
        }
    }
    
    // Calculate daily averages from sample
    let sample_days = (sample_blocks as f64 / blocks_per_day as f64).max(1.0);
    let daily_coinstakes = (coinstake_count as f64 / sample_days) as u64;
    let active_stakers = unique_stakers.len() as u64;
    
    // Estimate staked amount: assume 10,000 PIV average per staker
    let estimated_staked = (active_stakers as i64) * 10000_00000000;
    let participation_rate = if total_supply > 0 {
        (estimated_staked as f64 / total_supply as f64) * 100.0
    } else {
        0.0
    };
    
    // Build data points using sampled statistics
    let mut data_points = Vec::new();
    
    for day in 0..=days {
        let seconds_ago = day as u64 * 86400;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let timestamp = now - seconds_ago;
        let date = format_timestamp(timestamp);
        
        // Use sampled daily average
        let rewards_distributed = (daily_coinstakes as i64) * 500000000; // 5 PIV per stake
        
        // Calculate average block time from stake count
        let avg_block_time = if daily_coinstakes > 0 {
            86400.0 / (daily_coinstakes as f64)
        } else {
            60.0 // Default to 1 minute
        };
        
        data_points.push(StakingDataPoint {
            date: date.to_string(),
            participation_rate,
            total_staked: format_piv_amount(estimated_staked),
            active_stakers,
            rewards_distributed: format_piv_amount(rewards_distributed),
            avg_block_time,
            avg_stake_size: "0".to_string(),
            apy_estimate: 0.0,
            gross_yield_estimate: 0.0,
            top10_dominance: 0.0,
        });
    }
    
    data_points.reverse();
    Ok(data_points)
}

/// Read a block's header nTime via chain_metadata (height -> display hash)
/// and the blocks CF (internal hash -> header bytes, nTime at offset 68).
fn block_time_at_height(db: &Arc<DB>, height: i32) -> Option<u32> {
    let cf_metadata = db.cf_handle("chain_metadata")?;
    let cf_blocks = db.cf_handle("blocks")?;
    let display_hash = db.get_cf(&cf_metadata, height.to_le_bytes()).ok()??;
    let internal_hash: Vec<u8> = display_hash.iter().rev().cloned().collect();
    let header = db.get_cf(&cf_blocks, &internal_hash).ok()??;
    if header.len() >= 72 {
        Some(u32::from_le_bytes(header[68..72].try_into().ok()?))
    } else {
        None
    }
}

fn compute_network_health_analytics(
    db: &Arc<DB>,
    range: &str,
) -> Result<Vec<NetworkHealthDataPoint>, Box<dyn std::error::Error + Send + Sync>> {
    let chain_state = get_chain_state(db).map_err(|e| e.to_string())?;
    let current_height = chain_state.height;
    let days = parse_time_range(range) as i32;
    
    let blocks_cf = db.cf_handle("blocks").ok_or("blocks CF not found")?;
    
    let blocks_per_day = 1440i32;
    
    let mut data_points = Vec::new();
    
    // Sample network health daily
    for day in 0..=days {
        let height = current_height - (day * blocks_per_day);
        if height < 1 {
            break;
        }
        
        // Date from the REAL block header time at this height (chain_metadata
        // height->hash, header nTime at offset 68) — the previous now-minus-
        // estimate drifted by over a year on long ranges.
        let date = block_time_at_height(db, height)
            .map(|t| crate::enrich_addresses::unix_to_date(t as u64))
            .unwrap_or_else(|| {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                format_timestamp(now - day as u64 * 86400)
            });

        // Count orphaned blocks in this day range
        let day_start = height;
        let day_end = std::cmp::min(height + blocks_per_day, current_height);
        
        let mut total_blocks = 0u64;
        let orphaned_blocks = 0u64;
        let mut total_block_size = 0u64;
        
        for h in day_start..day_end {
            total_blocks += 1;
            
            // Check if block exists and is valid
            let key = format!("H{h}");
            if (db.get_cf(blocks_cf, key.as_bytes())?).is_some() {
                // Assume average block size
                total_block_size += 10000; // ~10KB average
            }
        }
        
        let orphan_rate = if total_blocks > 0 {
            (orphaned_blocks as f64 / total_blocks as f64) * 100.0
        } else {
            0.0
        };
        
        let avg_block_size = if total_blocks > 0 {
            total_block_size / total_blocks
        } else {
            0
        };
        
        data_points.push(NetworkHealthDataPoint {
            date: date.to_string(),
            difficulty: "1250000000".to_string(), // Placeholder - would need chainwork calc
            orphan_rate,
            blocks_per_day: total_blocks,
            avg_block_size,
            interval_p95_secs: 0,
            interval_max_secs: 0,
        });
    }
    
    data_points.reverse();
    Ok(data_points)
}

// Helper struct for maintaining top N addresses with a min-heap
#[derive(Debug)]
struct AddressBalance {
    address: String,
    balance: i64,
}

impl Eq for AddressBalance {}

impl PartialEq for AddressBalance {
    fn eq(&self, other: &Self) -> bool {
        self.balance == other.balance
    }
}

impl PartialOrd for AddressBalance {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AddressBalance {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap (we want to evict smallest)
        other.balance.cmp(&self.balance)
    }
}

fn compute_rich_list(
    db: &Arc<DB>,
    limit: u32,
) -> Result<Vec<RichListEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let addr_cf = db.cf_handle("addr_index").ok_or("addr_index CF not found")?;
    
    // Use a bounded min-heap to efficiently track top N addresses
    let mut top_addresses: BinaryHeap<AddressBalance> = BinaryHeap::new();
    let limit = limit.clamp(1, 1000); // DoS guard: bound requested result size
    let max_candidates = limit.saturating_mul(2); // Scan 2x the limit for good results
    
    let mut scanned = 0;
    let max_scan = 10000; // Scan up to 10K addresses
    
    // Threshold to stop early if we have strong candidates
    let mut min_threshold = 0i64;
    
    let iter = db.iterator_cf(addr_cf, rocksdb::IteratorMode::Start);
    for item in iter {
        let (key, value) = item?;
        
        // Skip non-address UTXO keys (keys starting with 'a')
        if key.first() != Some(&b'a') {
            continue;
        }
        
        // Skip addresses with no UTXOs
        if value.is_empty() {
            continue;
        }
        
        scanned += 1;
        
        // Skip addresses with too many UTXOs (likely dust/spam)
        let utxo_count = value.len() / 40;
        if utxo_count > 200 {
            continue;
        }
        
        // Early bailout: if we have full heap and scanned enough, check if worth continuing
        if scanned > max_scan {
            break;
        }
        
        // Extract address
        let address = String::from_utf8_lossy(&key[1..]).to_string();
        
        // Calculate balance by looking up each UTXO
        let balance = calculate_address_balance(db, &value);
        
        if balance <= 0 {
            continue;
        }
        
        // Add to heap if it's in top N
        if top_addresses.len() < max_candidates as usize {
            top_addresses.push(AddressBalance { address, balance });
            // Update minimum threshold
            if let Some(min) = top_addresses.peek() {
                min_threshold = min.balance;
            }
        } else if balance > min_threshold {
            top_addresses.pop();
            top_addresses.push(AddressBalance { address, balance });
            // Update minimum threshold
            if let Some(min) = top_addresses.peek() {
                min_threshold = min.balance;
            }
        }
    }
    
    // Convert heap to sorted vector (descending by balance)
    let mut sorted_addresses: Vec<AddressBalance> = top_addresses.into_vec();
    sorted_addresses.sort_by(|a, b| b.balance.cmp(&a.balance));
    
    // Take only the requested limit
    sorted_addresses.truncate(limit as usize);
    
    // Get total supply for percentage calculations
    let chain_state = get_chain_state(db).map_err(|e| e.to_string())?;
    let total_supply = calculate_total_supply_at_height(chain_state.height);
    
    // Build final rich list entries
    let rich_list: Vec<RichListEntry> = sorted_addresses
        .into_iter()
        .enumerate()
        .map(|(i, addr_bal)| {
            let percentage = (addr_bal.balance as f64 / total_supply as f64) * 100.0;
            RichListEntry {
                rank: (i + 1) as u32,
                address: addr_bal.address,
                balance: addr_bal.balance.to_string(), // Raw satoshis
                percentage,
                tx_count: 0, // Would need to count from transaction history
            }
        })
        .collect();
    
    Ok(rich_list)
}

fn compute_wealth_distribution(
    db: &Arc<DB>,
) -> Result<WealthDistribution, Box<dyn std::error::Error + Send + Sync>> {
    let addr_cf = db.cf_handle("addr_index").ok_or("addr_index CF not found")?;
    
    // Collect balances
    let mut balances: Vec<i64> = Vec::new();
    let mut total_balance = 0i64;
    
    let iter = db.iterator_cf(addr_cf, rocksdb::IteratorMode::Start);
    for item in iter {
        let (key, value) = item?;
        
        if key.first() != Some(&b'a') {
            continue;
        }
        
        let balance = calculate_address_balance(db, &value);
        if balance > 0 {
            balances.push(balance);
            total_balance += balance;
        }
        
        if balances.len() > 10000 {
            break;
        }
    }
    
    // Sort descending
    balances.sort_by(|a, b| b.cmp(a));
    
    // Calculate top percentages
    let top_10_balance: i64 = balances.iter().take(10).sum();
    let top_50_balance: i64 = balances.iter().take(50).sum();
    let top_100_balance: i64 = balances.iter().take(100).sum();
    let top_1000_balance: i64 = balances.iter().take(1000).sum();
    
    let top_10 = (top_10_balance as f64 / total_balance as f64) * 100.0;
    let top_50 = (top_50_balance as f64 / total_balance as f64) * 100.0;
    let top_100 = (top_100_balance as f64 / total_balance as f64) * 100.0;
    let top_1000 = (top_1000_balance as f64 / total_balance as f64) * 100.0;
    
    // Create histogram
    let histogram = create_balance_histogram(&balances);

    // Gini coefficient over ascending balances (balances is sorted descending).
    let n = balances.len();
    let gini = if n > 0 && total_balance > 0 {
        let mut weighted = 0.0f64;
        for (i, b) in balances.iter().rev().enumerate() {
            weighted += (i as f64 + 1.0) * (*b as f64);
        }
        (2.0 * weighted) / (n as f64 * total_balance as f64) - (n as f64 + 1.0) / n as f64
    } else {
        0.0
    };

    // Nakamoto coefficient: minimum holders summing to >50% of total balance.
    let mut nakamoto_coefficient: u32 = 0;
    let mut acc: i64 = 0;
    for b in &balances {
        acc = acc.saturating_add(*b);
        nakamoto_coefficient += 1;
        if (acc as f64) > total_balance as f64 / 2.0 {
            break;
        }
    }

    Ok(WealthDistribution {
        top_10,
        top_50,
        top_100,
        top_1000,
        histogram,
        gini,
        nakamoto_coefficient,
    })
}

// ========================================
// Helper Functions
// ========================================

struct DailyTxStats {
    date: String,
    count: u64,
    volume: i64,
    payment_count: u64,
    stake_count: u64,
    other_count: u64,
    total_size: u64,
    total_fee: u64,
}

fn parse_time_range(range: &str) -> i64 {
    match range {
        "24h" => 1,
        "7d" => 7,
        "30d" => 30,
        "90d" => 90,
        "1y" => 365,
        "all" => 3650,
        _ => 30,
    }
}

fn calculate_total_supply_at_height(height: i32) -> i64 {
    const GENESIS_SUPPLY: i64 = 60_000_00000000;
    
    if height <= 0 {
        return GENESIS_SUPPLY;
    }
    
    let block_reward = if height < 259200 {
        500_00000000
    } else if height < 518400 {
        450_00000000
    } else if height < 777600 {
        400_00000000
    } else if height < 1036800 {
        350_00000000
    } else {
        300_00000000
    };
    
    let estimated_mined = (height as i64) * block_reward;
    GENESIS_SUPPLY + estimated_mined
}

fn calculate_address_balance(db: &Arc<DB>, utxo_data: &[u8]) -> i64 {
    if utxo_data.is_empty() {
        return 0;
    }
    
    if utxo_data.len() % 40 != 0 {
        return 0;
    }
    
    let mut balance = 0i64;
    let tx_cf = match db.cf_handle("transactions") {
        Some(cf) => cf,
        None => return 0,
    };
    
    // Limit UTXOs processed per address to 50 for performance
    let max_utxos = 50;
    let utxo_count = (utxo_data.len() / 40).min(max_utxos);
    
    for chunk in utxo_data.chunks_exact(40).take(utxo_count) {
        let txid = &chunk[0..32];
        
        let vout_bytes: [u8; 8] = match chunk[32..40].try_into() {
            Ok(b) => b,
            Err(_) => continue,
        };
        let vout = u64::from_le_bytes(vout_bytes);
        
        let mut tx_key = vec![b't'];
        tx_key.extend_from_slice(txid);
        
        let tx_data = match db.get_cf(&tx_cf, &tx_key) {
            Ok(Some(data)) => data,
            _ => continue,
        };
        
        if tx_data.len() < 8 {
            continue;
        }
        
        let tx_bytes = &tx_data[8..];
        if tx_bytes.is_empty() {
            continue;
        }
        
        let mut tx_data_with_header = Vec::with_capacity(4 + tx_bytes.len());
        tx_data_with_header.extend_from_slice(&[0u8; 4]);
        tx_data_with_header.extend_from_slice(tx_bytes);
        
        match deserialize_transaction_blocking(&tx_data_with_header) {
            Ok(tx) => {
                if let Some(output) = tx.outputs.get(vout as usize) {
                    balance += output.value;
                }
            }
            Err(_) => continue,
        }
    }
    
    balance
}

fn create_balance_histogram(balances: &[i64]) -> Vec<WealthBucket> {
    let ranges = [
        (0, 1_00000000, "0-1 PIV"),
        (1_00000000, 10_00000000, "1-10 PIV"),
        (10_00000000, 100_00000000, "10-100 PIV"),
        (100_00000000, 1000_00000000, "100-1K PIV"),
        (1000_00000000, 10000_00000000, "1K-10K PIV"),
        (10000_00000000, 100000_00000000, "10K-100K PIV"),
        (100000_00000000, i64::MAX, "100K+ PIV"),
    ];
    
    let total_count = balances.len() as f64;
    
    ranges
        .iter()
        .map(|(min, max, label)| {
            let count = balances
                .iter()
                .filter(|&&b| b >= *min && b < *max)
                .count();
            
            WealthBucket {
                range: label.to_string(),
                count: count as u64,
                percentage: (count as f64 / total_count) * 100.0,
            }
        })
        .collect()
}

fn format_timestamp(timestamp: u64) -> String {
    const SECONDS_PER_DAY: u64 = 86400;
    const DAYS_TO_EPOCH: u64 = 719162;
    
    let days_since_epoch = timestamp / SECONDS_PER_DAY;
    let total_days = DAYS_TO_EPOCH + days_since_epoch;
    
    let mut year = (total_days * 400) / 146097;
    let mut remaining_days = total_days - (year * 365 + year / 4 - year / 100 + year / 400);
    
    if remaining_days >= 365 {
        year += 1;
        remaining_days = total_days - (year * 365 + year / 4 - year / 100 + year / 400);
    }
    
    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let month_days = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    
    let mut month = 1;
    let mut day = remaining_days as u32 + 1;
    
    for (i, &days_in_month) in month_days.iter().enumerate() {
        if day <= days_in_month {
            month = i + 1;
            break;
        }
        day -= days_in_month;
    }
    
    format!("{year:04}-{month:02}-{day:02}")
}
