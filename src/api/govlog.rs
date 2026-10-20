// GET /api/v2/govlog: budget votes the sidecar read from pivxd's debug.log,
// with the masternode behind each rejected vote recovered where the vote hash
// allows it (gov::votematch).

use axum::{extract::Query, http::StatusCode, Extension, Json};
use rocksdb::DB;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::cache::CacheManager;
use crate::gov::store::{parse_hash, scan, Hash, Outpoint, ProposalMeta};
use crate::gov::votematch::{Attempt, Index, Resolution};
use crate::govlog::{self, Class, Event, Outcome};

/// gov.db (the explorer's own) and the sidecar's govlog.db (read as a RocksDB
/// secondary). Either may be absent; the endpoint says which.
#[derive(Clone, Default)]
pub struct GovStores {
    pub gov: Option<Arc<DB>>,
    /// (govlog.db path, secondary scratch dir), when configured.
    pub govlog_path: Option<(String, String)>,
    govlog: Arc<Mutex<Option<Arc<DB>>>>,
}

impl GovStores {
    /// Opens on first use and retries on every request until it succeeds, so
    /// a sidecar started after the explorer is picked up without a restart.
    pub fn govlog(&self) -> Result<Arc<DB>, String> {
        let (path, secondary) = self
            .govlog_path
            .as_ref()
            .ok_or("governance.govlog_db_path not set")?;
        let mut slot = self.govlog.lock().map_err(|_| "govlog lock poisoned")?;
        if let Some(db) = slot.as_ref() {
            return Ok(Arc::clone(db));
        }
        let db = Arc::new(govlog::open_secondary(path, secondary)?);
        *slot = Some(Arc::clone(&db));
        Ok(db)
    }

    /// Drops the handle after a failed build so the next request reopens.
    fn reset_govlog(&self) {
        if let Ok(mut slot) = self.govlog.lock() {
            *slot = None;
        }
    }
}

#[derive(serde::Deserialize)]
pub struct GovLogQuery {
    hours: Option<u64>,
}

/// Hashes tried per build before further rejections are reported as not
/// attempted. A future-time match costs collaterals x proposals x 3 (~100k at
/// 1,968 x 17), so ~10 unresolved future votes per build, ~0.3 s of SHA256d.
/// Cost: a burst of more unmatched future votes shows the rest as skipped.
const HASH_BUDGET: u64 = 1_000_000;
/// Rejections listed per response, newest first.
const LIST_MAX: usize = 200;
/// The windows the UI offers; anything else would be a fresh uncached build.
const WINDOWS: [u64; 2] = [24, 168];
/// gov.db scans and listmasternodes are reused this long across builds.
const INDEX_TTL: Duration = Duration::from_secs(60);
/// ponytail: cleared wholesale past this, not LRU; a refill costs one budget.
const RESOLVED_MAX: usize = 100_000;

/// Positive resolutions never change, so they are kept for the process.
fn resolved() -> &'static Mutex<HashMap<(Hash, i64), Resolution>> {
    static R: OnceLock<Mutex<HashMap<(Hash, i64), Resolution>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Serializes builds across the two window keys (each key is already
/// single-flight in CacheManager). Owned so the spawned build task can hold
/// it past a request timeout.
fn build_permit() -> Arc<tokio::sync::Semaphore> {
    static S: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    Arc::clone(S.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1))))
}

struct Context {
    idx: Index,
    names: HashMap<Hash, String>,
    /// Collateral ("txid-n") to the address listmasternodes pays.
    addrs: HashMap<String, String>,
}
type Ctx = Arc<Context>;

fn ctx_cache() -> &'static Mutex<Option<(Instant, Ctx)>> {
    static C: OnceLock<Mutex<Option<(Instant, Ctx)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// The match index and object names, rebuilt at most once per INDEX_TTL.
