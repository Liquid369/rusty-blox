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
use crate::gov::rejects::{Evidence, Reject};
use crate::gov::store::{parse_hash, parse_mn_id, scan, Hash, Outpoint, ProposalMeta};
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
/// Cluster and funding-table caps. Vouts are capped per cluster and for the
/// funding table, so the body holds at most 41 x 3,000 vouts x ~5 B plus
/// 41 x 100 txids x ~70 B: about 0.9 MiB, under the 2 MiB cache cap.
/// ponytail: the full sets are built before the cut; that memory is bounded
/// by the window's rejections, not the response.
const CLUSTERS_MAX: usize = 40;
const CLUSTER_GROUPS_MAX: usize = 100;
const FUNDING_MAX: usize = 100;
const VOUTS_MAX: usize = 3_000;
/// The windows the UI offers; anything else would be a fresh uncached build.
const WINDOWS: [u64; 2] = [24, 168];
/// gov.db scans and listmasternodes are reused this long across builds.
const INDEX_TTL: Duration = Duration::from_secs(60);
/// ponytail: cleared wholesale past this, not LRU; a refill costs one budget.
const RESOLVED_MAX: usize = 100_000;

/// Brute-force matches only (known() and read-back votes are checked first,
/// as they can appear later): a hash's preimage never changes.
fn resolved() -> &'static Mutex<HashMap<(Hash, i64), Resolution>> {
    static R: OnceLock<Mutex<HashMap<(Hash, i64), Resolution>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Brute-force misses, retried after MISS_TTL so a new proposal or collateral
/// can still match, and so the newest misses do not spend the hash budget on
/// every build and starve older rejections.
fn missed() -> &'static Mutex<HashMap<(Hash, i64), Instant>> {
    static M: OnceLock<Mutex<HashMap<(Hash, i64), Instant>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}
const MISS_TTL: Duration = Duration::from_secs(3600);

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

