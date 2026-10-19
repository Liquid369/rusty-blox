// Governance poller: the only writer of gov.db. A detached task (never hooked
// into run_block_monitor, which re-enters after errors), one poll per interval.
// A failed call is recorded as a gap; nothing advances silently.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rocksdb::{WriteBatch, DB};
use serde_json::Value;
use tracing::{error, info, warn};

use super::diff::{diff_votes, parse_votes};
use super::store::*;

pub type RpcFut = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;
pub type Rpc = dyn Fn(&'static str, Value) -> RpcFut + Send + Sync;

/// Set once the first fully successful poll lands. Until then every poll is a
/// baseline: all votes are New and none of it is movement.
const BASELINED: &[u8] = b"meta/baselined";

/// One proposal row from getbudgetinfo, tolerant of missing fields so a new
/// Core field (or a dropped one) cannot stop history collection.
#[derive(Debug, Clone)]
struct Row {
    hash: Hash,
    name: String,
    url: String,
    fee_hash: Hash,
    payment_address: String,
    monthly_payment_sats: i64,
    block_start: u32,
    block_end: u32,
    total_payment_count: u32,
    remaining_payment_count: i32,
    yeas: u32,
    nays: u32,
    abstains: u32,
    is_valid: bool,
    is_established: bool,
}

/// Core emits every field read here unconditionally (rpc/budget.cpp
/// budgetToJSON), so a missing or mistyped one means a malformed response and
/// the whole row is rejected; defaulting it would write false history. Extra
/// fields are ignored.
fn row(v: &Value) -> Option<Row> {
    let s = |k: &str| v.get(k)?.as_str().map(str::to_string);
    let u = |k: &str| v.get(k)?.as_u64().and_then(|n| u32::try_from(n).ok());
    let b = |k: &str| v.get(k)?.as_bool();
    Some(Row {
        hash: parse_hash(&s("Hash")?)?,
        name: s("Name")?,
        url: s("URL")?,
        fee_hash: parse_hash(&s("FeeHash")?)?,
        payment_address: s("PaymentAddress")?,
        // Core emits a CAmount; anything non-finite, negative or beyond the
        // supply is malformed, not a value to saturate into storage.
        monthly_payment_sats: v
            .get("MonthlyPayment")?
            .as_f64()
            .filter(|p| p.is_finite() && (0.0..=1e9).contains(p))
            .map(|p| (p * 1e8).round() as i64)?,
        block_start: u("BlockStart")?,
        block_end: u("BlockEnd")?,
        total_payment_count: u("TotalPaymentCount")?,
        remaining_payment_count: i32::try_from(v.get("RemainingPaymentCount")?.as_i64()?).ok()?,
        yeas: u("Yeas")?,
        nays: u("Nays")?,
        abstains: u("Abstains")?,
        is_valid: b("IsValid")?,
        is_established: b("IsEstablished")?,
    })
}

fn rows(v: &Value, what: &str, gaps: &mut Vec<String>) -> Vec<Row> {
    let Some(arr) = v.as_array() else {
        gaps.push(format!("{what}: not an array"));
        return Vec::new();
    };
    arr.iter()
        .filter_map(|r| {
            let parsed = row(r);
            if parsed.is_none() {
                gaps.push(format!("{what}: malformed row {r}"));
            }
            parsed
        })
        .collect()
}

fn is_unknown_proposal(e: &str) -> bool {
    let e = e.to_lowercase();
    e.contains("unknown proposal") || e.contains("invalid proposal")
}

fn tallies_match(cur: &BTreeMap<Outpoint, VoteState>, r: &Row) -> bool {
    let n = |vote| cur.values().filter(|s| s.f_valid && s.vote == vote).count() as u32;
    (n(1), n(2), n(0)) == (r.yeas, r.nays, r.abstains)
}

fn mn_count(v: &Value) -> Option<MnCountPoint> {
    let g = |k: &str| v.get(k).and_then(|x| x.as_u64()).map(|n| n as u32);
    Some(MnCountPoint {
        total: g("total")?,
        stable: g("stable")?,
        enabled: g("enabled")?,
    })
}

fn write_failed(
    db: &DB,
    now: u64,
    height: u32,
    reason: String,
    ms: u32,
) -> Result<PollStatus, String> {
    let status = PollStatus {
        ts: now,
        height,
        ok: false,
        proposals: 0,
        votes: 0,
        ms,
        baseline: false,
    };
    let mut b = WriteBatch::default();
    b.put(gap_key(now), enc(&PollGap { reason })?);
    b.put(LAST_POLL, enc(&status)?);
    b.delete(POLL_STARTED);
    db.write(b).map_err(|e| e.to_string())?;
    Ok(status)
}

/// One poll. RPC failures are recorded as gaps and reported through
/// `ok = false`. A gov.db storage failure mid-poll also records a gap (best
/// effort) before returning Err, so earlier per-proposal commits are never an
/// unmarked partial poll.
pub async fn poll_once(db: &DB, rpc: &Rpc, now: u64, height: u32) -> Result<PollStatus, String> {
    match poll_inner(db, rpc, now, height).await {
        Ok(s) => Ok(s),
        Err(e) => {
            let _ = write_failed(db, now, height, format!("gov.db: {e}"), 0);
            Err(e)
        }
    }
}

async fn poll_inner(db: &DB, rpc: &Rpc, now: u64, height: u32) -> Result<PollStatus, String> {
    let started = Instant::now();
    let ms = || started.elapsed().as_millis().min(u32::MAX as u128) as u32;
    let baseline = db.get(BASELINED).map_err(|e| e.to_string())?.is_none();
    db.put(POLL_STARTED, now.to_be_bytes())
        .map_err(|e| e.to_string())?;

    // A node still syncing tier-two data serves partial vote maps that would
    // diff as mass Removed then New. Phase 999 = MASTERNODE_SYNC_FINISHED.
    match rpc("mnsync", Value::Array(vec!["status".into()])).await {
        Ok(v) => {
            let chain = v.get("IsBlockchainSynced").and_then(|x| x.as_bool()) == Some(true);
            let phase = v.get("RequestedMasternodeAssets").and_then(|x| x.as_i64());
            if !chain || phase != Some(999) {
                let reason =
                    format!("node tier-two sync incomplete (chain {chain}, phase {phase:?})");
                return write_failed(db, now, height, reason, ms());
            }
        }
        Err(e) => return write_failed(db, now, height, format!("mnsync status: {e}"), ms()),
    }

    // Global state first; without all three nothing can be diffed correctly.
    let mnc = match rpc("getmasternodecount", Value::Array(vec![]))
        .await
        .and_then(|v| mn_count(&v).ok_or_else(|| "getmasternodecount: missing fields".to_string()))
    {
        Ok(m) => m,
        Err(e) => return write_failed(db, now, height, e, ms()),
    };
    let list = match rpc("getbudgetinfo", Value::Array(vec![])).await {
        Ok(v) => v,
        Err(e) => return write_failed(db, now, height, format!("getbudgetinfo: {e}"), ms()),
    };
    let projection = match rpc("getbudgetprojection", Value::Array(vec![])).await {
        Ok(v) => v,
        Err(e) => return write_failed(db, now, height, format!("getbudgetprojection: {e}"), ms()),
    };

    let mut gaps: Vec<String> = Vec::new();
    let mut list_gaps = Vec::new();
    let mut live = rows(&list, "getbudgetinfo", &mut list_gaps);
    let mut seen = HashSet::new();
    if let Some(dup) = live.iter().find(|r| !seen.insert(r.hash)) {
        list_gaps.push(format!(
            "getbudgetinfo: duplicate hash {}",
            hex::encode(dup.hash)
        ));
    }
    if !list_gaps.is_empty() {
        // A dropped row would read as a vanished proposal and be marked gone.
        return write_failed(db, now, height, list_gaps.join("; "), ms());
    }
    // A dropped projection row would record a funded proposal as unfunded, so
    // an imperfect projection fails the poll instead.
    let mut proj_gaps = Vec::new();
    let funded: HashSet<Hash> = rows(&projection, "getbudgetprojection", &mut proj_gaps)
        .into_iter()
        .map(|r| r.hash)
        .collect();
    if !proj_gaps.is_empty() {
        return write_failed(db, now, height, proj_gaps.join("; "), ms());
    }

    let mut metas: BTreeMap<Hash, ProposalMeta> = BTreeMap::new();
    for (_, v) in scan(db, b"p/")? {
        let m: ProposalMeta = bincode::deserialize(&v).map_err(|e| e.to_string())?;
        metas.insert(m.hash, m);
    }
    let stored_vis: HashMap<Hash, Visibility> =
        metas.values().map(|m| (m.hash, m.visibility)).collect();

    // The no-arg list skips invalid proposals; ask for each missing one by name
    // before concluding it is gone.
    let listed: HashSet<Hash> = live.iter().map(|r| r.hash).collect();
    let missing: Vec<ProposalMeta> = metas
        .values()
        .filter(|m| m.gone_at.is_none() && !listed.contains(&m.hash))
        .cloned()
        .collect();
    for m in missing {
        match rpc("getbudgetinfo", Value::Array(vec![m.name.clone().into()])).await {
            Ok(v) => match rows(&v, "getbudgetinfo(name)", &mut gaps)
                .into_iter()
                .next()
            {
                Some(r) if r.hash == m.hash => live.push(r),
                // Not listed and the name resolves elsewhere: deleted or
                // invalid, unknowable which. last_seen keeps the last real
                // sighting; the s/ row marks where the blind interval starts.
                Some(_) => {
                    if let Some(meta) = metas.get_mut(&m.hash) {
                        meta.visibility = Visibility::Unresolvable;
                    }
                }
                None => gaps.push(format!("getbudgetinfo({}): empty", m.name)),
            },
            Err(e) if is_unknown_proposal(&e) => {
                if let Some(meta) = metas.get_mut(&m.hash) {
                    meta.gone_at = Some(now);
                }
            }
            Err(e) => gaps.push(format!("getbudgetinfo({}): {e}", m.name)),
        }
    }

    let mut votes_seen: u32 = 0;
    for r in &live {
        // getbudgetvotes resolves a name to the highest-net proposal of that
        // name (valid or not). getbudgetinfo(name) uses the same resolver, so
        // its hash proves whose votes the next call returns.
        let owner = match rpc("getbudgetinfo", Value::Array(vec![r.name.clone().into()])).await {
            Ok(v) => match rows(&v, "getbudgetinfo(name)", &mut gaps)
                .into_iter()
                .next()
            {
                Some(o) => Some(o.hash),
                None => {
                    gaps.push(format!("getbudgetinfo({}): empty", r.name));
                    continue;
                }
            },
            // Listed, yet unknown by name: Core accepts names its by-name RPCs
            // sanitize away (CheckStrings is not enforced), so neither call can
            // ever reach it. Record what the list gives, no votes, no gap; a
            // gap here would fail every poll forever.
            Err(e) if is_unknown_proposal(&e) => None,
            Err(e) => {
                gaps.push(format!("getbudgetinfo({}): {e}", r.name));
                continue;
            }
        };
        let shadowed = owner != Some(r.hash);
        let visibility = if shadowed {
            Visibility::Shadowed
        } else {
            Visibility::Readable
        };

        let mut batch = WriteBatch::default();
        let prior = metas.get(&r.hash);
        let first_seen = prior.map(|m| m.first_seen).unwrap_or(now);
        let mut meta = ProposalMeta {
            hash: r.hash,
            name: r.name.clone(),
            url: r.url.clone(),
            fee_hash: r.fee_hash,
            payment_address: r.payment_address.clone(),
            monthly_payment_sats: r.monthly_payment_sats,
            block_start: r.block_start,
            block_end: r.block_end,
            total_payment_count: r.total_payment_count,
            first_seen,
            last_seen: now,
            gone_at: None,
            visibility,
            votes_since: prior.and_then(|m| m.votes_since),
        };

        let totals = TotalsPoint {
            yeas: r.yeas,
            nays: r.nays,
            abstains: r.abstains,
            is_valid: r.is_valid,
            is_established: r.is_established,
            enabled_mns: mnc.enabled,
            remaining_payment_count: r.remaining_payment_count,
            in_projection: funded.contains(&r.hash),
        };
        if get::<TotalsPoint>(db, &latest_totals_key(&r.hash))?.as_ref() != Some(&totals) {
            batch.put(totals_key(&r.hash, now), enc(&totals)?);
            batch.put(latest_totals_key(&r.hash), enc(&totals)?);
        }

        if !shadowed {
            match rpc("getbudgetvotes", Value::Array(vec![r.name.clone().into()]))
                .await
                .and_then(|v| parse_votes(&v))
            {
                // Core's tallies count valid votes only; the polled set must
                // reproduce them, or a vote landed between the calls. Either
                // way the proposal is retried whole next poll.
                Ok(cur) if !tallies_match(&cur, r) => {
                    gaps.push(format!(
                        "getbudgetvotes({}): valid votes do not reproduce {}/{}/{}",
                        r.name, r.yeas, r.nays, r.abstains
                    ));
                    continue;
                }
                Ok(cur) => {
                    // Probe again: a vote between the first probe and the votes
                    // call can change which same-named proposal wins. The
                    // owner must hold on both sides of the read.
                    let after = rpc("getbudgetinfo", Value::Array(vec![r.name.clone().into()]))
                        .await
                        .ok()
                        .and_then(|v| {
                            rows(&v, "getbudgetinfo(name)", &mut gaps)
                                .into_iter()
                                .next()
                        })
                        .map(|o| o.hash);
                    if after != Some(r.hash) {
                        gaps.push(format!(
                            "getbudgetvotes({}): name owner changed during read",
                            r.name
                        ));
                        continue;
                    }
                    votes_seen += cur.len() as u32;
                    // First read of this proposal's votes (new, or readable
                    // for the first time): its transitions this poll are
                    // first sightings, not movement.
                    meta.votes_since.get_or_insert(now);
                    let prefix = vote_prefix(&r.hash);
                    let mut prev = HashMap::new();
                    for (k, v) in scan(db, &prefix)? {
                        let mn: Outpoint = k[prefix.len()..]
                            .try_into()
                            .map_err(|_| "gov.db: malformed v/ key".to_string())?;
                        prev.insert(mn, bincode::deserialize(&v).map_err(|e| e.to_string())?);
                    }
                    for t in diff_votes(&prev, &cur) {
                        match t.state {
                            Some(s) => batch.put(vote_key(&r.hash, &t.mn), enc(&s)?),
                            None => batch.delete(vote_key(&r.hash, &t.mn)),
                        }
                        batch.put(change_key(&r.hash, now, &t.mn), enc(&t.change)?);
                    }
                }
                // Nothing written for this proposal, totals included: totals
                // committed without their votes would split the two timelines.
                Err(e) => {
                    gaps.push(format!("getbudgetvotes({}): {e}", r.name));
                    continue;
                }
            }
        }
        batch.put(proposal_key(&r.hash), enc(&meta)?);
        if stored_vis.get(&r.hash) != Some(&visibility) {
            batch.put(visibility_key(&r.hash, now), enc(&visibility)?);
        }
        metas.remove(&r.hash);
        db.write(batch).map_err(|e| e.to_string())?;
    }

    // The payment schedule exists only from finalization until BlockEnd + 200
    // and is the only RPC source of which proposal each block pays.
    let finalized = match rpc("mnfinalbudget", Value::Array(vec!["show".into()])).await {
        Ok(v) => finalized_rows(&v, now, &mut gaps),
        Err(e) => {
            gaps.push(format!("mnfinalbudget show: {e}"));
            Vec::new()
        }
    };

    // Stored proposals the node no longer returns (gone or shadowed this poll).
    let mut tail = WriteBatch::default();
    for m in metas.values() {
        tail.put(proposal_key(&m.hash), enc(m)?);
        if stored_vis.get(&m.hash) != Some(&m.visibility) {
            tail.put(visibility_key(&m.hash, now), enc(&m.visibility)?);
        }
    }
    for mut f in finalized {
        let old = get::<FinalizedBudget>(db, &finalized_key(&f.hash))?;
        if old.as_ref().map(|o| o.votes()) != Some(f.votes()) {
            tail.put(finalized_votes_key(&f.hash, now), enc(&f.votes())?);
        }
        if let Some(o) = old {
            f.first_seen = o.first_seen;
        }
        tail.put(finalized_key(&f.hash), enc(&f)?);
    }
    if get::<MnCountPoint>(db, LAST_MNCOUNT)? != Some(mnc) {
        tail.put(mncount_key(now), enc(&mnc)?);
        tail.put(LAST_MNCOUNT, enc(&mnc)?);
    }
    let ok = gaps.is_empty();
    if !ok {
        tail.put(
            gap_key(now),
            enc(&PollGap {
                reason: gaps.join("; "),
            })?,
        );
    }
    let status = PollStatus {
        ts: now,
        height,
        ok,
        proposals: live.len() as u32,
        votes: votes_seen,
        ms: ms(),
        baseline,
    };
    tail.put(LAST_POLL, enc(&status)?);
    tail.delete(POLL_STARTED);
    if ok && baseline {
        tail.put(BASELINED, now.to_be_bytes());
    }
    db.write(tail).map_err(|e| e.to_string())?;
    Ok(status)
}

/// `mnfinalbudget show`: an object keyed "Name (hash)" (rpc/budget.cpp). A
/// malformed entry is a gap, never a silent skip: it may be the winning
/// schedule.
fn finalized_rows(v: &Value, now: u64, gaps: &mut Vec<String>) -> Vec<FinalizedBudget> {
    let Some(obj) = v.as_object() else {
        gaps.push("mnfinalbudget show: not an object".into());
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, e) in obj {
        let u32_of = |f: &str| e.get(f)?.as_i64().and_then(|x| u32::try_from(x).ok());
        let parsed = (|| {
            let (name, rest) = k.rsplit_once(" (")?;
            Some(FinalizedBudget {
                hash: parse_hash(rest.strip_suffix(')')?)?,
                name: name.to_string(),
                block_start: u32_of("BlockStart")?,
                block_end: u32_of("BlockEnd")?,
                proposals: e.get("Proposals")?.as_str()?.to_string(),
                vote_count: u32_of("VoteCount")?,
                status: e.get("Status")?.as_str()?.to_string(),
                is_valid: e.get("IsValid")?.as_bool()?,
                first_seen: now,
                last_seen: now,
            })
        })();
        match parsed {
            Some(f) => out.push(f),
            None => gaps.push(format!("mnfinalbudget show: malformed entry {k}")),
        }
    }
    out
}

/// Poll interval from `governance.poll_secs`. At 3600 s or more, normal-pace
/// vote changes (one per node per hour of signed time) slip between polls. An
/// out-of-range value falls back to 600 rather than stopping collection:
/// history missed while the explorer is down cannot be recovered.
pub fn poll_secs() -> u64 {
    let v = crate::config::try_global_config()
        .and_then(|c| c.get_int("governance.poll_secs").ok())
        .unwrap_or(600);
    if (60..3600).contains(&v) {
        v as u64
    } else {
        error!(
            configured = v,
            "governance.poll_secs must be 60..3600; using 600"
        );
        600
    }
}

/// Max |wall clock - tip block time|. A synced PIVX tip lags real time by
/// ~1 block (26 s measured 2026-10-19). Cost: a chain quiet for over 2 h
/// pauses polling until blocks resume.
const CLOCK_TOLERANCE_SECS: u64 = 7200;

/// The poll's timestamp, or why this tick is skipped. Keys are time-ordered,
/// so a stored future ts would make every later poll invalid until the clock
/// caught up. Skipping loses no votes: the next poll diffs the whole set and
/// the late-poll check marks the hole.
fn poll_ts(wall: u64, last: Option<u64>, tip_time: u64) -> Result<u64, String> {
    if wall.abs_diff(tip_time) > CLOCK_TOLERANCE_SECS {
        return Err(format!(
            "clock {wall} disagrees with tip block time {tip_time}"
        ));
    }
    match last {
        Some(prev) if wall <= prev => Err(format!("clock {wall} not past last poll {prev}")),
        _ => Ok(wall),
    }
}

async fn tip_time(rpc: &Rpc) -> Result<u64, String> {
    let hash = rpc("getbestblockhash", Value::Array(vec![])).await?;
    let header = rpc("getblockheader", Value::Array(vec![hash])).await?;
    header
        .get("time")
        .and_then(|t| t.as_u64())
        .ok_or_else(|| "getblockheader: no time".to_string())
}

/// A gov.db fault stops the poller, never the explorer, and nothing is
/// written after it. ponytail: the error log is the only signal until the
/// read API adds /gov/status; once the poller has run in production for a
/// while, consider exiting the process instead.
fn stopped(e: &str) {
    error!(error = %e, "governance poller STOPPED on a gov.db fault; history is not being recorded until restart");
}

/// Cursor for the next poll's ts. A poll that began but never wrote its status
/// was cut off (crash, kill) with its proposals partly committed: mark the
/// hole, and keep later polls sorting after its rows. Fails closed: a marker
/// or cursor that cannot be read would let a ts repeat.
fn recover_cursor(gov: &DB) -> Result<Option<u64>, String> {
    let interrupted = match gov.get(POLL_STARTED).map_err(|e| e.to_string())? {
        None => None,
        Some(v) => {
            let ts = <[u8; 8]>::try_from(v.as_slice())
                .map(u64::from_be_bytes)
                .map_err(|_| "meta/poll_started is not 8 bytes".to_string())?;
            let gap = PollGap {
                reason: "previous poll interrupted before completion".into(),
            };
            let mut batch = WriteBatch::default();
            batch.put(gap_key(ts), enc(&gap)?);
            batch.delete(POLL_STARTED);
            gov.write(batch).map_err(|e| e.to_string())?;
            Some(ts)
        }
    };
    // Seeded from the stored status so downtime across a restart is a gap too.
    let last = get::<PollStatus>(gov, LAST_POLL)?.map(|s| s.ts);
    Ok(last.max(interrupted))
}

pub async fn run(main_db: Arc<DB>, gov: Arc<DB>, poll_secs: u64) {
    let rpc = |m: &'static str, p: Value| -> RpcFut {
        Box::pin(async move {
            crate::api::helpers::rpc_call_json(m, p)
                .await
                .map_err(|e| e.to_string())
        })
    };
    let mut tick = tokio::time::interval(Duration::from_secs(poll_secs));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    info!(poll_secs, "governance poller started");
    let mut last = match recover_cursor(&gov) {
        Ok(l) => l,
        Err(e) => return stopped(&e),
    };
    loop {
        tick.tick().await;
        let height = crate::chain_state::get_chain_state(&main_db)
            .map(|c| c.height.max(0) as u32)
            .unwrap_or(0);
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let tip = match tip_time(&rpc).await {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "governance poll skipped: tip block time unavailable");
                continue;
            }
        };
        let now = match poll_ts(wall, last, tip) {
            Ok(t) => t,
            Err(e) => {
                error!(reason = %e, "governance poll skipped: clock check failed");
                continue;
            }
        };
        // Skipped ticks (a slow poll or a stalled runtime) leave a hole the
        // history must show.
        if let Some(prev) = last {
            if now.saturating_sub(prev) > poll_secs + poll_secs / 2 {
                let reason = format!("poller late: {} s since previous poll", now - prev);
                if let Err(e) = gov.put(
                    gap_key(prev + poll_secs),
                    enc(&PollGap { reason }).unwrap_or_default(),
                ) {
                    return stopped(e.as_ref());
                }
            }
        }
        let result = poll_once(&gov, &rpc, now, height).await;
        // Advance only when the poll left a durable record; otherwise the next
        // tick still measures from the last recorded poll and marks the hole.
        if result.is_ok() {
            last = Some(now);
        }
        match result {
            Ok(s) if s.ok => info!(
                proposals = s.proposals,
                votes = s.votes,
                ms = s.ms,
                baseline = s.baseline,
                "governance poll"
            ),
            Ok(s) => warn!(
                proposals = s.proposals,
                ms = s.ms,
                "governance poll incomplete; gap recorded"
            ),
            Err(e) => return stopped(&e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn fx(name: &str) -> Value {
        let p = format!("{}/tests/fixtures/gov/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    /// Fixture-backed node. `votes` maps proposal name to its vote array;
    /// unlisted names return []. `fail` makes that method error.
    struct Node {
        info: Value,
        votes: HashMap<String, Value>,
        fail: Option<&'static str>,
        by_name: HashMap<String, Result<Value, String>>,
        sync: Value,
        finalized: Value,
    }

    fn rpc_for(node: Arc<Mutex<Node>>) -> Box<Rpc> {
        Box::new(move |m: &'static str, p: Value| -> RpcFut {
            let node = node.clone();
            Box::pin(async move {
                let n = node.lock().unwrap();
                if n.fail == Some(m) {
                    return Err("connection refused".into());
                }
                let arg = p.get(0).and_then(|x| x.as_str()).map(str::to_string);
                match (m, arg) {
                    ("mnsync", _) => Ok(n.sync.clone()),
                    ("getmasternodecount", _) => Ok(fx("mncount.json")),
                    ("getbudgetprojection", _) => Ok(fx("budgetprojection.json")),
                    ("mnfinalbudget", _) => Ok(n.finalized.clone()),
                    ("getbudgetinfo", None) => Ok(n.info.clone()),
                    ("getbudgetinfo", Some(name)) => {
                        n.by_name.get(&name).cloned().unwrap_or_else(|| {
                            // Core FindProposalByName: highest net among same-named.
                            let net = |r: &&Value| {
                                r["Yeas"].as_i64().unwrap() - r["Nays"].as_i64().unwrap()
                            };
                            n.info
                                .as_array()
                                .unwrap()
                                .iter()
                                .filter(|r| r["Name"] == name.as_str())
                                .rev()
                                .max_by_key(net)
                                .map(|r| Value::Array(vec![r.clone()]))
                                .ok_or_else(|| "Unknown proposal name".to_string())
                        })
                    }
                    ("getbudgetvotes", Some(name)) => {
                        Ok(n.votes.get(&name).cloned().unwrap_or(Value::Array(vec![])))
                    }
                    _ => Err(format!("unexpected {m}")),
                }
            })
        })
    }

    fn node() -> Arc<Mutex<Node>> {
        let mut votes = HashMap::new();
        votes.insert(
            "MrAliMMFund01".to_string(),
            fx("budgetvotes_MrAliMMFund01.json"),
        );
        // Proposals without a vote fixture return no votes, so their listed
        // tallies must be zero to stay consistent with a real node.
        let mut info = fx("budgetinfo.json");
        for r in info.as_array_mut().unwrap() {
            if r["Name"] != "MrAliMMFund01" {
                r["Yeas"] = 0.into();
                r["Nays"] = 0.into();
                r["Abstains"] = 0.into();
            }
        }
        Arc::new(Mutex::new(Node {
            info,
            votes,
            fail: None,
            by_name: HashMap::new(),
            sync: serde_json::json!({"IsBlockchainSynced": true, "RequestedMasternodeAssets": 999}),
            finalized: serde_json::json!({}),
        }))
    }

    fn changes(db: &DB) -> Vec<VoteChange> {
        scan(db, b"c/")
            .unwrap()
            .iter()
            .map(|(_, v)| bincode::deserialize(v).unwrap())
            .collect()
    }

    fn mrali_hash() -> Hash {
        let info = fx("budgetinfo.json");
        let r = info
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["Name"] == "MrAliMMFund01")
            .unwrap();
        parse_hash(r["Hash"].as_str().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn baseline_then_quiet_then_change() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        let rpc = rpc_for(n.clone());

        let s1 = poll_once(&db, &*rpc, 1_000, 10).await.unwrap();
        assert!(s1.ok && s1.baseline);
        assert_eq!((s1.proposals, s1.votes), (13, 999));
        assert!(
            db.get(POLL_STARTED).unwrap().is_none(),
            "marker cleared by status write"
        );
        assert_eq!(changes(&db).len(), 999);
        assert!(changes(&db).iter().all(|c| c.kind == VoteChangeKind::New));
        assert_eq!(scan(&db, b"p/").unwrap().len(), 13);

        // Same node state: nothing new is logged, no new totals points.
        let s2 = poll_once(&db, &*rpc, 1_600, 11).await.unwrap();
        assert!(s2.ok && !s2.baseline);
        assert_eq!(changes(&db).len(), 999);
        assert_eq!(scan(&db, b"t/").unwrap().len(), 13);

        // One NO voter flips to YES with a new signature.
        {
            let mut g = n.lock().unwrap();
            let arr = g
                .votes
                .get_mut("MrAliMMFund01")
                .unwrap()
                .as_array_mut()
                .unwrap();
            let row = arr.iter_mut().find(|r| r["Vote"] == "NO").unwrap();
            row["Vote"] = "YES".into();
            row["nHash"] = "ab".repeat(32).into();
            let info = g.info.as_array_mut().unwrap();
            let p = info
                .iter_mut()
                .find(|r| r["Name"] == "MrAliMMFund01")
                .unwrap();
            p["Yeas"] = (p["Yeas"].as_u64().unwrap() + 1).into();
            p["Nays"] = (p["Nays"].as_u64().unwrap() - 1).into();
        }
        poll_once(&db, &*rpc, 2_200, 12).await.unwrap();
        let c = changes(&db);
        assert_eq!(c.len(), 1000);
        let last = c
            .iter()
            .find(|x| x.kind == VoteChangeKind::Changed)
            .unwrap();
        assert_eq!((last.from, last.to), (Some(2), Some(1)));
        let h = mrali_hash();
        assert_eq!(scan(&db, &vote_prefix(&h)).unwrap().len(), 999);
    }

    #[tokio::test]
    async fn restart_continues_without_duplicates() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let rpc = rpc_for(node());
        {
            let db = open(&path).unwrap();
            poll_once(&db, &*rpc, 1_000, 10).await.unwrap();
        }
        let db = open(&path).unwrap();
        let s = poll_once(&db, &*rpc, 1_600, 11).await.unwrap();
        assert!(s.ok && !s.baseline);
        assert_eq!(changes(&db).len(), 999);
    }

    #[tokio::test]
    async fn global_rpc_failure_writes_gap_and_nothing_else() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        n.lock().unwrap().fail = Some("getbudgetinfo");
        let s = poll_once(&db, &*rpc_for(n.clone()), 1_000, 10)
            .await
            .unwrap();
        assert!(!s.ok);
        assert_eq!(scan(&db, b"g/").unwrap().len(), 1);
        assert!(scan(&db, b"p/").unwrap().is_empty() && scan(&db, b"v/").unwrap().is_empty());

        // A failed first poll must not consume the baseline.
        n.lock().unwrap().fail = None;
        let s = poll_once(&db, &*rpc_for(n), 1_600, 11).await.unwrap();
        assert!(s.ok && s.baseline);
    }

    #[tokio::test]
    async fn votes_failure_keeps_stored_votes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        poll_once(&db, &*rpc_for(n.clone()), 1_000, 10)
            .await
            .unwrap();
        let before: TotalsPoint = get(&db, &latest_totals_key(&mrali_hash()))
            .unwrap()
            .unwrap();
        {
            let mut g = n.lock().unwrap();
            g.fail = Some("getbudgetvotes");
            let info = g.info.as_array_mut().unwrap();
            let p = info
                .iter_mut()
                .find(|r| r["Name"] == "MrAliMMFund01")
                .unwrap();
            p["Yeas"] = (p["Yeas"].as_u64().unwrap() + 1).into();
        }
        let s = poll_once(&db, &*rpc_for(n), 1_600, 11).await.unwrap();
        assert!(!s.ok);
        assert_eq!(scan(&db, &vote_prefix(&mrali_hash())).unwrap().len(), 999);
        assert_eq!(
            changes(&db).len(),
            999,
            "no Removed rows from a failed call"
        );
        let after: TotalsPoint = get(&db, &latest_totals_key(&mrali_hash()))
            .unwrap()
            .unwrap();
        assert_eq!(before, after, "totals not committed without their votes");
    }

    #[tokio::test]
    async fn invalid_then_gone_lifecycle() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        poll_once(&db, &*rpc_for(n.clone()), 1_000, 10)
            .await
            .unwrap();
        let h = mrali_hash();

        // Dropped from the no-arg list but still known by name: went invalid.
        let full = n.lock().unwrap().info.clone();
        let (keep, gone): (Vec<_>, Vec<_>) = full
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .partition(|r| r["Name"] != "MrAliMMFund01");
        let mut invalid = gone[0].clone();
        invalid["IsValid"] = false.into();
        {
            let mut g = n.lock().unwrap();
            g.info = Value::Array(keep);
            g.by_name
                .insert("MrAliMMFund01".into(), Ok(Value::Array(vec![invalid])));
        }
        poll_once(&db, &*rpc_for(n.clone()), 1_600, 11)
            .await
            .unwrap();
        let t: TotalsPoint = get(&db, &latest_totals_key(&h)).unwrap().unwrap();
        assert!(!t.is_valid);
        let m: ProposalMeta = get(&db, &proposal_key(&h)).unwrap().unwrap();
        assert_eq!(m.gone_at, None);

        // Node forgets it entirely.
        n.lock().unwrap().by_name.clear();
        poll_once(&db, &*rpc_for(n), 2_200, 12).await.unwrap();
        let m: ProposalMeta = get(&db, &proposal_key(&h)).unwrap().unwrap();
        assert_eq!(m.gone_at, Some(2_200));
        assert_eq!(
            scan(&db, &vote_prefix(&h)).unwrap().len(),
            999,
            "history kept after gone"
        );
    }

    #[tokio::test]
    async fn shared_name_votes_go_only_to_the_resolved_proposal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        {
            let mut g = n.lock().unwrap();
            let mut dup = g
                .info
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["Name"] == "MrAliMMFund01")
                .unwrap()
                .clone();
            dup["Hash"] = "cd".repeat(32).into();
            g.info.as_array_mut().unwrap().push(dup);
        }
        poll_once(&db, &*rpc_for(n), 1_000, 10).await.unwrap();
        // Core resolves the shared name to one of them; only that one gets votes.
        let winner: ProposalMeta = get(&db, &proposal_key(&mrali_hash())).unwrap().unwrap();
        assert_eq!(winner.visibility, Visibility::Readable);
        let dup: ProposalMeta = get(&db, &proposal_key(&[0xcd; 32])).unwrap().unwrap();
        assert_eq!(dup.visibility, Visibility::Shadowed);
        assert_eq!(scan(&db, &vote_prefix(&mrali_hash())).unwrap().len(), 999);
        assert!(scan(&db, &vote_prefix(&[0xcd; 32])).unwrap().is_empty());
    }

    #[tokio::test]
    async fn unsynced_node_writes_gap_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        n.lock().unwrap().sync =
            serde_json::json!({"IsBlockchainSynced": true, "RequestedMasternodeAssets": 3});
        let s = poll_once(&db, &*rpc_for(n), 1_000, 10).await.unwrap();
        assert!(!s.ok);
        assert_eq!(scan(&db, b"g/").unwrap().len(), 1);
        assert!(scan(&db, b"p/").unwrap().is_empty() && scan(&db, b"c/").unwrap().is_empty());
    }

    #[tokio::test]
    async fn votes_not_matching_tallies_are_not_written() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        {
            // Listed tallies disagree with the returned votes (another proposal's
            // votes under the same name, or a vote landing between the calls).
            let mut g = n.lock().unwrap();
            let info = g.info.as_array_mut().unwrap();
            let p = info
                .iter_mut()
                .find(|r| r["Name"] == "MrAliMMFund01")
                .unwrap();
            p["Yeas"] = 5.into();
        }
        let s = poll_once(&db, &*rpc_for(n), 1_000, 10).await.unwrap();
        assert!(!s.ok);
        assert!(scan(&db, &vote_prefix(&mrali_hash())).unwrap().is_empty());
        assert!(scan(&db, b"c/").unwrap().is_empty());
        assert_eq!(scan(&db, b"g/").unwrap().len(), 1);
    }

    #[tokio::test]
    async fn owner_flip_during_read_writes_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        let calls = Arc::new(Mutex::new(0u32));
        let inner = rpc_for(n);
        let c2 = calls.clone();
        // Second by-name probe for MrAli resolves to a different hash.
        let rpc: Box<Rpc> = Box::new(move |m: &'static str, p: Value| -> RpcFut {
            let is_probe =
                m == "getbudgetinfo" && p.get(0).and_then(|x| x.as_str()) == Some("MrAliMMFund01");
            if is_probe {
                let mut k = c2.lock().unwrap();
                *k += 1;
                if *k == 2 {
                    let mut row = fx("budgetinfo.json")
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|r| r["Name"] == "MrAliMMFund01")
                        .unwrap()
                        .clone();
                    row["Hash"] = "ef".repeat(32).into();
                    return Box::pin(async move { Ok(Value::Array(vec![row])) });
                }
            }
            inner(m, p)
        });
        let s = poll_once(&db, &*rpc, 1_000, 10).await.unwrap();
        assert!(!s.ok);
        assert!(scan(&db, &vote_prefix(&mrali_hash())).unwrap().is_empty());
    }

    #[tokio::test]
    async fn malformed_projection_fails_the_poll() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let inner = rpc_for(node());
        let rpc: Box<Rpc> = Box::new(move |m: &'static str, p: Value| -> RpcFut {
            if m == "getbudgetprojection" {
                return Box::pin(async { Ok(serde_json::json!([{"Name": "x"}])) });
            }
            inner(m, p)
        });
        let s = poll_once(&db, &*rpc, 1_000, 10).await.unwrap();
        assert!(!s.ok);
        assert!(scan(&db, b"p/").unwrap().is_empty() && scan(&db, b"t/").unwrap().is_empty());
    }

    #[tokio::test]
    async fn malformed_list_row_fails_the_poll() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        poll_once(&db, &*rpc_for(n.clone()), 1_000, 10)
            .await
            .unwrap();
        {
            let mut g = n.lock().unwrap();
            let r = g
                .info
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|r| r["Name"] == "MrAliMMFund01")
                .unwrap();
            r.as_object_mut().unwrap().remove("Yeas");
        }
        let s = poll_once(&db, &*rpc_for(n), 1_600, 11).await.unwrap();
        assert!(!s.ok);
        let m: ProposalMeta = get(&db, &proposal_key(&mrali_hash())).unwrap().unwrap();
        assert_eq!(
            m.gone_at, None,
            "a malformed row must not mark the proposal gone"
        );
        assert_eq!(m.last_seen, 1_000);
    }

    #[tokio::test]
    async fn unresolvable_name_records_list_data_without_gap() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        // By-name lookup for MrAli fails as Core does for a sanitized-away name.
        n.lock()
            .unwrap()
            .by_name
            .insert("MrAliMMFund01".into(), Err("Unknown proposal name".into()));
        let s = poll_once(&db, &*rpc_for(n), 1_000, 10).await.unwrap();
        assert!(
            s.ok && s.baseline,
            "an unreachable name must not fail every poll"
        );
        let m: ProposalMeta = get(&db, &proposal_key(&mrali_hash())).unwrap().unwrap();
        assert_eq!(m.visibility, Visibility::Shadowed);
        assert_eq!(m.votes_since, None);
        assert!(get::<TotalsPoint>(&db, &latest_totals_key(&mrali_hash()))
            .unwrap()
            .is_some());
        assert!(scan(&db, &vote_prefix(&mrali_hash())).unwrap().is_empty());
    }

    #[tokio::test]
    async fn duplicate_hash_in_list_fails_the_poll() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        {
            let mut g = n.lock().unwrap();
            let first = g.info.as_array().unwrap()[0].clone();
            g.info.as_array_mut().unwrap().push(first);
        }
        let s = poll_once(&db, &*rpc_for(n), 1_000, 10).await.unwrap();
        assert!(!s.ok);
        assert!(scan(&db, b"p/").unwrap().is_empty());
    }

    #[test]
    fn poll_timestamps_never_go_backwards_or_get_invented() {
        assert_eq!(poll_ts(10_000, None, 9_970), Ok(10_000));
        assert_eq!(poll_ts(10_000, Some(9_400), 9_970), Ok(10_000));
        // Same-second restart or a step back: skip, never last + 1.
        assert!(poll_ts(10_000, Some(10_000), 9_970).is_err());
        assert!(poll_ts(9_000, Some(10_000), 9_000).is_err());
        // A clock far from the chain must not store a ts at all.
        assert!(poll_ts(10_000 + 365 * 86_400, None, 10_000).is_err());
        assert!(poll_ts(10_000, None, 10_000 + 7_201).is_err());
        assert_eq!(poll_ts(10_000, None, 10_000 + 7_200), Ok(10_000));
    }

    #[test]
    fn interrupted_poll_is_marked_and_bad_marker_fails_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        db.put(POLL_STARTED, 900u64.to_be_bytes()).unwrap();
        assert_eq!(recover_cursor(&db), Ok(Some(900)));
        assert!(db.get(POLL_STARTED).unwrap().is_none());
        assert!(db.get(gap_key(900)).unwrap().is_some());
        db.put(POLL_STARTED, [1u8, 2, 3]).unwrap();
        assert!(recover_cursor(&db).is_err());
    }

    #[tokio::test]
    async fn failed_poll_also_clears_the_start_marker() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        n.lock().unwrap().fail = Some("getbudgetinfo");
        poll_once(&db, &*rpc_for(n), 1_000, 10).await.unwrap();
        assert!(db.get(POLL_STARTED).unwrap().is_none());
    }

    fn vis_log(db: &DB, h: &Hash) -> Vec<(u64, Visibility)> {
        let p = [b"s/".as_slice(), h].concat();
        scan(db, &p)
            .unwrap()
            .into_iter()
            .map(|(k, v)| {
                let ts = u64::from_be_bytes(k[p.len()..].try_into().unwrap());
                (ts, bincode::deserialize(&v).unwrap())
            })
            .collect()
    }

    #[tokio::test]
    async fn proposal_first_seen_after_baseline_gets_its_own_baseline() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        let full = n.lock().unwrap().info.clone();
        n.lock()
            .unwrap()
            .info
            .as_array_mut()
            .unwrap()
            .retain(|r| r["Name"] != "MrAliMMFund01");
        assert!(
            poll_once(&db, &*rpc_for(n.clone()), 1_000, 10)
                .await
                .unwrap()
                .baseline
        );
        n.lock().unwrap().info = full;
        let s = poll_once(&db, &*rpc_for(n), 1_600, 11).await.unwrap();
        assert!(s.ok && !s.baseline);
        let m: ProposalMeta = get(&db, &proposal_key(&mrali_hash())).unwrap().unwrap();
        // Its 999 New rows sit after the global baseline; votes_since marks
        // them as first sightings.
        assert_eq!(m.votes_since, Some(1_600));
        assert_eq!(changes(&db).len(), 999);
    }

    #[tokio::test]
    async fn unlisted_proposal_owned_by_another_is_unresolvable_not_live() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        poll_once(&db, &*rpc_for(n.clone()), 1_000, 10)
            .await
            .unwrap();
        let h = mrali_hash();
        assert_eq!(vis_log(&db, &h), vec![(1_000, Visibility::Readable)]);
        // MrAli drops out of the list; a same-named proposal now owns the name.
        for r in n.lock().unwrap().info.as_array_mut().unwrap() {
            if r["Name"] == "MrAliMMFund01" {
                r["Hash"] = "cd".repeat(32).into();
            }
        }
        poll_once(&db, &*rpc_for(n.clone()), 1_600, 11)
            .await
            .unwrap();
        poll_once(&db, &*rpc_for(n), 2_200, 12).await.unwrap();
        let m: ProposalMeta = get(&db, &proposal_key(&h)).unwrap().unwrap();
        assert_eq!(m.visibility, Visibility::Unresolvable);
        assert_eq!(m.gone_at, None);
        assert_eq!(m.last_seen, 1_000, "not seen since, so not refreshed");
        assert_eq!(
            vis_log(&db, &h),
            vec![
                (1_000, Visibility::Readable),
                (1_600, Visibility::Unresolvable)
            ],
            "logged once, on the transition"
        );
        assert_eq!(
            scan(&db, &vote_prefix(&h)).unwrap().len(),
            999,
            "last known votes kept"
        );
    }

    #[tokio::test]
    async fn finalized_budgets_are_kept_and_malformed_entries_are_gaps() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let n = node();
        // Shape of rpc/budget.cpp mnfinalbudget "show".
        let key = format!("main (dirty) ({})", "ab".repeat(32));
        n.lock().unwrap().finalized = serde_json::json!({ key.clone(): {
            "FeeTX": "ef".repeat(32), "BlockStart": 5_659_200, "BlockEnd": 5_659_203,
            "Proposals": "A, B, C, D", "VoteCount": 1500, "Status": "OK", "IsValid": true
        }});
        assert!(
            poll_once(&db, &*rpc_for(n.clone()), 1_000, 10)
                .await
                .unwrap()
                .ok
        );
        poll_once(&db, &*rpc_for(n.clone()), 1_600, 11)
            .await
            .unwrap();
        let f: FinalizedBudget = get(&db, &finalized_key(&[0xab; 32])).unwrap().unwrap();
        assert_eq!(f.name, "main (dirty)");
        assert_eq!((f.block_start, f.block_end), (5_659_200, 5_659_203));
        assert_eq!(f.proposals, "A, B, C, D");
        assert_eq!((f.first_seen, f.last_seen), (1_000, 1_600));
        let fv = |db: &DB| {
            scan(db, &[b"fv/".as_slice(), &[0xab; 32]].concat())
                .unwrap()
                .len()
        };
        assert_eq!(fv(&db), 1, "unchanged count logs once");
        n.lock().unwrap().finalized[key.clone()]["VoteCount"] = 1501.into();
        poll_once(&db, &*rpc_for(n.clone()), 1_900, 11)
            .await
            .unwrap();
        assert_eq!(fv(&db), 2, "a lead change must survive later polls");
        n.lock().unwrap().finalized[key]["VoteCount"] = "many".into();
        let s = poll_once(&db, &*rpc_for(n), 2_200, 12).await.unwrap();
        assert!(!s.ok, "a malformed schedule entry must be a gap");
        let f: FinalizedBudget = get(&db, &finalized_key(&[0xab; 32])).unwrap().unwrap();
        assert_eq!(f.last_seen, 1_900);
    }

    /// Two polls against the node in config.toml. Run before a deploy:
    /// cargo test --lib gov::poller::tests::live_node -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "needs a synced pivxd at rpc.host"]
    async fn live_node_two_polls() {
        // Another ignored test may have set the OnceCell already.
        let _ = crate::config::init_global_config();
        let rpc = |m: &'static str, p: Value| -> RpcFut {
            Box::pin(async move {
                crate::api::helpers::rpc_call_json(m, p)
                    .await
                    .map_err(|e| e.to_string())
            })
        };
        let height = rpc("getblockcount", Value::Array(vec![])).await.unwrap();
        let height = height.as_u64().unwrap() as u32;
        let tmp = tempfile::TempDir::new().unwrap();
        let db = crate::gov::store::open(tmp.path().to_str().unwrap()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let tip = tip_time(&rpc).await.unwrap();
        assert_eq!(poll_ts(now, None, tip), Ok(now), "live clock check");
        let a = poll_once(&db, &rpc, now, height).await.unwrap();
        let changes_a = scan(&db, b"c/").unwrap().len();
        let gaps_a = scan(&db, b"g/").unwrap();
        let b = poll_once(&db, &rpc, now + 1, height).await.unwrap();
        let changes_b = scan(&db, b"c/").unwrap().len() - changes_a;
        for (k, v) in scan(&db, b"g/").unwrap() {
            let g: PollGap = bincode::deserialize(&v).unwrap();
            eprintln!(
                "gap {}: {}",
                u64::from_be_bytes(k[2..10].try_into().unwrap()),
                g.reason
            );
        }
        eprintln!("poll 1 {a:?} changes={changes_a} gaps={}", gaps_a.len());
        eprintln!("poll 2 {b:?} new_changes={changes_b}");
        assert!(
            a.ok && a.baseline && a.votes > 0,
            "first poll must baseline"
        );
        assert!(b.ok && !b.baseline, "second poll must succeed");
    }
}