async fn context(gov: Option<Arc<DB>>) -> Result<Ctx, String> {
    if let Some((at, c)) = ctx_cache().lock().ok().and_then(|g| g.clone()) {
        if at.elapsed() < INDEX_TTL {
            return Ok(c);
        }
    }
    let (listed, addrs): (Vec<Outpoint>, HashMap<String, String>) = listed_collaterals()
        .await
        .into_iter()
        .map(|(o, a)| (o, (crate::gov::votematch::outpoint_str(&o), a)))
        .unzip();
    let c: Ctx = tokio::task::spawn_blocking(move || {
        let (idx, names) = match gov.as_deref() {
            Some(g) => (Index::build(g, &listed)?, names(g)?),
            None => (Index::build_empty(listed), HashMap::new()),
        };
        Ok::<_, String>(Arc::new(Context { idx, names, addrs }))
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Ok(mut g) = ctx_cache().lock() {
        *g = Some((Instant::now(), Arc::clone(&c)));
    }
    Ok(c)
}

#[derive(Serialize)]
struct Rejection {
    ts: u64,
    class: String,
    reason: String,
    /// Seconds after the stored vote (too soon) or the vote's nTime (future).
    detail: Option<i64>,
    vote: String,
    seen: u32,
    /// "proposal" or "finalized": which page `object` belongs on.
    object_kind: &'static str,
    object: Option<String>,
    object_name: Option<String>,
    /// As Core logged it, if it did.
    mn_logged: Option<String>,
    mn: Resolution,
}

#[derive(Serialize)]
struct Other {
    class: String,
    text: String,
    count: u32,
    last_ts: u64,
    mn: Option<Resolution>,
}

fn reason(o: &Option<Outcome>) -> (String, Option<i64>) {
    match o {
        None => ("accepted".into(), None),
        Some(Outcome::Inserted) => ("accepted: new".into(), None),
        Some(Outcome::Updated) => ("accepted: changed".into(), None),
        Some(Outcome::RejectOlder) => ("older than the stored vote".into(), None),
        Some(Outcome::RejectTooSoon(g)) => ("under 1 h after the stored vote".into(), Some(*g)),
        Some(Outcome::RejectFuture(t)) => ("signed over 1 h in the future".into(), Some(*t)),
        Some(Outcome::Removed(r)) => (format!("proposal removed: {r}"), None),
    }
}

/// Collaterals the node lists now, as gov.db outpoints, with their address.
async fn listed_collaterals() -> Vec<(Outpoint, String)> {
    let Ok(v) = crate::api::helpers::rpc_call_json("listmasternodes", serde_json::json!([])).await
    else {
        return Vec::new();
    };
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let txid = parse_hash(m.get("txhash")?.as_str()?)?;
            let n = u32::try_from(m.get("outidx")?.as_u64()?).ok()?;
            let mut o = [0u8; 36];
            o[..32].copy_from_slice(&txid);
            o[32..].copy_from_slice(&n.to_be_bytes());
            Some((o, m.get("addr")?.as_str()?.to_string()))
        })
        .collect()
}

fn names(gov: &DB) -> Result<HashMap<Hash, String>, String> {
    let mut out = HashMap::new();
    for (_, v) in scan(gov, b"p/")? {
        let m: ProposalMeta = bincode::deserialize(&v).map_err(|e| e.to_string())?;
        out.insert(m.hash, m.name);
    }
    for (_, v) in scan(gov, b"f/")? {
        let f: crate::gov::store::FinalizedBudget =
            bincode::deserialize(&v).map_err(|e| e.to_string())?;
        out.insert(f.hash, format!("finalized budget {}", f.name));
    }
    Ok(out)
}

