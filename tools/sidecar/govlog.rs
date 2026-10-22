//! rustyblox-govlog: follows pivxd debug.log (-debug=mnbudget) into govlog.db.
//! Runs beside pivxd as the user that can read debug.log; the explorer never
//! opens this store, so relay data stays on the node host.

use clap::{Parser, Subcommand};
use rustyblox::govlog::{self, Class, Outcome};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(about = "Budget-vote log sidecar for pivxd")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Follow debug.log and store vote events.
    Run {
        #[arg(long)]
        log: String,
        #[arg(long)]
        db: String,
        /// Raw events older than this are pruned; gaps are kept.
        #[arg(long, default_value_t = 90)]
        retention_days: u64,
        #[arg(long, default_value_t = 1000)]
        poll_ms: u64,
        /// pivxd's P2P address (not RPC), e.g. 127.0.0.1:51472. When set,
        /// votes rejected as older or too soon are read back from the node
        /// with getdata so their masternode and proposal are known.
        #[arg(long)]
        p2p: Option<String>,
    },
    /// Summarise stored events (secondary instance; safe while `run` is live).
    Report {
        #[arg(long)]
        db: String,
        #[arg(long, default_value_t = 24)]
        hours: u64,
        #[arg(long, default_value_t = 20)]
        recent: usize,
    },
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn outcome_name(o: &Option<Outcome>) -> String {
    match o {
        None => "accepted (no AddOrUpdateVote line)".into(),
        Some(Outcome::Inserted) => "accepted: new".into(),
        Some(Outcome::Updated) => "accepted: updated".into(),
        Some(Outcome::RejectOlder) => "rejected: older than stored vote".into(),
        Some(Outcome::RejectTooSoon(_)) => "rejected: under 1 h after stored vote".into(),
        Some(Outcome::RejectFuture(_)) => "rejected: over 1 h in the future".into(),
        Some(Outcome::Removed(r)) => format!("proposal removed: {r}"),
    }
}

fn is_reject(o: &Option<Outcome>) -> bool {
    o.as_ref().is_some_and(Outcome::is_reject)
}

/// Seconds between recovery passes, and the backoff after a failed connect.
const RECOVER_EVERY: u64 = 10;
const RECOVER_BACKOFF: u64 = 60;
/// Rejections older than this are no longer asked for: Core's ReloadMapSeen
/// has very likely dropped them, and asking again costs a connection.
const RECOVER_WINDOW: u64 = 86_400;
/// Core answers every miss with notfound (net_processing.cpp:1196), so a hash
/// left unanswered means a cut-short read; after this many passes it is
/// recorded as not found rather than asked for all day.
const RECOVER_TRIES: u8 = 3;

fn run(
    log: &str,
    db_path: &str,
    retention_days: u64,
    poll_ms: u64,
    p2p: Option<&str>,
) -> Result<(), String> {
    let db = govlog::open(db_path)?;
    let mut tail = govlog::Tail::new(&db, log)?;
    tracing::info!(log, db = db_path, p2p, "govlog sidecar started");
    let mut last_prune = 0;
    let mut next_recover = 0;
    let mut failing = false;
    let mut unanswered: std::collections::HashMap<govlog::Hash, u8> = Default::default();
    loop {
        // A storage or read error stops the sidecar loudly; systemd restarts
        // it and the gap logic records whatever the restart missed.
        let n = tail.poll(&db)?;
        if n > 0 {
            tracing::debug!(lines = n, "ingested");
        }
        if let Some(addr) = p2p.filter(|_| now() >= next_recover) {
            next_recover = now() + RECOVER_EVERY;
            let pending = govlog::unrecovered(&db, now().saturating_sub(RECOVER_WINDOW))?;
            if !pending.is_empty() {
                // One request and one retry count per distinct hash; the
                // answer fans out to every event carrying it.
                let mut hashes: Vec<govlog::Hash> = pending.iter().map(|(_, h)| *h).collect();
                hashes.sort_unstable();
                hashes.dedup();
                match rustyblox::gov::p2pvote::fetch(addr, &hashes, Duration::from_secs(10)) {
                    Ok(got) => {
                        failing = false;
                        let at = now();
                        let mut settled: std::collections::HashMap<govlog::Hash, Option<_>> =
                            Default::default();
                        for h in &hashes {
                            match got.get(h) {
                                Some(v) => {
                                    unanswered.remove(h);
                                    settled.insert(*h, v.clone());
                                }
                                None => {
                                    let n = unanswered.entry(*h).or_default();
                                    *n += 1;
                                    if *n >= RECOVER_TRIES {
                                        unanswered.remove(h);
                                        settled.insert(*h, None);
                                    }
                                }
                            }
                        }
                        let rows: Vec<_> = pending
                            .iter()
                            .filter_map(|(ts, h)| {
                                let vote = settled.get(h)?.clone();
                                Some((
                                    *ts,
                                    *h,
                                    govlog::Recovered {
                                        fetched_at: at,
                                        vote,
                                    },
                                ))
                            })
                            .collect();
                        govlog::put_recovered(&db, &rows)?;
                        let found = rows.iter().filter(|r| r.2.vote.is_some()).count();
                        tracing::info!(
                            asked = hashes.len(),
                            found,
                            notfound = rows.len() - found,
                            "recovered rejected votes"
                        );
                    }
                    Err(e) => {
                        // Logged once per outage; the node being down must not
                        // fill the journal every ten seconds.
                        if !failing {
                            tracing::warn!(error = %e, addr, "vote recovery: node unreachable over P2P");
                        }
                        failing = true;
                        next_recover = now() + RECOVER_BACKOFF;
                    }
                }
            }
        }
        if now().saturating_sub(last_prune) > 3600 {
            let pruned = govlog::prune(&db, now().saturating_sub(retention_days * 86_400))?;
            if pruned > 0 {
                tracing::info!(pruned, "pruned events past retention");
            }
            last_prune = now();
        }
        std::thread::sleep(Duration::from_millis(poll_ms));
    }
}

