// Sidecar store for pivxd budget-vote log lines (-debug=mnbudget). getbudgetvotes
// shows only the votes a node kept; arrival times and rejected votes exist only
// in debug.log, which logrotate cuts daily on prod (copytruncate). A separate
// process and RocksDB keep this away from the public explorer.
//
//   e/T/V      Event        T = unix secs u64 BE, V = 32-byte vote (or proposal) hash
//   o/T/D      OtherReject  rejection with no vote hash; D = sha256 of its text
//   g/F/U      Gap          log coverage possibly lost between F and U
//   meta/cursor  Cursor     file identity + offset through the last full line
//   meta/schema  u32 BE
//
// Line formats are PIVX Core bd8a672 (budgetproposal.cpp:225-250,
// finalizedbudget.cpp:54-78, budgetmanager.cpp:383-397,1089-1176,1325-1331).
// Mainnet runs legacy masternodes (UPGRADE_V6_0 not active), so accepted votes
// log "from mn COutPoint(<10 hex>, n)" and legacy rejections name no masternode.

use rocksdb::{Direction, IteratorMode, WriteBatch, DB};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const SCHEMA: &[u8] = b"meta/schema";
pub const SCHEMA_VERSION: u32 = 1;
pub const CURSOR: &[u8] = b"meta/cursor";
/// u64 BE unix time the sidecar first opened this DB. Votes before it were
/// never watched, so coverage starts here, not at the first logged event.
pub const SINCE: &[u8] = b"meta/since";

pub type Hash = [u8; 32];

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// AddOrUpdateVote lines are worded the same for proposal and finalized-
    /// budget votes; only the mvote/fbvote/bad-* line names the type. A vote
    /// that stays Unknown came in through local RPC, which logs no mvote line.
    Unknown,
    Proposal,
    Finalized,
    /// CheckAndRemove dropped the proposal; `vote` holds its hash.
    ProposalRemoved,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Inserted,
    Updated,
    /// Signed time older than the stored vote.
    RejectOlder,
    /// Under 3600 s after the stored vote; the gap in seconds.
    RejectTooSoon(i64),
    /// Signed time over 1 h ahead of the node clock; the vote's nTime.
    RejectFuture(i64),
    /// CheckAndRemove reason, e.g. "Heavily Downvoted".
    Removed(String),
}

impl Outcome {
    pub fn is_reject(&self) -> bool {
        matches!(
            self,
            Outcome::RejectOlder | Outcome::RejectTooSoon(_) | Outcome::RejectFuture(_)
        )
    }
}

/// One vote (or removal) seen in the log. An accepted vote takes two lines
/// (AddOrUpdateVote, then mvote/fbvote) and a rejection up to two; each line
/// gets its own timestamp, so lines for one hash within a second of each
/// other merge into one record.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Event {
    pub ts: u64,
    pub vote: Hash,
    pub class: Class,
    pub outcome: Option<Outcome>,
    /// Proposal or finalized-budget hash, from the mvote/fbvote line.
    pub object: Option<Hash>,
    /// Masternode as Core prints it: "COutPoint(<10 hex>, n)" for legacy
    /// masternodes, the ProTx hash for deterministic ones.
    pub mn: Option<String>,
    /// Network messages logged for this vote (mvote/fbvote/ProcessMessageInner
    /// lines). Core logs older/too-soon rejections once per node lifetime and
    /// future-time rejections on every receipt (budgetmanager.cpp:1093-1103).
    pub seen: u32,
    /// Which line kinds formed this record (LINE_* bits). Records for one
    /// hash a second apart merge only when their line kinds do not overlap.
    pub lines: u8,
}

/// mvote/fbvote/ProcessMessageInner: one network message each.
pub const LINE_NET: u8 = 1;
/// AddOrUpdateVote: restates a vote handled by a network or RPC path.
pub const LINE_UPDATE: u8 = 2;
/// "Unable to add orphan vote ...": wraps the AddOrUpdateVote reason for a
/// vote that arrived before its proposal (budgetmanager.cpp:63-64,85-86).
pub const LINE_ORPHAN: u8 = 4;
pub const LINE_REMOVAL: u8 = 8;

impl Event {
    fn merge(&mut self, o: Event) {
        if self.class == Class::Unknown {
            self.class = o.class;
        }
        if self.outcome.is_none() {
            self.outcome = o.outcome;
        }
        self.object = self.object.or(o.object);
        if self.mn.is_none() {
            self.mn = o.mn;
        }
        self.seen += o.seen;
        self.lines |= o.lines;
    }
}