fn resolve(e: &Event, idx: &Index, budget: &mut u64) -> Resolution {
    let key = (e.vote, e.ts as i64);
    if let Some(r) = resolved().lock().ok().and_then(|m| m.get(&key).cloned()) {
        return r;
    }
    let attempt = match idx.known(&e.vote) {
        Some(r) => Attempt::Found(r),
        None => match &e.outcome {
            Some(Outcome::RejectFuture(t)) => {
                idx.match_time(&e.vote, e.class == Class::Finalized, *t, budget)
            }
            Some(Outcome::RejectTooSoon(g)) if e.class != Class::Finalized => {
                idx.match_gap(&e.vote, *g, budget)
            }
            _ => Attempt::NotFound,
        },
    };
    let reason = match attempt {
        Attempt::Found(r) => {
            if let Ok(mut m) = resolved().lock() {
                if m.len() >= RESOLVED_MAX {
                    m.clear();
                }
                m.insert(key, r.clone());
            }
            return r;
        }
        Attempt::Skipped => "not attempted: per-request hash budget spent".into(),
        Attempt::NotFound => match &e.outcome {
            Some(Outcome::RejectOlder) => {
                "older than the stored vote and never seen before; its signed time is not logged"
                    .into()
            }
            Some(Outcome::RejectTooSoon(_)) if e.class == Class::Finalized => {
                "finalized-budget vote history is not stored per masternode".into()
            }
            _ => "no known collateral and object produce this hash".into(),
        },
    };
    Resolution::NotIdentifiable { reason }
}

