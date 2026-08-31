// Balance-history API endpoint
//
// GET /api/v2/balancehistory/{address}?groupBy=&from=&to=
// Blockbook-compatible per-bucket flow series ({time, txs, received, sent,
// sentToSelf}, sats as STRINGS) reconstructed from the address's own 't' tx
// list. Every input that spends this address's coin references a prevout tx
// that itself credited the address, so that tx is in the same 't' list by
// construction, and the whole series costs ONE body read per tx with no
// external prevout lookups. Attribution matches enrichment exactly (every
// address in an output's decoded vector, P2CS credits/debits both parties),
// so sum(received - sent) always lands on the /address balance (r - s).

use axum::{extract::Query, Extension, Json};
use rocksdb::DB;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use crate::cache::CacheManager;
use crate::types::CTransaction;

pub use axum::extract::Path as AxumPath;

/// Zerocoin-era caveat (accepted): a zerocoin spend of this address's coin has
/// no resolvable prevout, so it never debits; the SAME blind spot the lifetime
/// r/s totals have, which is what keeps the series consistent with BALANCE.
const TOO_MANY_TXS: &str = "Address has too many transactions for a balance history";

fn max_txs_cap() -> usize {
    std::env::var("RUSTYBLOX_BALANCE_HISTORY_MAX_TXS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(250_000)
}

#[derive(Deserialize, Debug, Clone)]
pub struct BalanceHistoryQuery {
    #[serde(rename = "groupBy")]
    pub group_by: Option<u64>,
    pub from: Option<u64>,
    pub to: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BalanceHistoryBucket {
    /// Bucket start (unix secs, floor(block_time / groupBy) * groupBy).
    pub time: u64,
    pub txs: u32,
    /// Satoshi STRINGS (house rule for /address-family money).
    pub received: String,
    pub sent: String,
    #[serde(rename = "sentToSelf")]
    pub sent_to_self: String,
}

/// GET /api/v2/balancehistory/{address}
///
/// **CACHED**: 300 second TTL, keyed on the full query (single-flight, so a
/// herd on a whale address computes once).
pub async fn balance_history_v2(
    AxumPath(address): AxumPath<String>,
    Query(params): Query<BalanceHistoryQuery>,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<
    Json<Vec<BalanceHistoryBucket>>,
    (axum::http::StatusCode, Json<super::types::BlockbookError>),
> {
    if !super::addresses::is_valid_address(&address) {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(super::types::BlockbookError::new(format!(
                "Invalid address '{address}': checksum mismatch"
            ))),
        ));
    }
    // Same reindex gate as /address: a mid-rebuild 't' list is not servable.
    if !crate::chain_state::addr_index_ready(&db) {
        return Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(super::types::BlockbookError::new(
                "Address index is reindexing; please retry shortly",
            )),
        ));
    }

    let group_by = params.group_by.unwrap_or(3600).clamp(60, 2_592_000);
    let cache_key = format!(
        "balhist:{address}:{group_by}:{:?}:{:?}",
        params.from, params.to
    );
    let db_clone = Arc::clone(&db);
    let address_clone = address.clone();
    let (from, to) = (params.from, params.to);

    let result = cache
        .get_or_compute(&cache_key, Duration::from_secs(300), || async move {
            compute_balance_history(&db_clone, &address_clone, group_by, from, to).await
        })
        .await;

    match result {
        Ok(series) => Ok(Json(series)),
        Err(e) => {
            let msg = e.to_string();
            if msg == TOO_MANY_TXS {
                return Err((
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    Json(super::types::BlockbookError::new(msg)),
                ));
            }
            // Fail LOUD in logs, generic to clients (no internal-detail leak). A
            // partial/zeroed series here would be a confident false money statement.
            warn!(address = %address, error = %e, "balance history compute failed");
            Err((
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(super::types::BlockbookError::new(
                    "Internal error computing balance history; please retry",
                )),
            ))
        }
    }
}

/// One tx's flow for the target address: block height, satoshis credited, and
/// the prevout refs (display-hex txid, vout index) to resolve debits against
/// the address's OWN output map in the second pass.
struct TxFlow {
    height: i32,
    credit: i64,
    refs: Vec<(String, u64)>,
}

/// Extract the address's credit + prevout refs from one parsed tx, recording
/// its own outputs into `own` for the debit pass. Pure logic, unit-tested.
fn tx_flow(
    tx: &CTransaction,
    txid_hex: &str,
    address: &str,
    own: &mut HashMap<(String, u64), i64>,
) -> (i64, Vec<(String, u64)>) {
    let mut credit = 0i64;
    for o in &tx.outputs {
        if o.address.iter().any(|a| a == address) {
            credit = credit.saturating_add(o.value);
            own.insert((txid_hex.to_string(), o.index), o.value);
        }
    }
    let refs = tx
        .inputs
        .iter()
        .filter(|i| i.coinbase.is_none())
        .filter_map(|i| i.prevout.as_ref().map(|p| (p.hash.clone(), p.n as u64)))
        .collect();
    (credit, refs)
}