/// A bad-mvote/bad-fbvote line with no vote hash in it ("masternode not
/// valid", unknown masternode, invalid signature). Kept by its text, never
/// under a hash that only looks like a vote.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OtherReject {
    pub ts: u64,
    pub class: Class,
    pub text: String,
    pub mn: Option<String>,
    pub seen: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Gap {
    pub reason: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Cursor {
    pub inode: u64,
    /// Byte offset through the last complete line; a partial line is re-read.
    pub offset: u64,
    /// Timestamp of the last timestamped line read (any line, not only votes).
    pub last_ts: u64,
    /// First bytes of the file. A copytruncate that refills the file past the
    /// old offset keeps inode and size plausible; changed head bytes give it away.
    pub head: Vec<u8>,
    /// Rotated-away files still to be finished, oldest first, from the
    /// rotated copies (<log>.N or <log>.N.gz). Persisted, so a restart
    /// mid-rotation still finishes them. The current file is not read while
    /// any is pending, so events stay in log order.
    pub drains: Vec<Drain>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Drain {
    pub offset: u64,
    pub head: Vec<u8>,
    pub from_ts: u64,
    /// Unix secs the rotation was seen; copies are searched until DRAIN_WAIT.
    pub since: u64,
    pub copytruncate: bool,
}

/// What one physical log line contributes.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Event(Event),
    Other(OtherReject),
    /// The " (<protx>)" tail Core prints on its own line after a deterministic-
    /// MN rejection: the reason ends in "\n" and the id follows in the same
    /// LogPrint call, so it gets no timestamp (budgetmanager.cpp:1126,1235).
    Continuation(String),
    /// Anything else; carries its timestamp if it has one.
    Plain(Option<u64>),
}

fn hash(s: &str) -> Option<Hash> {
    let s = s.get(..64)?;
    hex::decode(s).ok()?.try_into().ok()
}

/// "2026-10-20T00:11:32Z" or with -logtimemicros "2026-10-20T00:11:32.123456Z".
pub fn parse_ts(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[10] != b'T' || !s.ends_with('Z') {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hh, mm, ss) = (n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // days_from_civil (H. Hinnant), proleptic Gregorian.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

fn after<'a>(msg: &'a str, pat: &str) -> Option<&'a str> {
    msg.find(pat).map(|i| &msg[i + pat.len()..])
}

fn ev(ts: u64, vote: Hash, class: Class, outcome: Option<Outcome>, lines: u8) -> Event {
    Event {
        ts,
        vote,
        class,
        outcome,
        object: None,
        mn: None,
        seen: u32::from(lines & LINE_NET != 0),
        lines,
    }
}

/// Classifies one line. Unrecognised lines are `Plain`, never an error: the
/// log carries every category the node writes.
pub fn parse_line(line: &str) -> Line {
    let line = line.trim_end_matches(['\r', '\n']);
    let Some((ts, msg)) = line
        .split_once(' ')
        .and_then(|(t, m)| Some((parse_ts(t)?, m)))
    else {
        return match line
            .trim()
            .strip_prefix('(')
            .and_then(|r| r.strip_suffix(')'))
        {
            Some(id) if !id.is_empty() && !id.contains(' ') => Line::Continuation(id.to_string()),
            _ => Line::Plain(None),
        };
    };
    // Anchor every pattern at the start of Core's message: proposal names are
    // network-supplied and unsanitised (budgetproposal.cpp:149), and they show
    // up later in ordinary lines ("Cleaning proposal votes for <name>").
    if let Some(r) = msg.strip_prefix("mvote - new vote (") {
        return vote_line(ts, r, Class::Proposal);
    }
    if let Some(r) = msg.strip_prefix("fbvote - new vote (") {
        return vote_line(ts, r, Class::Finalized);
    }
    // Only CheckAndRemove means removal; AddProposal logs the same wording when
    // it refuses a proposal that was never added (budgetmanager.cpp:338-340).
    if let Some(r) = msg.strip_prefix("CheckAndRemove: Invalid budget proposal ") {
        let reason = r.rsplit_once("]: ").map_or("", |(_, why)| why).trim();
        return hash(r).map_or(Line::Plain(Some(ts)), |h| {
            Line::Event(ev(
                ts,
                h,
                Class::ProposalRemoved,
                Some(Outcome::Removed(reason.to_string())),
                LINE_REMOVAL,
            ))
        });
    }
    // ProcessMessageInner lines are one network message each; AddOrUpdateVote
    // and orphan lines restate a vote and are not counted.
    // ProcessMessageInner prints FormatStateMessage, "<reject code>, <debug>"
    // (util/validation.cpp:19-21), from budgetmanager.cpp:1329 (proposal
    // votes) and :1365 (finalized-budget votes). Every vote code and reason
    // at bd8a672 (:1102-1163, :1211-1271), fb* mirroring m*:
    //   bad-mvote      new vote is too far ahead of current time - <hash> ...   -> RejectFuture
    //   bad-mvote      <AddOrUpdateVote reason> - <hash> [(<protx>)]             -> RejectOlder/TooSoon
    //   bad-mvote      masternode (<protx>) not valid or PoSe banned            -> OtherReject
    //   bad-mvote      unknown masternode - vin: COutPoint(<10 hex>, n)        -> OtherReject
    //   bad-mvote      masternode not valid                                    -> OtherReject
    //   bad-mvote      Proposal not found! [(<protx>)]                         -> OtherReject
    //   bad-fbvote     Finalized Budget <hash> not found! [(<protx>)]          -> OtherReject
    //   bad-mvote      (empty: unknown proposal before tier-two sync ends)      -> OtherReject
    //   bad-mvote-sig  invalid mvote sig from dmn: <protx>                     -> OtherReject
    //   bad-mvote-sig  signature from masternode COutPoint(<10 hex>, n) invalid -> OtherReject
    // An empty debug message prints the code alone, with no ", " (an unknown
    // proposal while tier-two sync runs, budgetmanager.cpp:1500-1502).
    let net = msg
        .strip_prefix("ProcessMessageInner: ")
        .map(|r| r.split_once(", ").unwrap_or((r.trim(), "")))
        .and_then(|(code, body)| {
            let class = if code.starts_with("bad-fbvote") {
                Class::Finalized
            } else if code.starts_with("bad-mvote") {
                Class::Proposal
            } else {
                return None;
            };
            Some((code, body, class))
        });
    let mut code = "";
    let (body, class, kind) = if let Some((c, r, class)) = net {
        code = c;
        (r, class, LINE_NET)
    } else if let Some(r) = msg.strip_prefix("AddOrUpdateVote: ") {
        (r, Class::Unknown, LINE_UPDATE)
    } else if let Some(r) = msg.strip_prefix("Unable to add orphan vote for proposal: ") {
        (r, Class::Proposal, LINE_ORPHAN)
    } else if let Some(r) = msg.strip_prefix("Unable to add orphan vote for final budget: ") {
        (r, Class::Finalized, LINE_ORPHAN)
    } else {
        return Line::Plain(Some(ts));
    };
    let network = kind == LINE_NET;
    let one = |r: &str, o: Outcome| hash(r).map(|h| Line::Event(ev(ts, h, class, Some(o), kind)));
    let parsed = if let Some(r) = body.strip_prefix("New vote inserted: ") {
        one(r, Outcome::Inserted)
    } else if let Some(r) = body.strip_prefix("Existing vote updated: ") {
        one(r, Outcome::Updated)
    } else if let Some(r) = body.strip_prefix("new vote older than existing vote - ") {
        one(r, Outcome::RejectOlder)
    } else if let Some(r) = body.strip_prefix("time between votes is too soon - ") {
        let gap = r
            .get(67..)
            .and_then(|t| t.split(' ').next())
            .and_then(|d| d.parse().ok())
            .unwrap_or(-1);
        one(r, Outcome::RejectTooSoon(gap))
    } else if let Some(r) = body.strip_prefix("new vote is too far ahead of current time - ") {
        let n_time = after(r, "nTime ")
            .and_then(|t| t.split(' ').next())
            .and_then(|t| t.parse().ok())
            .unwrap_or(-1);
        one(r, Outcome::RejectFuture(n_time))
    } else {
        None
    };
    match parsed {
        Some(l) => l,
        None if network => {
            let text: String = format!("{code}: {body}").chars().take(200).collect();
            Line::Other(OtherReject {
                ts,
                class,
                text: text.trim().to_string(),
                mn: None,
                seen: 1,
            })
        }
        None => Line::Plain(Some(ts)),
    }
}

/// "mvote - new vote (<h>) for proposal <p> from mn|dmn <id>" after the prefix.
fn vote_line(ts: u64, r: &str, class: Class) -> Line {
    let Some(h) = hash(r) else {
        return Line::Plain(Some(ts));
    };
    let mut e = ev(ts, h, class, None, LINE_NET);
    e.object = after(r, " for proposal ")
        .or_else(|| after(r, " for budget "))
        .and_then(hash);
    e.mn = after(r, " from dmn ")
        .or_else(|| after(r, " from mn "))
        .map(|m| m.trim().to_string());
    Line::Event(e)
}

pub fn event_key(ts: u64, vote: &Hash) -> Vec<u8> {
    [b"e/".as_slice(), &ts.to_be_bytes(), vote].concat()
}

fn other_key(ts: u64, class: Class, text: &str) -> Vec<u8> {
    let digest: [u8; 32] = Sha256::digest(format!("{class:?}|{text}").as_bytes()).into();
    [b"o/".as_slice(), &ts.to_be_bytes(), &digest].concat()
}

pub fn gap_key(from: u64, until: u64) -> Vec<u8> {
    [b"g/".as_slice(), &from.to_be_bytes(), &until.to_be_bytes()].concat()
}

fn enc<T: Serialize>(v: &T) -> Result<Vec<u8>, String> {
    bincode::serialize(v).map_err(|e| e.to_string())
}

pub fn get<T: for<'de> Deserialize<'de>>(db: &DB, k: &[u8]) -> Result<Option<T>, String> {
    match db.get(k).map_err(|e| e.to_string())? {
        Some(v) => bincode::deserialize(&v)
            .map(Some)
            .map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

pub fn open(path: &str) -> Result<DB, String> {
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.set_max_open_files(64);
    let db = DB::open(&opts, path).map_err(|e| format!("open govlog.db at {path}: {e}"))?;
    let want = SCHEMA_VERSION.to_be_bytes();
    match db.get(SCHEMA).map_err(|e| e.to_string())? {
        None => db.put(SCHEMA, want).map_err(|e| e.to_string())?,
        Some(v) if v.as_slice() == want => {}
        Some(_) => {
            return Err(format!(
                "govlog.db at {path}: schema mismatch, expected {SCHEMA_VERSION}"
            ))
        }
    }
    if db.get(SINCE).map_err(|e| e.to_string())?.is_none() {
        // A DB from before SINCE existed keeps its oldest event as the start.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let start = oldest_event(&db)?.unwrap_or(now);
        db.put(SINCE, start.to_be_bytes())
            .map_err(|e| e.to_string())?;
    }
    Ok(db)
}

/// For `report` beside a live `run`. RocksDB forbids a read-only open while a
/// writer holds the DB (db.h OpenForReadOnly), so this is a secondary instance
/// caught up to the primary; `secondary` is a scratch directory it owns.
pub fn open_secondary(path: &str, secondary: &str) -> Result<DB, String> {
    let mut opts = rocksdb::Options::default();
    opts.set_max_open_files(-1);
    let db = DB::open_as_secondary(&opts, path, secondary)
        .map_err(|e| format!("open govlog.db at {path} as secondary: {e}"))?;
    db.try_catch_up_with_primary().map_err(|e| e.to_string())?;
    Ok(db)
}

pub fn put_gap(db: &DB, from: u64, until: u64, reason: &str) -> Result<(), String> {
    db.put(
        gap_key(from, until),
        enc(&Gap {
            reason: reason.to_string(),
        })?,
    )
    .map_err(|e| e.to_string())
}

fn scan<T: for<'de> Deserialize<'de>>(
    db: &DB,
    prefix: &[u8],
    from: u64,
    to: u64,
) -> Result<Vec<(Vec<u8>, T)>, String> {
    let start = [prefix, &from.to_be_bytes()].concat();
    let mut out = Vec::new();
    for item in db.iterator(IteratorMode::From(&start, Direction::Forward)) {
        let (k, v) = item.map_err(|e| e.to_string())?;
        let ts = k
            .get(2..10)
            .and_then(|b| b.try_into().ok())
            .map(u64::from_be_bytes);
        if !k.starts_with(prefix) || ts.is_none_or(|t| t >= to) {
            break;
        }
        out.push((
            k.to_vec(),
            bincode::deserialize(&v).map_err(|e| e.to_string())?,
        ));
    }
    Ok(out)
}

/// When this DB started watching: `SINCE`, else (DBs made before it) the
/// oldest stored event.
pub fn since(db: &DB) -> Result<Option<u64>, String> {
    if let Some(v) = db.get(SINCE).map_err(|e| e.to_string())? {
        return Ok(v.as_slice().try_into().ok().map(u64::from_be_bytes));
    }
    oldest_event(db)
}

fn oldest_event(db: &DB) -> Result<Option<u64>, String> {
    match db
        .iterator(IteratorMode::From(b"e/", Direction::Forward))
        .next()
    {
        Some(item) => {
            let (k, _) = item.map_err(|e| e.to_string())?;
            Ok(k.strip_prefix(b"e/")
                .and_then(|r| r.get(..8))
                .and_then(|b| b.try_into().ok())
                .map(u64::from_be_bytes))
        }
        None => Ok(None),
    }
}

/// Events in [from, to), oldest first.
pub fn events(db: &DB, from: u64, to: u64) -> Result<Vec<Event>, String> {
    Ok(scan(db, b"e/", from, to)?
        .into_iter()
        .map(|(_, e)| e)
        .collect())
}

pub fn others(db: &DB, from: u64, to: u64) -> Result<Vec<OtherReject>, String> {
    Ok(scan(db, b"o/", from, to)?
        .into_iter()
        .map(|(_, e)| e)
        .collect())
}

/// Gaps as (from, until, gap), oldest first.
pub fn gaps(db: &DB) -> Result<Vec<(u64, u64, Gap)>, String> {
    let mut out = Vec::new();
    for (k, g) in scan::<Gap>(db, b"g/", 0, u64::MAX)? {
        let until = k
            .get(10..18)
            .and_then(|b| b.try_into().ok())
            .map(u64::from_be_bytes)
            .ok_or("bad g/ key")?;
        let from = u64::from_be_bytes(k[2..10].try_into().map_err(|_| "bad g/ key")?);
        out.push((from, until, g));
    }
    Ok(out)
}

/// Deletes events and other rejections older than `before`; gaps are kept.
pub fn prune(db: &DB, before: u64) -> Result<usize, String> {
    let mut n = 0;
    let mut batch = WriteBatch::default();
    for prefix in [b"e/".as_slice(), b"o/"] {
        let end = [prefix, &before.to_be_bytes()].concat();
        for item in db.iterator(IteratorMode::From(prefix, Direction::Forward)) {
            let (k, _) = item.map_err(|e| e.to_string())?;
            if !k.starts_with(prefix) || k.as_ref() >= end.as_slice() {
                break;
            }
            batch.delete(k);
            n += 1;
        }
    }
    db.write(batch).map_err(|e| e.to_string())?;
    Ok(n)
}

/// Records staged during one poll, so later lines (and a drain before the
/// current file) see earlier ones before anything is committed.
#[derive(Default)]
pub struct Staging {
    events: HashMap<Vec<u8>, Event>,
    others: HashMap<Vec<u8>, OtherReject>,
}

impl Staging {
    fn event(&mut self, db: &DB, k: &[u8]) -> Result<Option<Event>, String> {
        match self.events.remove(k) {
            Some(e) => Ok(Some(e)),
            None => get(db, k),
        }
    }

    fn other(&mut self, db: &DB, k: &[u8]) -> Result<Option<OtherReject>, String> {
        match self.others.remove(k) {
            Some(o) => Ok(Some(o)),
            None => get(db, k),
        }
    }

    pub fn flush(self, batch: &mut WriteBatch) -> Result<(), String> {
        for (k, e) in self.events {
            batch.put(k, enc(&e)?);
        }
        for (k, o) in self.others {
            batch.put(k, enc(&o)?);
        }
        Ok(())
    }
}

/// Ingests complete lines into `staging`. `pending` is the key of the last
/// rejection, so a following Continuation line attaches its masternode.
/// Returns the newest timestamp of any line.
pub fn ingest(
    db: &DB,
    lines: &[&str],
    pending: &mut Option<Vec<u8>>,
    staging: &mut Staging,
) -> Result<u64, String> {
    let mut last_ts = 0;
    for l in lines {
        match parse_line(l) {
            Line::Event(e) => {
                last_ts = last_ts.max(e.ts);
                let rejected = e.outcome.as_ref().is_some_and(Outcome::is_reject);
                let here = event_key(e.ts, &e.vote);
                let prev = event_key(e.ts.saturating_sub(1), &e.vote);
                // Each line is timestamped on its own, so lines of one vote
                // can straddle a second. Merge across it only when the line
                // kinds differ; two network lines are two receipts.
                let (k, cur) = match staging.event(db, &here)? {
                    Some(c) => (here, Some(c)),
                    None => match staging.event(db, &prev)? {
                        Some(c) if e.ts > 0 && c.lines & e.lines == 0 => (prev, Some(c)),
                        Some(c) => {
                            staging.events.insert(prev, c);
                            (here, None)
                        }
                        None => (here, None),
                    },
                };
                let merged = match cur {
                    Some(mut c) => {
                        c.merge(e);
                        c
                    }
                    None => e,
                };
                *pending = rejected.then(|| k.clone());
                staging.events.insert(k, merged);
            }
            Line::Other(o) => {
                last_ts = last_ts.max(o.ts);
                let k = other_key(o.ts, o.class, &o.text);
                let merged = match staging.other(db, &k)? {
                    Some(mut c) => {
                        c.seen += o.seen;
                        c
                    }
                    None => o,
                };
                *pending = Some(k.clone());
                staging.others.insert(k, merged);
            }
            Line::Continuation(mn) => {
                if let Some(k) = pending.take() {
                    if k.starts_with(b"o/") {
                        if let Some(mut c) = staging.other(db, &k)? {
                            c.mn.get_or_insert(mn);
                            staging.others.insert(k, c);
                        }
                    } else if let Some(mut c) = staging.event(db, &k)? {
                        c.mn.get_or_insert(mn);
                        staging.events.insert(k, c);
                    }
                }
            }
            // A blank line follows each rejection; keep `pending` across it.
            Line::Plain(None) if l.trim().is_empty() => {}
            Line::Plain(ts) => {
                last_ts = last_ts.max(ts.unwrap_or(0));
                *pending = None;
            }
        }
    }
    Ok(last_ts)
}

/// `ingest` committed on its own, for callers without a cursor.
pub fn ingest_commit(
    db: &DB,
    lines: &[&str],
    pending: &mut Option<Vec<u8>>,
) -> Result<u64, String> {
    let mut staging = Staging::default();
    let ts = ingest(db, lines, pending, &mut staging)?;
    let mut batch = WriteBatch::default();
    staging.flush(&mut batch)?;
    db.write(batch).map_err(|e| e.to_string())?;
    Ok(ts)
}

const HEAD_LEN: usize = 128;
/// How long rotated copies are searched before a drain becomes a gap.
/// logrotate copies, truncates, then compresses <log>.1 to <log>.1.gz
/// (prod: compress, no delaycompress), so the copy can briefly be missing.
const DRAIN_WAIT: u64 = 120;
/// Rotated copies searched for a drain: <log>.1 .. <log>.MAX_COPY (+ .gz).
const MAX_COPY: u32 = 9;
/// "2026-10-20T00:11:32Z " plus a byte: enough to tell two log files apart.
const MIN_HEAD: usize = 22;

fn head(path: &std::path::Path) -> Vec<u8> {
    use std::io::Read;
    let mut b = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(HEAD_LEN as u64).read_to_end(&mut b);
    }
    b
}

/// Same file, but rewritten from the start since `c` was saved.
fn truncated(c: &Cursor, len: u64, now_head: &[u8]) -> bool {
    len < c.offset
        || (!c.head.is_empty() && !now_head.starts_with(&c.head) && !c.head.starts_with(now_head))
}

fn inode(m: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::ino(m)
}

/// Timestamp of the first timestamped line in `path`.
fn first_ts(path: &std::path::Path) -> Option<u64> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(path).ok()?;
    BufReader::new(f)
        .split(b'\n')
        .take(1000)
        .map_while(Result::ok)
        .find_map(|l| {
            let l = String::from_utf8_lossy(&l);
            l.split_once(' ').and_then(|(t, _)| parse_ts(t))
        })
}

fn sibling(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(suffix);
    p.into()
}

/// Contents of rotated copy N: <log>.N if present, else <log>.N.gz through
/// the system gzip (no compression crate in the dependency tree).
fn copy_contents(path: &std::path::Path, n: u32) -> Option<Vec<u8>> {
    if let Ok(b) = std::fs::read(sibling(path, &format!(".{n}"))) {
        return Some(b);
    }
    let gz = sibling(path, &format!(".{n}.gz"));
    if !gz.exists() {
        return None;
    }
    let out = std::process::Command::new("gzip")
        .arg("-dc")
        .arg(&gz)
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// The rotated copy that continues `d`: same head bytes, long enough.
/// Searched newest first, so an empty head matches the newest copy.
fn find_copy(path: &std::path::Path, d: &Drain) -> Option<Vec<u8>> {
    // A head shorter than one full timestamp could prefix several copies; a
    // guess could drop one file and count another twice. Such a drain ends
    // as a gap instead.
    if d.head.len() < MIN_HEAD {
        return None;
    }
    (1..=MAX_COPY)
        .filter_map(|n| copy_contents(path, n))
        .find(|b| b.len() as u64 >= d.offset && b.starts_with(&d.head))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Follows one log file across logrotate. A rename-and-create rotation is
/// drained through the open handle; a copytruncate rotation (prod) and any
/// rotation seen at startup are finished from the rotated copies before the
/// current file is read again.
pub struct Tail {
    path: std::path::PathBuf,
    file: Option<std::fs::File>,
    cursor: Cursor,
    /// Bytes after the last newline; not yet part of `cursor.offset`.
    partial: Vec<u8>,
    pending: Option<Vec<u8>>,
    drain_wait: u64,
}

impl Tail {
    pub fn new(db: &DB, path: impl Into<std::path::PathBuf>) -> Result<Tail, String> {
        let path = path.into();
        let cursor: Cursor = get(db, CURSOR)?.unwrap_or_default();
        let mut t = Tail {
            path,
            file: None,
            cursor,
            partial: Vec::new(),
            pending: None,
            drain_wait: DRAIN_WAIT,
        };
        let meta = t.meta()?;
        if t.cursor.inode != 0
            && (t.cursor.inode != inode(&meta) || truncated(&t.cursor, meta.len(), &head(&t.path)))
        {
            // Rotated while the sidecar was down: a rename leaves the whole
            // old file in the copy; a copytruncate may have lost its window.
            let copytruncate = t.cursor.inode == inode(&meta);
            t.start_drain(copytruncate);
        }
        t.cursor.inode = inode(&meta);
        Ok(t)
    }

    #[cfg(test)]
    fn no_wait(mut self) -> Tail {
        self.drain_wait = 0;
        self
    }

    fn meta(&self) -> Result<std::fs::Metadata, String> {
        std::fs::metadata(&self.path).map_err(|e| format!("{}: {e}", self.path.display()))
    }

    /// Queues the rest of the file the cursor was reading and moves the
    /// cursor to the start of the current file.
    fn start_drain(&mut self, copytruncate: bool) {
        self.cursor.drains.push(Drain {
            offset: self.cursor.offset,
            head: self.cursor.head.clone(),
            from_ts: self.cursor.last_ts,
            since: unix_now(),
            copytruncate,
        });
        self.partial.clear();
        self.pending = None;
        self.file = None;
        self.cursor.offset = 0;
        self.cursor.head.clear();
    }

    /// Finishes queued drains in order. Returns true when none is left.
    fn drain(
        &mut self,
        db: &DB,
        staging: &mut Staging,
        batch: &mut WriteBatch,
    ) -> Result<bool, String> {
        while let Some(d) = self.cursor.drains.first().cloned() {
            let next = first_ts(&self.path).unwrap_or(d.from_ts).max(d.from_ts);
            match find_copy(&self.path, &d) {
                Some(b) => {
                    let text = String::from_utf8_lossy(&b[d.offset as usize..]);
                    let lines: Vec<&str> = text.split('\n').collect();
                    let last = ingest(db, &lines, &mut None, staging)?;
                    self.cursor.last_ts = self.cursor.last_ts.max(last);
                    if d.copytruncate {
                        // Lines written between logrotate's copy and its
                        // truncate are in neither file; usually none.
                        batch.put(
                            gap_key(last.max(d.from_ts), next.max(last)),
                            enc(&Gap {
                                reason: "possible loss in the copytruncate window".into(),
                            })?,
                        );
                    }
                }
                None if unix_now().saturating_sub(d.since) < self.drain_wait => return Ok(false),
                None => batch.put(
                    gap_key(d.from_ts, next),
                    enc(&Gap {
                        reason: "log rotated; rotated copy not found".into(),
                    })?,
                ),
            }
            self.cursor.drains.remove(0);
        }
        Ok(true)
    }

    /// Reads everything new and commits lines, gaps and cursor together.
    /// Returns the number of lines read.
    pub fn poll(&mut self, db: &DB) -> Result<usize, String> {
        use std::io::{Read, Seek, SeekFrom};
        let meta = self.meta()?;
        let renamed = inode(&meta) != self.cursor.inode;
        let mut staging = Staging::default();
        let mut batch = WriteBatch::default();
        let mut n = 0;
        if renamed {
            if let Some(f) = self.file.as_mut() {
                // The open handle still reads the renamed file: finish it.
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                if !buf.is_empty() || !self.partial.is_empty() {
                    buf.push(b'\n');
                }
                n += self.consume(db, &buf, &mut staging)?;
            } else if self.cursor.offset > 0 || !self.cursor.head.is_empty() {
                // Not open (reading paused for a drain): the rest of the
                // renamed file is in a rotated copy.
                self.start_drain(false);
            }
            self.partial.clear();
            self.pending = None;
            self.file = None;
            self.cursor.inode = inode(&meta);
            self.cursor.offset = 0;
            self.cursor.head.clear();
        } else if truncated(&self.cursor, meta.len(), &head(&self.path)) {
            self.start_drain(true);
        }
        if self.drain(db, &mut staging, &mut batch)? {
            if self.file.is_none() {
                let mut f = std::fs::File::open(&self.path)
                    .map_err(|e| format!("{}: {e}", self.path.display()))?;
                // Renamed between the metadata check and the open: this handle
                // is the replacement file. Leave it for the next poll.
                if f.metadata().map(|m| inode(&m)).map_err(|e| e.to_string())? != self.cursor.inode
                {
                    staging.flush(&mut batch)?;
                    batch.put(CURSOR, enc(&self.cursor)?);
                    db.write(batch).map_err(|e| e.to_string())?;
                    return Ok(n);
                }
                f.seek(SeekFrom::Start(
                    self.cursor.offset + self.partial.len() as u64,
                ))
                .map_err(|e| e.to_string())?;
                self.file = Some(f);
            }
            let mut buf = Vec::new();
            if let Some(f) = self.file.as_mut() {
                f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            }
            n += self.consume(db, &buf, &mut staging)?;
        }
        // Tracked even while reading is paused, so a second rotation of a
        // file not yet read is still detected.
        if self.cursor.head.len() < HEAD_LEN {
            self.cursor.head = head(&self.path);
        }
        staging.flush(&mut batch)?;
        batch.put(CURSOR, enc(&self.cursor)?);
        db.write(batch).map_err(|e| e.to_string())?;
        Ok(n)
    }

    fn consume(&mut self, db: &DB, chunk: &[u8], staging: &mut Staging) -> Result<usize, String> {
        self.partial.extend_from_slice(chunk);
        let Some(end) = self.partial.iter().rposition(|&b| b == b'\n') else {
            return Ok(0);
        };
        let rest = self.partial.split_off(end + 1);
        let complete = std::mem::replace(&mut self.partial, rest);
        // Core logs network-supplied strings (proposal names) unvalidated, so
        // a line may not be UTF-8; decode lossily rather than stop.
        let text = String::from_utf8_lossy(&complete);
        let lines: Vec<&str> = text.split('\n').collect();
        let last = ingest(db, &lines, &mut self.pending, staging)?;
        self.cursor.offset += complete.len() as u64;
        self.cursor.last_ts = self.cursor.last_ts.max(last);
        Ok(lines.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const H2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const H3: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    const LEGACY_MN: &str = "COutPoint(0c58c42e76, 12)";

    fn h(s: &str) -> Hash {
        hash(s).unwrap()
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_ts("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_ts("2026-10-20T00:11:32Z"), Some(1_792_455_092));
        assert_eq!(parse_ts("2026-10-20T00:11:32.123456Z"), Some(1_792_455_092));
        assert_eq!(parse_ts("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(parse_ts("ProcessMessages:"), None);
    }

    #[test]
    fn accepted_vote_lines() {
        let Line::Event(a) = parse_line(&format!(
            "2026-10-20T00:11:32Z AddOrUpdateVote: New vote inserted: {H1}"
        )) else {
            panic!()
        };
        assert_eq!(
            (a.vote, a.outcome.clone(), a.class, a.seen),
            (h(H1), Some(Outcome::Inserted), Class::Unknown, 0)
        );
        // Mainnet legacy masternode (budgetmanager.cpp:1170-1171).
        let Line::Event(m) = parse_line(&format!(
            "2026-10-20T00:11:32Z mvote - new vote ({H1}) for proposal {H2} from mn {LEGACY_MN}"
        )) else {
            panic!()
        };
        assert_eq!(
            (m.class, m.object, m.mn.as_deref(), m.seen),
            (Class::Proposal, Some(h(H2)), Some(LEGACY_MN), 1)
        );
        let Line::Event(f) = parse_line(&format!(
            "2026-10-20T00:11:32Z fbvote - new vote ({H1}) for budget {H2} from dmn {H3}"
        )) else {
            panic!()
        };
        assert_eq!((f.class, f.mn.as_deref()), (Class::Finalized, Some(H3)));
    }

    #[test]
    fn rejection_lines() {
        let older = format!(
            "2026-10-20T00:11:33Z AddOrUpdateVote: new vote older than existing vote - {H1}"
        );
        assert!(matches!(
            parse_line(&older),
            Line::Event(Event {
                outcome: Some(Outcome::RejectOlder),
                seen: 0,
                ..
            })
        ));
        let Line::Event(e) = parse_line(&format!(
            "2026-10-20T00:11:33Z ProcessMessageInner: bad-mvote, time between votes is too soon - {H1} - 1200 sec < 3600 sec"
        )) else {
            panic!()
        };
        assert_eq!(
            (e.class, e.outcome, e.seen),
            (Class::Proposal, Some(Outcome::RejectTooSoon(1200)), 1)
        );
        let Line::Event(e) = parse_line(&format!(
            "2026-10-20T00:11:33Z ProcessMessageInner: bad-fbvote, new vote is too far ahead of current time - {H1} - nTime 1792500000 - Max Time 1792458692"
        )) else {
            panic!()
        };
        assert_eq!(
            (e.class, e.outcome),
            (Class::Finalized, Some(Outcome::RejectFuture(1_792_500_000)))
        );
        assert_eq!(
            parse_line(&format!(" ({H3})")),
            Line::Continuation(H3.to_string())
        );
        let Line::Event(e) = parse_line(&format!(
            "2026-10-20T00:11:34Z CheckAndRemove: Invalid budget proposal {H2} [Luke (Q4 2026)]: Heavily Downvoted"
        )) else {
            panic!()
        };
        assert_eq!(
            (e.class, e.outcome),
            (
                Class::ProposalRemoved,
                Some(Outcome::Removed("Heavily Downvoted".into()))
            )
        );
        // Same wording from AddProposal refusing a new proposal: not a removal.
        assert!(matches!(
            parse_line(&format!(
                "2026-10-20T00:11:34Z AddProposal: Invalid budget proposal {H2} [x]: bad"
            )),
            Line::Plain(Some(_))
        ));
        assert_eq!(
            parse_line("2026-10-20T00:11:32Z UpdatedBlockTip:  PASSED"),
            Line::Plain(Some(1_792_455_092))
        );
    }

    #[test]
    fn rejection_without_a_vote_hash_is_never_stored_as_a_vote() {
        // Legacy unknown masternode (budgetmanager.cpp:1155-1159): the hex
        // in it is a collateral, not a vote.
        let line = format!(
            "2026-10-20T00:11:35Z ProcessMessageInner: bad-mvote, unknown masternode - vin: COutPoint({}, 3)",
            &H1[..10]
        );
        let Line::Other(o) = parse_line(&line) else {
            panic!()
        };
        assert_eq!(o.class, Class::Proposal);
        assert!(
            o.text.starts_with("bad-mvote: unknown masternode"),
            "{}",
            o.text
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let dmn =
            "2026-10-20T00:11:35Z ProcessMessageInner: bad-mvote, masternode is PoSe banned (H1)";
        ingest_commit(&db, &[&line, &line, dmn], &mut None).unwrap();
        assert!(events(&db, 0, u64::MAX).unwrap().is_empty());
        let o = others(&db, 0, u64::MAX).unwrap();
        assert_eq!(o.len(), 2);
        assert_eq!(o.iter().map(|x| x.seen).sum::<u32>(), 3);
    }

    #[test]
    fn lines_for_one_vote_merge_even_across_a_second() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let lines = [
            format!("2026-10-20T00:11:33Z AddOrUpdateVote: time between votes is too soon - {H1} - 1200 sec < 3600 sec"),
            String::new(),
            format!("2026-10-20T00:11:34Z ProcessMessageInner: bad-mvote, time between votes is too soon - {H1} - 1200 sec < 3600 sec"),
            format!(" ({H3})"),
            format!("2026-10-20T00:11:40Z AddOrUpdateVote: New vote inserted: {H2}"),
            format!("2026-10-20T00:11:41Z mvote - new vote ({H2}) for proposal {H1} from mn {LEGACY_MN}"),
        ];
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        ingest_commit(&db, &refs, &mut None).unwrap();
        let ev = events(&db, 0, u64::MAX).unwrap();
        assert_eq!(ev.len(), 2);
        assert_eq!(
            (
                ev[0].class,
                ev[0].outcome.clone(),
                ev[0].mn.as_deref(),
                ev[0].seen
            ),
            (
                Class::Proposal,
                Some(Outcome::RejectTooSoon(1200)),
                Some(H3),
                1
            )
        );
        assert_eq!(
            (
                ev[1].class,
                ev[1].outcome.clone(),
                ev[1].object,
                ev[1].mn.as_deref()
            ),
            (
                Class::Proposal,
                Some(Outcome::Inserted),
                Some(h(H1)),
                Some(LEGACY_MN)
            )
        );
        assert_eq!(prune(&db, 1_792_455_100).unwrap(), 1, "strictly older only");
    }

    #[test]
    fn non_utf8_bytes_do_not_stop_ingest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().join("db").to_str().unwrap()).unwrap();
        let log = tmp.path().join("debug.log");
        let mut bytes = b"2026-10-20T00:00:01Z AddProposal: name \xff\xfe bad\n".to_vec();
        bytes.extend_from_slice(vote_line("2026-10-20T00:00:02Z", H1).as_bytes());
        std::fs::write(&log, bytes).unwrap();
        let mut t = Tail::new(&db, &log).unwrap();
        t.poll(&db).unwrap();
        assert_eq!(events(&db, 0, u64::MAX).unwrap().len(), 1);
    }

    fn vote_line(ts: &str, hash: &str) -> String {
        format!("{ts} AddOrUpdateVote: New vote inserted: {hash}\n")
    }

    fn append(log: &std::path::Path, s: &str) {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap()
            .write_all(s.as_bytes())
            .unwrap()
    }

    #[test]
    fn partial_line_survives_a_restart() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().join("db").to_str().unwrap()).unwrap();
        let log = tmp.path().join("debug.log");
        append(&log, &vote_line("2026-10-20T00:00:01Z", H1));
        append(
            &log,
            &format!(
                "2026-10-20T00:00:02Z AddOrUpdateVote: New vote inserted: {}",
                &H2[..10]
            ),
        );
        Tail::new(&db, &log).unwrap().poll(&db).unwrap();
        // Restart: the half line was not committed, so it is read again whole.
        append(&log, &format!("{}\n", &H2[10..]));
        Tail::new(&db, &log).unwrap().poll(&db).unwrap();
        let ev = events(&db, 0, u64::MAX).unwrap();
        assert_eq!(
            ev.iter().map(|e| e.vote).collect::<Vec<_>>(),
            vec![h(H1), h(H2)]
        );
    }

    #[test]
    fn tail_follows_rename_rotation_and_copytruncate() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().join("db").to_str().unwrap()).unwrap();
        let log = tmp.path().join("debug.log");
        let copy = tmp.path().join("debug.log.1");
        // Long enough that the saved head is full, as on a real log; each
        // file starts with its own timestamps, as after a real rotation.
        let filler = |ts: &str| {
            format!(
                "{ts} UpdatedBlockTip: some ordinary line that is long enough to fill the head\n"
            )
            .repeat(3)
        };
        append(&log, &filler("2026-10-20T00:00:00Z"));
        append(&log, &vote_line("2026-10-20T00:00:01Z", H1));
        let mut t = Tail::new(&db, &log).unwrap().no_wait();
        t.poll(&db).unwrap();
        assert_eq!(events(&db, 0, u64::MAX).unwrap().len(), 1);

        // logrotate create: rename, new file. Lines written before the rename
        // are drained through the open handle.
        append(&log, &vote_line("2026-10-20T00:00:03Z", H2));
        std::fs::rename(&log, &copy).unwrap();
        append(&log, &filler("2026-10-20T00:00:03Z"));
        append(&log, &vote_line("2026-10-20T00:00:04Z", H3));
        t.poll(&db).unwrap();
        assert_eq!(events(&db, 0, u64::MAX).unwrap().len(), 3);
        assert!(gaps(&db).unwrap().is_empty());

        // copytruncate (prod): a line lands after the last read, logrotate
        // copies to .1 and truncates. The copy supplies the line; only the
        // copy/truncate window is marked.
        append(&log, &vote_line("2026-10-20T00:00:05Z", H1));
        std::fs::copy(&log, &copy).unwrap();
        std::fs::write(
            &log,
            filler("2026-10-20T00:00:08Z") + &vote_line("2026-10-20T00:00:09Z", H2),
        )
        .unwrap();
        t.poll(&db).unwrap();
        assert_eq!(events(&db, 0, u64::MAX).unwrap().len(), 5);
        let g = gaps(&db).unwrap();
        assert_eq!(g.len(), 1);
        assert!(g[0].2.reason.contains("copytruncate window"));

        // The next polls must not see the truncation again (head refreshed).
        t.poll(&db).unwrap();
        append(&log, &vote_line("2026-10-20T00:00:10Z", H3));
        t.poll(&db).unwrap();
        assert_eq!(events(&db, 0, u64::MAX).unwrap().len(), 6);
        assert_eq!(gaps(&db).unwrap().len(), 1);

        // Truncated with no usable copy: a gap that says so.
        std::fs::remove_file(&copy).unwrap();
        std::fs::write(&log, vote_line("2026-10-20T00:00:20Z", H1)).unwrap();
        t.poll(&db).unwrap();
        assert_eq!(events(&db, 0, u64::MAX).unwrap().len(), 7);
        let g = gaps(&db).unwrap();
        assert_eq!(g.len(), 2);
        assert!(g[1].2.reason.contains("not found"));
    }

    #[test]
    fn restart_after_rotation_drains_the_copy_or_records_the_window() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().join("db").to_str().unwrap()).unwrap();
        let log = tmp.path().join("debug.log");
        std::fs::write(&log, vote_line("2026-10-20T00:00:01Z", H1)).unwrap();
        Tail::new(&db, &log).unwrap().poll(&db).unwrap();
        // Down while one line was added and the file renamed to .1.
        append(&log, &vote_line("2026-10-20T00:00:02Z", H2));
        std::fs::rename(&log, sibling(&log, ".1")).unwrap();
        std::fs::write(&log, vote_line("2026-10-20T01:00:00Z", H3)).unwrap();
        Tail::new(&db, &log).unwrap().poll(&db).unwrap();
        assert_eq!(events(&db, 0, u64::MAX).unwrap().len(), 3);
        assert!(gaps(&db).unwrap().is_empty(), "renamed copy drained fully");

        // Down across two rotations: .1 is no longer the cursor's file.
        let later = tmp.path().join("other");
        std::fs::write(&later, vote_line("2026-10-20T02:00:00Z", H1)).unwrap();
        std::fs::rename(&later, sibling(&log, ".1")).unwrap();
        std::fs::remove_file(&log).unwrap();
        std::fs::write(&log, vote_line("2026-10-20T03:00:00Z", H2)).unwrap();
        Tail::new(&db, &log).unwrap().no_wait().poll(&db).unwrap();
        let g = gaps(&db).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(
            (g[0].0, g[0].1),
            (
                parse_ts("2026-10-20T01:00:00Z").unwrap(),
                parse_ts("2026-10-20T03:00:00Z").unwrap()
            )
        );
    }

    #[test]
    fn copytruncate_with_compressed_copy_is_drained() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().join("db").to_str().unwrap()).unwrap();
        let log = tmp.path().join("debug.log");
        append(&log, &vote_line("2026-10-20T00:00:01Z", H1));
        let mut t = Tail::new(&db, &log).unwrap();
        t.poll(&db).unwrap();
        // Prod: copy, truncate, then gzip the copy (compress, no delaycompress).
        append(&log, &vote_line("2026-10-20T00:00:02Z", H2));
        std::fs::copy(&log, sibling(&log, ".1")).unwrap();
        std::fs::write(&log, vote_line("2026-10-20T00:00:09Z", H3)).unwrap();
        let ok = std::process::Command::new("gzip")
            .arg(sibling(&log, ".1"))
            .status()
            .unwrap()
            .success();
        assert!(ok && sibling(&log, ".1.gz").exists());
        t.poll(&db).unwrap();
        let votes: Vec<Hash> = events(&db, 0, u64::MAX)
            .unwrap()
            .iter()
            .map(|e| e.vote)
            .collect();
        assert_eq!(votes, vec![h(H1), h(H2), h(H3)]);
        assert!(gaps(&db).unwrap()[0]
            .2
            .reason
            .contains("copytruncate window"));
    }

    #[test]
    fn missing_copy_waits_before_recording_a_gap() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().join("db").to_str().unwrap()).unwrap();
        let log = tmp.path().join("debug.log");
        append(&log, &vote_line("2026-10-20T00:00:01Z", H1));
        let mut t = Tail::new(&db, &log).unwrap();
        t.poll(&db).unwrap();
        std::fs::write(&log, "").unwrap();
        t.poll(&db).unwrap();
        // Still inside the wait: no gap yet, and the drain survives a restart.
        assert!(gaps(&db).unwrap().is_empty());
        let c: Cursor = get(&db, CURSOR).unwrap().unwrap();
        assert!(!c.drains.is_empty());
        let mut t = Tail::new(&db, &log).unwrap().no_wait();
        t.poll(&db).unwrap();
        assert!(gaps(&db).unwrap()[0].2.reason.contains("not found"));
    }

    #[test]
    fn receipts_in_adjacent_seconds_stay_separate() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let fut = |ts: &str| {
            format!("{ts} ProcessMessageInner: bad-mvote, new vote is too far ahead of current time - {H1} - nTime 1792500000 - Max Time 1792458692")
        };
        let (a, b) = (fut("2026-10-20T00:11:33Z"), fut("2026-10-20T00:11:34Z"));
        ingest_commit(&db, &[&a, "", &b, ""], &mut None).unwrap();
        let ev = events(&db, 0, u64::MAX).unwrap();
        assert_eq!(
            ev.iter().map(|e| (e.ts, e.seen)).collect::<Vec<_>>(),
            vec![(1_792_455_093, 1), (1_792_455_094, 1)]
        );
    }

    #[test]
    fn proposal_names_cannot_fake_log_events() {
        let name_lines = [
            "2026-10-20T00:18:16Z Cleaning proposal votes for bad-mvote, masternode not valid. Before: YES=1, NO=0".to_string(),
            format!("2026-10-20T00:18:16Z Cleaned proposal votes for mvote - new vote ({H1}) for proposal {H2}. After: YES=1, NO=0"),
        ];
        for l in &name_lines {
            assert!(matches!(parse_line(l), Line::Plain(Some(_))), "{l}");
        }
        // Same text as a proposal rejection and as a finalized one: two rows.
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        ingest_commit(
            &db,
            &[
                "2026-10-20T00:11:35Z ProcessMessageInner: bad-mvote, masternode not valid",
                "2026-10-20T00:11:35Z ProcessMessageInner: bad-fbvote, masternode not valid",
            ],
            &mut None,
        )
        .unwrap();
        let o = others(&db, 0, u64::MAX).unwrap();
        assert_eq!(o.len(), 2);
    }

    #[test]
    fn orphan_wrapper_line_merges_with_its_reason_across_a_second() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        let a = format!(
            "2026-10-20T00:11:33Z AddOrUpdateVote: new vote older than existing vote - {H1}"
        );
        let b = format!("2026-10-20T00:11:34Z Unable to add orphan vote for proposal: new vote older than existing vote - {H1}");
        ingest_commit(&db, &[&a, "", &b, ""], &mut None).unwrap();
        let ev = events(&db, 0, u64::MAX).unwrap();
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].class, ev[0].seen), (Class::Proposal, 0));
    }

    #[test]
    fn vote_split_by_a_copytruncate_stays_one_event() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().join("db").to_str().unwrap()).unwrap();
        let log = tmp.path().join("debug.log");
        append(&log, &vote_line("2026-10-20T00:00:01Z", H1));
        let mut t = Tail::new(&db, &log).unwrap();
        t.poll(&db).unwrap();
        // AddOrUpdateVote lands in the old file, its mvote line in the new one.
        append(&log, &vote_line("2026-10-20T00:00:05Z", H2));
        std::fs::copy(&log, sibling(&log, ".1")).unwrap();
        std::fs::write(
            &log,
            format!("2026-10-20T00:00:06Z mvote - new vote ({H2}) for proposal {H3} from mn {LEGACY_MN}\n"),
        )
        .unwrap();
        t.poll(&db).unwrap();
        let ev = events(&db, 0, u64::MAX).unwrap();
        assert_eq!(ev.len(), 2);
        assert_eq!(
            (
                ev[1].vote,
                ev[1].outcome.clone(),
                ev[1].class,
                ev[1].mn.as_deref()
            ),
            (
                h(H2),
                Some(Outcome::Inserted),
                Class::Proposal,
                Some(LEGACY_MN)
            )
        );
    }

    #[test]
    fn two_rotations_during_one_pending_drain_recover_both_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().join("db").to_str().unwrap()).unwrap();
        let log = tmp.path().join("debug.log");
        append(&log, &vote_line("2026-10-20T00:00:01Z", H1));
        let mut t = Tail::new(&db, &log).unwrap();
        t.poll(&db).unwrap();
        // First copytruncate, but the copy is not there yet (still compressing).
        append(&log, &vote_line("2026-10-20T00:00:02Z", H2));
        let gen1 = std::fs::read(&log).unwrap();
        std::fs::write(&log, vote_line("2026-10-20T00:00:03Z", H3)).unwrap();
        t.poll(&db).unwrap();
        assert_eq!(
            events(&db, 0, u64::MAX).unwrap().len(),
            1,
            "paused until the copy appears"
        );
        // Second rotation: gen1 is now .2, gen2 (never read) is .1.
        std::fs::write(sibling(&log, ".2"), &gen1).unwrap();
        std::fs::copy(&log, sibling(&log, ".1")).unwrap();
        std::fs::write(&log, vote_line("2026-10-20T00:00:09Z", H1)).unwrap();
        t.poll(&db).unwrap();
        let votes: Vec<(u64, Hash)> = events(&db, 0, u64::MAX)
            .unwrap()
            .iter()
            .map(|e| (e.ts, e.vote))
            .collect();
        assert_eq!(votes.len(), 4);
        assert_eq!(
            votes.iter().map(|v| v.1).collect::<Vec<_>>(),
            vec![h(H1), h(H2), h(H3), h(H1)]
        );
    }

    #[test]
    fn signature_rejections_are_kept() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().to_str().unwrap()).unwrap();
        ingest_commit(
            &db,
            &[
                "2026-10-20T00:11:35Z ProcessMessageInner: bad-mvote-sig, invalid signature from COutPoint(0c58c42e76, 12)",
                "2026-10-20T00:11:35Z ProcessMessageInner: bad-fbvote-sig, invalid signature",
                "2026-10-20T00:11:35Z ProcessMessageInner: bad-proposal, not a vote",
            ],
            &mut None,
        )
        .unwrap();
        let o = others(&db, 0, u64::MAX).unwrap();
        let mut got: Vec<(Class, bool)> = o
            .iter()
            .map(|x| (x.class, x.text.contains("-sig: invalid signature")))
            .collect();
        got.sort_by_key(|g| format!("{g:?}"));
        assert_eq!(got, vec![(Class::Finalized, true), (Class::Proposal, true)]);
    }

    #[test]
    fn short_head_drain_records_a_gap_instead_of_guessing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = open(tmp.path().join("db").to_str().unwrap()).unwrap();
        let log = tmp.path().join("debug.log");
        // One short complete line: committed, but its head (7 bytes) is too
        // short to identify the rotated copy.
        std::fs::write(&log, "2026 x\n").unwrap();
        let mut t = Tail::new(&db, &log).unwrap().no_wait();
        t.poll(&db).unwrap();
        std::fs::write(sibling(&log, ".1"), "2026 anything").unwrap();
        std::fs::write(&log, "").unwrap();
        t.poll(&db).unwrap();
        assert!(gaps(&db).unwrap()[0].2.reason.contains("not found"));
    }

    #[test]
    fn every_core_vote_reject_code_and_reason() {
        let cases: [(&str, Option<Outcome>); 11] = [
            ("bad-mvote", None),
            ("bad-mvote, Proposal not found!", None),
            ("bad-fbvote, Finalized Budget HASH not found!", None),
            ("bad-mvote, new vote is too far ahead of current time - HASH - nTime 1792500000 - Max Time 1792458692", Some(Outcome::RejectFuture(1_792_500_000))),
            ("bad-mvote, new vote older than existing vote - HASH", Some(Outcome::RejectOlder)),
            ("bad-fbvote, time between votes is too soon - HASH - 60 sec < 3600 sec", Some(Outcome::RejectTooSoon(60))),
            ("bad-mvote, masternode (PROTX) not valid or PoSe banned", None),
            ("bad-mvote, unknown masternode - vin: COutPoint(0c58c42e76, 12)", None),
            ("bad-fbvote, masternode not valid", None),
            ("bad-mvote-sig, invalid mvote sig from dmn: PROTX", None),
            ("bad-fbvote-sig, signature from masternode COutPoint(0c58c42e76, 12) invalid", None),
        ];
        for (body, want) in cases {
            let line = format!(
                "2026-10-20T00:11:35Z ProcessMessageInner: {}",
                body.replace("HASH", H1).replace("PROTX", H3)
            );
            let class = if body.starts_with("bad-fb") {
                Class::Finalized
            } else {
                Class::Proposal
            };
            match (parse_line(&line), want) {
                (Line::Event(e), Some(o)) => assert_eq!(
                    (e.class, e.outcome, e.vote, e.seen),
                    (class, Some(o), h(H1), 1),
                    "{line}"
                ),
                (Line::Other(o), None) => {
                    assert_eq!(o.class, class, "{line}");
                    assert!(
                        o.text.starts_with(body.split(',').next().unwrap()),
                        "{line}"
                    );
                }
                (got, _) => panic!("{line} -> {got:?}"),
            }
        }
    }

    #[test]
    fn since_keeps_a_legacy_dbs_oldest_event() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("g").to_string_lossy().into_owned();
        {
            let db = open(&path).unwrap();
            db.delete(SINCE).unwrap();
            db.put(event_key(1_000, &[1; 32]), b"x").unwrap();
        }
        let db = open(&path).unwrap();
        assert_eq!(since(&db).unwrap(), Some(1_000));
        let fresh = tmp.path().join("h").to_string_lossy().into_owned();
        assert!(since(&open(&fresh).unwrap()).unwrap().unwrap() > 1_000);
    }
}