fn build(log: &DB, hours: u64, ctx: &Context) -> Result<serde_json::Value, String> {
    let (idx, names) = (&ctx.idx, &ctx.names);
    log.try_catch_up_with_primary().map_err(|e| e.to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let from = now.saturating_sub(hours * 3600);
    let events = govlog::events(log, from, u64::MAX)?;
    let others = govlog::others(log, from, u64::MAX)?;
    let gaps: Vec<_> = govlog::gaps(log)?
        .into_iter()
        .filter(|(start, until, _)| *until > from && *start < now)
        .collect();
    let coverage_from = govlog::since(log)?;

    let mut totals: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut hourly: BTreeMap<u64, BTreeMap<String, u32>> = BTreeMap::new();
    let mut removals = Vec::new();
    for e in &events {
        let (r, _) = reason(&e.outcome);
        let label = if e.outcome.as_ref().is_some_and(Outcome::is_reject) {
            r.clone()
        } else if matches!(e.outcome, Some(Outcome::Removed(_))) {
            "proposal removed".into()
        } else {
            "accepted".into()
        };
        // `seen` counts repeats of one vote hash at one ts; 0 only in old rows.
        let n = e.seen.max(1);
        *totals.entry((format!("{:?}", e.class), r)).or_default() += n;
        *hourly
            .entry(e.ts - e.ts % 3600)
            .or_default()
            .entry(label)
            .or_default() += n;
        if let Some(Outcome::Removed(why)) = &e.outcome {
            removals.push(serde_json::json!({
                "ts": e.ts,
                "proposal": hex::encode(e.vote),
                "name": names.get(&e.vote),
                "reason": why,
            }));
        }
    }

    let mut budget = HASH_BUDGET;
    let rejections: Vec<Rejection> = events
        .iter()
        .rev()
        .filter(|e| e.outcome.as_ref().is_some_and(Outcome::is_reject))
        .take(LIST_MAX)
        .map(|e| {
            let (r, detail) = reason(&e.outcome);
            let mn = resolve(e, idx, &mut budget);
            let object = match &mn {
                Resolution::Known { object, .. } | Resolution::Matched { object, .. } => {
                    Some(object.clone())
                }
                _ => e.object.map(hex::encode),
            };
            let object_name = object
                .as_deref()
                .and_then(parse_hash)
                .and_then(|h| names.get(&h).cloned());
            Rejection {
                ts: e.ts,
                class: format!("{:?}", e.class),
                reason: r,
                detail,
                vote: hex::encode(e.vote),
                seen: e.seen,
                object_kind: if e.class == Class::Finalized {
                    "finalized"
                } else {
                    "proposal"
                },
                object,
                object_name,
                mn_logged: e.mn.clone(),
                mn,
            }
        })
        .collect();

    let mut grouped: BTreeMap<(String, String), Other> = BTreeMap::new();
    for o in others {
        let class = format!("{:?}", o.class);
        let g = grouped
            .entry((class.clone(), o.text.clone()))
            .or_insert_with(|| Other {
                class,
                text: o.text.clone(),
                count: 0,
                last_ts: 0,
                mn: idx.prefix(&o.text),
            });
        g.count += o.seen;
        g.last_ts = g.last_ts.max(o.ts);
    }
    // Capped like rejections: a body past CacheManager's 2 MiB limit is
    // never cached, so every request would rebuild it.
    let mut others: Vec<Other> = grouped.into_values().collect();
    others.sort_by(|a, b| b.count.cmp(&a.count).then(b.last_ts.cmp(&a.last_ts)));
    let others_total = others.len();
    others.truncate(LIST_MAX);
    let removals_total = removals.len();
    removals.truncate(LIST_MAX);

    // Only collaterals this response names; unlisted ones (spent, or out of
    // the node's list) have no address here.
    let mut addresses: BTreeMap<&str, &str> = BTreeMap::new();
    let named = rejections
        .iter()
        .map(|r| &r.mn)
        .chain(others.iter().filter_map(|o| o.mn.as_ref()));
    for mn in named {
        let ops = match mn {
            Resolution::Known { outpoint, .. } | Resolution::Matched { outpoint, .. } => {
                std::slice::from_ref(outpoint)
            }
            Resolution::Prefix { outpoints } => outpoints.as_slice(),
            Resolution::NotIdentifiable { .. } => &[],
        };
        for op in ops {
            if let Some(a) = ctx.addrs.get(op) {
                addresses.insert(op, a);
            }
        }
    }

    Ok(serde_json::json!({
        "as_of": now,
        "hours": hours,
        "coverage_from": coverage_from,
        "gaps": gaps.iter().map(|(f, u, g)| serde_json::json!({"from": f, "until": u, "reason": g.reason})).collect::<Vec<_>>(),
        "totals": totals.iter().map(|((c, r), n)| serde_json::json!({"class": c, "reason": r, "count": n})).collect::<Vec<_>>(),
        "hourly": hourly.iter().map(|(h, m)| serde_json::json!({"hour": h, "counts": m})).collect::<Vec<_>>(),
        "rejections": rejections,
        "others": others,
        "others_total": others_total,
        "removals": removals,
        "removals_total": removals_total,
        "collaterals_known": idx.collaterals.len(),
        "addresses": addresses,
        "hash_budget_left": budget,
    }))
}

/// GET /api/v2/govlog?hours=24|168
///
/// **CACHED**: 30 second TTL
pub async fn govlog_v2(
    Query(q): Query<GovLogQuery>,
    Extension(stores): Extension<GovStores>,
    Extension(cache): Extension<Arc<CacheManager>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let hours = q.hours.unwrap_or(24);
    if !WINDOWS.contains(&hours) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "hours must be 24 or 168"})),
        ));
    }
    let s = stores.clone();
    let log = tokio::task::spawn_blocking(move || s.govlog())
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r)
        .map_err(|detail| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "vote log unavailable", "detail": detail})),
            )
        })?;
    cache
        .get_or_compute(
            &format!("govlog:{hours}"),
            Duration::from_secs(30),
            || async move {
                let permit = build_permit()
                    .acquire_owned()
                    .await
                    .map_err(|e| e.to_string())?;
                // Spawned, not awaited inline: a request timeout drops this
                // future, and the permit must outlive every blocking task the
                // build starts (context() and build()), not the request.
                tokio::spawn(async move {
                    let _permit = permit;
                    let built = match context(stores.gov.clone()).await {
                        Ok(ctx) => tokio::task::spawn_blocking(move || build(&log, hours, &ctx))
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|r| r),
                        Err(e) => Err(e),
                    };
                    if built.is_err() {
                        stores.reset_govlog();
                    }
                    built
                })
                .await
                .map_err(|e| e.to_string())?
            },
        )
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
        })
}