/// Evidence strength of one resolved rejection (gov::rejects::Evidence). A
/// read-back vote passed Core's signature check before the older/too-soon
/// test (budgetmanager.cpp:1154-1162).
fn evidence(mn: &Resolution, outcome: &Option<Outcome>) -> Option<Evidence> {
    match mn {
        Resolution::Known { .. } => Some(Evidence::Seen),
        Resolution::Recovered { .. } if matches!(outcome, Some(Outcome::RejectOlder)) => {
            Some(Evidence::Older)
        }
        Resolution::Recovered { .. } => Some(Evidence::Signed),
        Resolution::Matched { signed: true, .. } => Some(Evidence::Signed),
        Resolution::Matched { .. } | Resolution::Prefix { .. } => Some(Evidence::Claimed),
        Resolution::NotIdentifiable { .. } => None,
    }
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
    evidence: Option<Evidence>,
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

fn resolve(
    e: &Event,
    idx: &Index,
    recovered: Option<&govlog::Recovered>,
    budget: &mut u64,
) -> Resolution {
    let key = (e.vote, e.ts as i64);
    if let Some(r) = idx.known(&e.vote) {
        return r;
    }
    let read_back = recovered
        .and_then(|r| r.vote.as_ref())
        .map(|v| Resolution::Recovered {
            outpoint: crate::gov::votematch::outpoint_str(&v.outpoint()),
            object: hex::encode(v.proposal),
            vote: v.vote,
            n_time: v.n_time,
            canonical: v.canonical,
        });
    if let Some(r) = read_back {
        return r;
    }
    if let Some(r) = resolved().lock().ok().and_then(|m| m.get(&key).cloned()) {
        return r;
    }
    let recently_missed = missed()
        .lock()
        .ok()
        .and_then(|m| m.get(&key).copied())
        .is_some_and(|t| t.elapsed() < MISS_TTL);
    // Only a miss that cost hashes is remembered; cheap misses (older votes
    // without a read-back, finalized too-soon) would just crowd the cap.
    let mut searched = false;
    let attempt = if recently_missed {
        Attempt::NotFound
    } else {
        match &e.outcome {
            Some(Outcome::RejectFuture(t)) => {
                searched = true;
                idx.match_time(&e.vote, e.class == Class::Finalized, *t, budget)
            }
            Some(Outcome::RejectTooSoon(g)) if e.class != Class::Finalized => {
                searched = true;
                idx.match_gap(&e.vote, *g, budget)
            }
            _ => Attempt::NotFound,
        }
    };
    if attempt == Attempt::NotFound && searched {
        if let Ok(mut m) = missed().lock() {
            if m.len() >= RESOLVED_MAX {
                m.clear();
            }
            m.insert(key, Instant::now());
        }
    }
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
    let recovered = govlog::recovered(log, from, u64::MAX)?;
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

    // Every rejection in the window is resolved, newest first (the hash
    // budget goes to recent ones); the list shows the latest LIST_MAX, the
    // clusters and funding table cover all of them.
    let mut budget = HASH_BUDGET;
    let resolved_all: Vec<(&Event, Resolution)> = events
        .iter()
        .rev()
        .filter(|e| e.outcome.as_ref().is_some_and(Outcome::is_reject))
        .map(|e| {
            (
                e,
                resolve(e, idx, recovered.get(&(e.ts, e.vote)), &mut budget),
            )
        })
        .collect();
    let rejections: Vec<Rejection> = resolved_all
        .iter()
        .take(LIST_MAX)
        .map(|(e, mn)| {
            let (r, detail) = reason(&e.outcome);
            let mn = mn.clone();
            let evidence = evidence(&mn, &e.outcome);
            let object = match &mn {
                Resolution::Known { object, .. }
                | Resolution::Matched { object, .. }
                | Resolution::Recovered { object, .. } => Some(object.clone()),
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
                evidence,
            }
        })
        .collect();
    let reject_rows: Vec<Reject> = resolved_all
        .iter()
        .map(|(e, mn)| {
            let (object, choice, n_time, outpoint) = match mn {
                Resolution::Known {
                    object,
                    vote,
                    n_time,
                    outpoint,
                    ..
                }
                | Resolution::Matched {
                    object,
                    vote,
                    n_time,
                    outpoint,
                    ..
                } => (
                    parse_hash(object),
                    vote.map(u32::from),
                    Some(*n_time),
                    Some(outpoint),
                ),
                Resolution::Recovered {
                    object,
                    vote,
                    n_time,
                    outpoint,
                    ..
                } => (
                    parse_hash(object),
                    Some(*vote),
                    Some(*n_time),
                    Some(outpoint),
                ),
                _ => (e.object, None, None, None),
            };
            Reject {
                reason: reason(&e.outcome).0,
                object,
                choice,
                evidence: evidence(mn, &e.outcome),
                collaterals: outpoint.and_then(|o| parse_mn_id(o)).into_iter().collect(),
                ts: e.ts,
                n_time,
                count: e.seen,
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
    // Rejections without a vote hash (bad signature, unknown or invalid
    // masternode) name a collateral prefix only: they feed the funding table
    // as claimed, never the clusters.
    let mut funding_rows = reject_rows.clone();
    for o in grouped.values() {
        // A prefix that fits several collaterals is not charged to any of them.
        if let Some(Resolution::Prefix { outpoints }) =
            o.mn.as_ref()
                .filter(|m| matches!(m, Resolution::Prefix { outpoints } if outpoints.len() == 1))
        {
            funding_rows.push(Reject {
                reason: o.text.clone(),
                object: None,
                choice: None,
                evidence: Some(Evidence::Claimed),
                collaterals: outpoints.iter().filter_map(|x| parse_mn_id(x)).collect(),
                ts: o.last_ts,
                n_time: None,
                count: o.count,
            });
        }
    }
    let all_clusters = crate::gov::rejects::clusters(&reject_rows, CLUSTER_GROUPS_MAX, VOUTS_MAX);
    let clusters_total = all_clusters.len();
    let clusters: Vec<serde_json::Value> = all_clusters
        .into_iter()
        .take(CLUSTERS_MAX)
        .map(|c| {
            let name = c
                .object
                .as_deref()
                .and_then(parse_hash)
                .and_then(|h| names.get(&h));
            let mut v = serde_json::to_value(&c).unwrap_or_default();
            v["object_name"] = serde_json::json!(name);
            v
        })
        .collect();
    let funding_total = funding_rows
        .iter()
        .flat_map(|r| r.collaterals.iter().map(|c| &c[..32]))
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let funding = crate::gov::rejects::funding(&funding_rows, FUNDING_MAX, VOUTS_MAX);
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
            Resolution::Known { outpoint, .. }
            | Resolution::Matched { outpoint, .. }
            | Resolution::Recovered { outpoint, .. } => std::slice::from_ref(outpoint),
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
        "clusters": clusters,
        "clusters_total": clusters_total,
        "funding": funding,
        "funding_total": funding_total,
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

/// Groups of masternodes casting the same choices on several proposals
/// (gov::coord) over gov.db vote movement in the window.
fn build_groups(
    gov: &DB,
    log: Result<Option<Arc<DB>>, String>,
    hours: u64,
    ctx: &Context,
) -> Result<serde_json::Value, String> {
    use crate::gov::coord::{groups, Vote, BATCH_GAP, MIN_GROUP};
    use crate::gov::store::{get, MnCountPoint, VoteChange, LAST_MNCOUNT};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let from = now.saturating_sub(hours * 3600);
    let mut since: HashMap<Hash, Option<u64>> = HashMap::new();
    for (_, v) in scan(gov, b"p/")? {
        let m: ProposalMeta = bincode::deserialize(&v).map_err(|e| e.to_string())?;
        since.insert(m.hash, m.votes_since);
    }
    // c/H/T/M. A row at or before its proposal's votes_since is a first
    // sighting, not movement; T bounds n_time to within ~1 h (Core's future
    // limit), so scan from an hour before the window.
    let mut votes = Vec::new();
    for (k, v) in scan(gov, b"c/")? {
        let (Some(h), Some(t), Some(mn)) = (
            k.get(2..34).and_then(|b| <Hash>::try_from(b).ok()),
            k.get(34..42)
                .and_then(|b| b.try_into().ok())
                .map(u64::from_be_bytes),
            k.get(42..78).and_then(|b| <Outpoint>::try_from(b).ok()),
        ) else {
            continue;
        };
        let Some(Some(vs)) = since.get(&h) else {
            continue;
        };
        if t <= *vs || t + 3600 < from {
            continue;
        }
        let c: VoteChange = bincode::deserialize(&v).map_err(|e| e.to_string())?;
        let Some(to) = c.to else { continue };
        // Signed time may run up to 1 h ahead; the window ends now.
        if c.n_time < from as i64 || c.n_time > now as i64 {
            continue;
        }
        votes.push(Vote {
            proposal: h,
            mn,
            kind: c.kind,
            from: c.from,
            to,
            n_time: c.n_time,
            f_valid: c.f_valid,
        });
    }
    let enabled = get::<MnCountPoint>(gov, LAST_MNCOUNT)?.map(|m| m.enabled);
    // Node-side removals (sidecar), to show what a group's votes led to. A
    // configured store that fails is reported, never read as "no removals".
    let (log, removals_error) = match log {
        Ok(l) => (l, None),
        Err(e) => (None, Some(e)),
    };
    let removals: Vec<(u64, Hash, String)> = match &log {
        Some(l) => govlog::events(l, from, u64::MAX)?
            .into_iter()
            .filter_map(|e| match e.outcome {
                Some(Outcome::Removed(r)) => Some((e.ts, e.vote, r)),
                _ => None,
            })
            .collect(),
        None => Vec::new(),
    };
    let found = groups(&votes, BATCH_GAP, MIN_GROUP);
    let total = found.len();
    let out: Vec<serde_json::Value> = found
        .into_iter()
        .take(GROUPS_MAX)
        .map(|g| {
            let choices: Vec<_> = g
                .choices
                .iter()
                .take(CHOICES_MAX)
                .map(|(h, c)| {
                    let swing = g.swing.iter().find(|(s, _)| s == h).map_or(0, |s| s.1);
                    let removed = removals
                        .iter()
                        .find(|(ts, p, _)| p == h && *ts as i64 >= g.first)
                        .map(|(ts, _, r)| serde_json::json!({"ts": ts, "reason": r}));
                    serde_json::json!({
                        "proposal": hex::encode(h),
                        "name": ctx.names.get(h),
                        "choice": c,
                        "swing": swing,
                        "removed": removed,
                    })
                })
                .collect();
            let members: Vec<_> = g
                .members
                .iter()
                .take(MEMBERS_MAX)
                .map(|m| {
                    let op = crate::gov::votematch::outpoint_str(m);
                    let addr = ctx.addrs.get(&op).cloned();
                    serde_json::json!({"outpoint": op, "addr": addr})
                })
                .collect();
            serde_json::json!({
                "size": g.members.len(),
                "first": g.first,
                "last": g.last,
                "new": g.new,
                "changed": g.changed,
                "unknown_from": g.unknown_from,
                "choices": choices,
                "members": members,
                "members_total": g.members.len(),
                // Distinct collateral funding transactions: shared ones are an
                // on-chain link between members; one per member is none.
                "funding_txs": g.members.iter().map(|m| &m[..32]).collect::<std::collections::BTreeSet<_>>().len(),
                "choices_total": g.choices.len(),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "as_of": now,
        "hours": hours,
        "enabled_masternodes": enabled,
        "batch_gap_secs": crate::gov::coord::BATCH_GAP,
        "min_group": crate::gov::coord::MIN_GROUP,
        "groups": out,
        "groups_total": total,
        "removals_error": removals_error,
    }))
}

/// 20 groups x 250 members x ~140 B stays near 0.7 MiB, under CacheManager's
/// 2 MiB entry limit; 250 covers the largest group seen (213, 2026-10-20).
/// A ballot lists at most one choice per live proposal (16 on 2026-10-21).
const GROUPS_MAX: usize = 20;
const MEMBERS_MAX: usize = 250;
const CHOICES_MAX: usize = 50;

/// GET /api/v2/govgroups?hours=24|168
///
/// **CACHED**: 60 second TTL
pub async fn govgroups_v2(
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
    let Some(gov) = stores.gov.clone() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "governance history unavailable"})),
        ));
    };
    cache
        .get_or_compute(
            &format!("govgroups:{hours}"),
            Duration::from_secs(60),
            || async move {
                let permit = build_permit()
                    .acquire_owned()
                    .await
                    .map_err(|e| e.to_string())?;
                // Same shape as govlog_v2: the spawned task holds the permit
                // past a request timeout. The vote log is optional here.
                tokio::spawn(async move {
                    let _permit = permit;
                    let ctx = context(Some(Arc::clone(&gov))).await?;
                    tokio::task::spawn_blocking(move || {
                        let log = match stores.govlog_path {
                            None => Ok(None),
                            Some(_) => stores.govlog().and_then(|l| {
                                l.try_catch_up_with_primary().map_err(|e| e.to_string())?;
                                Ok(Some(l))
                            }),
                        };
                        build_groups(&gov, log, hours, &ctx)
                    })
                    .await
                    .map_err(|e| e.to_string())?
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