fn report(db_path: &str, hours: u64, recent: usize) -> Result<(), String> {
    // A secondary instance needs a directory of its own; this one is scratch.
    let scratch = std::env::temp_dir().join(format!("govlog-report-{}", std::process::id()));
    let db = govlog::open_secondary(db_path, &scratch.to_string_lossy())?;
    let res = print_report(&db, hours, recent);
    drop(db);
    let _ = std::fs::remove_dir_all(&scratch);
    res
}

fn print_report(db: &rocksdb::DB, hours: u64, recent: usize) -> Result<(), String> {
    let from = now().saturating_sub(hours * 3600);
    let ev = govlog::events(db, from, u64::MAX)?;
    let other = govlog::others(db, from, u64::MAX)?;
    println!("votes in the last {hours} h: {}", ev.len());
    let mut by_kind: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut rejects_by_mn: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_object: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for e in &ev {
        *by_kind
            .entry((format!("{:?}", e.class), outcome_name(&e.outcome)))
            .or_default() += 1;
        if is_reject(&e.outcome) {
            *rejects_by_mn
                .entry(e.mn.clone().unwrap_or_else(|| "(not logged)".into()))
                .or_default() += 1;
        }
        if let Some(o) = e.object {
            let c = by_object.entry(hex::encode(o)).or_default();
            if is_reject(&e.outcome) {
                c.1 += 1
            } else {
                c.0 += 1
            }
        }
    }
    println!("\nby kind:");
    for ((class, out), n) in &by_kind {
        println!("  {n:>7}  {class:<16} {out}");
    }
    let mut mns: Vec<_> = rejects_by_mn.into_iter().collect();
    mns.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\nrejected votes by masternode (top 15; legacy rejections name none):");
    for (mn, n) in mns.iter().take(15) {
        println!("  {n:>7}  {mn}");
    }
    println!("\nby proposal/budget (accepted, rejected):");
    for (o, (a, r)) in &by_object {
        println!("  {a:>7} {r:>7}  {o}");
    }
    let mut texts: BTreeMap<&str, u32> = BTreeMap::new();
    for o in &other {
        *texts.entry(o.text.as_str()).or_default() += o.seen;
    }
    println!("\nother rejections (no vote hash), by message:");
    for (t, n) in &texts {
        println!("  {n:>7}  {t}");
    }
    println!("\nlatest {recent} rejected votes:");
    for e in ev
        .iter()
        .rev()
        .filter(|e| is_reject(&e.outcome))
        .take(recent)
    {
        let detail = match &e.outcome {
            Some(Outcome::RejectTooSoon(d)) => format!(" gap={d}s"),
            Some(Outcome::RejectFuture(t)) => format!(" nTime={t}"),
            _ => String::new(),
        };
        println!(
            "  {} {:<10} {} vote={} mn={} seen={}{}",
            e.ts,
            format!("{:?}", e.class),
            outcome_name(&e.outcome),
            hex::encode(e.vote),
            e.mn.as_deref().unwrap_or("-"),
            e.seen,
            detail
        );
    }
    let removed = ev
        .iter()
        .filter(|e| e.class == Class::ProposalRemoved)
        .count();
    if removed > 0 {
        println!("\nproposals removed by the node in window: {removed}");
    }
    let gaps = govlog::gaps(db)?;
    println!("\ncoverage gaps (all time): {}", gaps.len());
    for (from, until, g) in gaps.iter().rev().take(10) {
        println!("  {from}..{until} {}", g.reason);
    }
    Ok(())
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let res = match Cli::parse().cmd {
        Cmd::Run {
            log,
            db,
            retention_days,
            poll_ms,
            p2p,
        } => run(&log, &db, retention_days, poll_ms, p2p.as_deref()),
        Cmd::Report { db, hours, recent } => report(&db, hours, recent),
    };
    if let Err(e) = res {
        tracing::error!(error = %e, "govlog sidecar failed");
        std::process::exit(1);
    }
}