/// Bucket the per-tx flows by block time. `resolve_time` maps a height to its
/// header nTime; an unresolvable CANONICAL height is index corruption and
/// fails the whole series (a silently dropped tx would desync the curve from
/// the address balance). Pure logic, unit-tested with a closure.
fn bucket_series(
    flows: &[TxFlow],
    own: &HashMap<(String, u64), i64>,
    mut resolve_time: impl FnMut(i32) -> Option<u32>,
    group_by: u64,
    from: Option<u64>,
    to: Option<u64>,
) -> Result<Vec<BalanceHistoryBucket>, String> {
    #[derive(Default)]
    struct Agg {
        txs: u32,
        received: i64,
        sent: i64,
        sent_to_self: i64,
    }
    let mut time_cache: HashMap<i32, Option<u32>> = HashMap::new();
    let mut buckets: BTreeMap<u64, Agg> = BTreeMap::new();

    for f in flows {
        let debit: i64 = f.refs.iter().filter_map(|r| own.get(r)).sum();
        let t = *time_cache
            .entry(f.height)
            .or_insert_with(|| resolve_time(f.height));
        let Some(t) = t else {
            return Err(format!("no header time for canonical height {}", f.height));
        };
        let start = (t as u64 / group_by) * group_by;
        let agg = buckets.entry(start).or_default();
        agg.txs += 1;
        agg.received = agg.received.saturating_add(f.credit);
        agg.sent = agg.sent.saturating_add(debit);
        if debit > 0 {
            // Blockbook semantics: value this address paid back to itself
            // (change) in txs where it was a sender.
            agg.sent_to_self = agg.sent_to_self.saturating_add(f.credit);
        }
    }

    Ok(buckets
        .into_iter()
        .filter(|(t, _)| from.is_none_or(|f| *t >= f) && to.is_none_or(|hi| *t <= hi))
        .map(|(time, a)| BalanceHistoryBucket {
            time,
            txs: a.txs,
            received: a.received.to_string(),
            sent: a.sent.to_string(),
            sent_to_self: a.sent_to_self.to_string(),
        })
        .collect())
}

pub(crate) async fn compute_balance_history(
    db: &Arc<DB>,
    address: &str,
    group_by: u64,
    from: Option<u64>,
    to: Option<u64>,
) -> Result<Vec<BalanceHistoryBucket>, Box<dyn std::error::Error + Send + Sync>> {
    let db = Arc::clone(db);
    let address = address.to_string();
    tokio::task::spawn_blocking(move || -> Result<Vec<BalanceHistoryBucket>, String> {
        let cf_addr = db
            .cf_handle("addr_index")
            .ok_or("addr_index CF not found")?;
        let cf_tx = db
            .cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        let cf_meta = db
            .cf_handle("chain_metadata")
            .ok_or("chain_metadata CF not found")?;
        let cf_blocks = db.cf_handle("blocks").ok_or("blocks CF not found")?;

        let mut key = vec![b't'];
        key.extend_from_slice(address.as_bytes());
        let Some(list) = db.get_cf(&cf_addr, &key).map_err(|e| e.to_string())? else {
            return Ok(vec![]); // no history, a valid empty account
        };
        // Parser is async-only; we're on a blocking thread (same pattern as
        // deserialize_transaction_blocking).
        let entries = futures::executor::block_on(crate::parser::deserialize_addr_txs(&list))
            .map_err(|e| e.to_string())?;
        // Skip non-canonical entries: orphan (-1) / unresolved (-2) sentinels
        // and any mempool 0; the curve is CONFIRMED history.
        let canonical: Vec<&(Vec<u8>, i32)> = entries.iter().filter(|(_, h)| *h > 0).collect();
        if canonical.len() > max_txs_cap() {
            return Err(TOO_MANY_TXS.to_string());
        }

        // Batch ALL tx-record reads into one multi_get (both key orders per txid;
        // same semantics as read_tx_record_orphan_aware: body = first len>8 probing
        // internal then display, orphan if EITHER order carries HEIGHT_ORPHAN).
        // Serial point-gets cost a cold 17k-tx staker ~30s on the VPS; one batched
        // multi_get is 1-2 orders of magnitude cheaper on the same data.
        let tx_keys: Vec<(&rocksdb::ColumnFamily, Vec<u8>)> = canonical
            .iter()
            .flat_map(|(txid, _)| {
                let internal: Vec<u8> = txid.iter().rev().cloned().collect();
                let mut ik = vec![b't'];
                ik.extend_from_slice(&internal);
                let mut dk = vec![b't'];
                dk.extend_from_slice(txid);
                [(cf_tx, ik), (cf_tx, dk)]
            })
            .collect();
        let tx_vals = db.multi_get_cf(tx_keys);

        let mut own: HashMap<(String, u64), i64> = HashMap::new();
        let mut flows: Vec<TxFlow> = Vec::with_capacity(canonical.len());
        for (i, (txid_bytes, height)) in canonical.iter().enumerate() {
            let pair = [&tx_vals[2 * i], &tx_vals[2 * i + 1]];
            let mut body: Option<&Vec<u8>> = None;
            let mut orphan_marked = false;
            for v in pair {
                let d = match v {
                    Ok(Some(d)) => d,
                    Ok(None) => continue,
                    // Orphan-aware on a SUMMING path, and IO errors must stay
                    // errors: reading a failure as "absent" would drop money.
                    Err(e) => return Err(e.to_string()),
                };
                if d.len() >= 8
                    && i32::from_le_bytes([d[4], d[5], d[6], d[7]])
                        == crate::constants::HEIGHT_ORPHAN
                {
                    orphan_marked = true;
                }
                if d.len() > 8 && body.is_none() {
                    body = Some(d);
                }
            }
            if orphan_marked {
                continue;
            }
            let Some(rec) = body else { continue };
            // Record = version(4) ++ height(4) ++ raw_tx; the parser wants a
            // 4-byte block_version prefix (same framing as /tx).
            let mut framed = Vec::with_capacity(4 + rec.len() - 8);
            framed.extend_from_slice(&[0u8; 4]);
            framed.extend_from_slice(&rec[8..]);
            let tx = crate::parser::deserialize_transaction_blocking(&framed)
                .map_err(|e| format!("tx parse failed for {}: {e}", hex::encode(txid_bytes)))?;
            let txid_hex = hex::encode(txid_bytes);
            let (credit, refs) = tx_flow(&tx, &txid_hex, &address, &mut own);
            flows.push(TxFlow {
                height: *height,
                credit,
                refs,
            });
        }

        // Batch the header-time resolution the same way: distinct heights ->
        // chain_metadata (height -> display hash) -> blocks CF (internal hash ->
        // header, nTime at [68..72]). A per-block staker has one distinct height
        // per tx, which made the closure-per-height path the other half of the 30s.
        let mut heights: Vec<i32> = flows.iter().map(|f| f.height).collect();
        heights.sort_unstable();
        heights.dedup();
        let hash_keys: Vec<(&rocksdb::ColumnFamily, Vec<u8>)> = heights
            .iter()
            .map(|h| (cf_meta, h.to_le_bytes().to_vec()))
            .collect();
        let hashes = db.multi_get_cf(hash_keys);
        let mut header_keys: Vec<(&rocksdb::ColumnFamily, Vec<u8>)> =
            Vec::with_capacity(heights.len());
        for v in &hashes {
            match v {
                Ok(Some(display)) => {
                    header_keys.push((cf_blocks, display.iter().rev().cloned().collect()))
                }
                // Missing canonical height = index corruption; fail loud rather
                // than desync the curve. Keep index alignment with a dummy key.
                Ok(None) => header_keys.push((cf_blocks, vec![])),
                Err(e) => return Err(e.to_string()),
            }
        }
        let headers = db.multi_get_cf(header_keys);
        let mut time_by_height: HashMap<i32, u32> = HashMap::with_capacity(heights.len());
        for (i, h) in heights.iter().enumerate() {
            if let Ok(Some(hdr)) = &headers[i] {
                if hdr.len() >= 72 {
                    if let Ok(b) = <[u8; 4]>::try_from(&hdr[68..72]) {
                        time_by_height.insert(*h, u32::from_le_bytes(b));
                    }
                }
            }
        }

        bucket_series(
            &flows,
            &own,
            |h| time_by_height.get(&h).copied(),
            group_by,
            from,
            to,
        )
    })
    .await
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
    .map_err(|e| e.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{COutPoint, CScript, CTxIn, CTxOut};

    fn out(value: i64, index: u64, addrs: &[&str]) -> CTxOut {
        CTxOut {
            value,
            script_length: 0,
            script_pubkey: CScript { script: vec![] },
            index,
            address: addrs.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn spend(prev_hex: &str, n: u32) -> CTxIn {
        CTxIn {
            prevout: Some(COutPoint {
                hash: prev_hex.to_string(),
                n,
            }),
            script_sig: CScript { script: vec![] },
            sequence: 0,
            index: 0,
            coinbase: None,
        }
    }
    fn tx(inputs: Vec<CTxIn>, outputs: Vec<CTxOut>) -> CTransaction {
        CTransaction {
            txid: String::new(),
            version: 1,
            tx_type: 0,
            inputs,
            outputs,
            lock_time: 0,
            sapling_data: None,
            extra_payload: None,
        }
    }

    // Credit, self-spend change (sentToSelf), P2CS both-parties attribution, and
    // the cumulative == sum(received - sent) identity that anchors the curve to the
    // address balance.
    #[test]
    fn flows_and_buckets_reconstruct_the_balance() {
        const A: &str = "DAddrAAAA";
        let mut own = HashMap::new();

        // tx1: A receives 100 (plus an unrelated output that must not count)
        let t1 = tx(vec![], vec![out(100, 0, &[A]), out(7, 1, &["DOther"])]);
        let (c1, r1) = tx_flow(&t1, "t1", A, &mut own);
        assert_eq!((c1, r1.len()), (100, 0));

        // tx2: A receives 50 as the OWNER leg of a P2CS output [staker, owner]
        let t2 = tx(vec![], vec![out(50, 0, &["SStaker", A])]);
        let (c2, _) = tx_flow(&t2, "t2", A, &mut own);
        assert_eq!(c2, 50, "P2CS output credits every listed address");

        // tx3: A spends the 100, pays 60 away, takes 39 change (fee 1)
        let t3 = tx(
            vec![spend("t1", 0)],
            vec![out(60, 0, &["DOther"]), out(39, 1, &[A])],
        );
        let (c3, r3) = tx_flow(&t3, "t3", A, &mut own);
        assert_eq!((c3, r3.len()), (39, 1));

        let flows = vec![
            TxFlow {
                height: 10,
                credit: c1,
                refs: vec![],
            },
            TxFlow {
                height: 20,
                credit: c2,
                refs: vec![],
            },
            TxFlow {
                height: 30,
                credit: c3,
                refs: r3,
            },
        ];
        // heights 10/20 land in bucket 0, height 30 in bucket 3600
        let series = bucket_series(
            &flows,
            &own,
            |h| Some(if h < 30 { 100 } else { 3700 }),
            3600,
            None,
            None,
        )
        .unwrap();
        assert_eq!(series.len(), 2);
        assert_eq!((series[0].time, series[0].txs), (0, 2));
        assert_eq!(series[0].received, "150");
        assert_eq!(series[0].sent, "0");
        assert_eq!(series[0].sent_to_self, "0");
        assert_eq!((series[1].time, series[1].txs), (3600, 1));
        assert_eq!(series[1].received, "39");
        assert_eq!(series[1].sent, "100", "debit = the full spent prevout");
        assert_eq!(series[1].sent_to_self, "39", "change back to self");

        // The anchor identity: sum(received - sent) == final balance (100+50-100+39 = 89)
        let net: i64 = series
            .iter()
            .map(|b| b.received.parse::<i64>().unwrap() - b.sent.parse::<i64>().unwrap())
            .sum();
        assert_eq!(net, 89);
    }

    // An unresolvable canonical height must FAIL the series, never silently
    // drop a tx (that would desync the curve from the balance).
    #[test]
    fn unresolvable_height_fails_loud() {
        let own = HashMap::new();
        let flows = vec![TxFlow {
            height: 42,
            credit: 1,
            refs: vec![],
        }];
        assert!(bucket_series(&flows, &own, |_| None, 3600, None, None).is_err());
    }

    // from/to filter on bucket starts; coinbase inputs contribute no refs.
    #[test]
    fn from_to_filter_and_coinbase_inputs() {
        const A: &str = "DAddrAAAA";
        let mut own = HashMap::new();
        let cb = tx(
            vec![CTxIn {
                prevout: None,
                script_sig: CScript { script: vec![] },
                sequence: 0,
                index: 0,
                coinbase: Some(vec![1, 2]),
            }],
            vec![out(5, 0, &[A])],
        );
        let (c, r) = tx_flow(&cb, "cb", A, &mut own);
        assert_eq!((c, r.len()), (5, 0));

        let flows = vec![
            TxFlow {
                height: 1,
                credit: 5,
                refs: vec![],
            },
            TxFlow {
                height: 2,
                credit: 5,
                refs: vec![],
            },
        ];
        let series = bucket_series(
            &flows,
            &own,
            |h| Some(if h == 1 { 100 } else { 7300 }),
            3600,
            Some(3600),
            None,
        )
        .unwrap();
        assert_eq!(series.len(), 1, "from=3600 drops the first bucket");
        assert_eq!(series[0].time, 7200);
    }
}
