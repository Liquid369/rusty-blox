// gov.db key space and record types.
//
// gov.db is a separate RocksDB instance: vote history comes from RPC polling and
// cannot be rebuilt from the chain, so it must survive a main-DB resync. Keys are
// an ASCII prefix plus fixed-width binary parts; timestamps are big-endian u64 so
// prefix scans return time order. Values are bincode.
//
//   p/H        ProposalMeta      t/H/T    TotalsPoint (only when changed)
//   tl/H       TotalsPoint       latest totals, for change detection
//   v/H/M      VoteState         current vote per collateral
//   c/H/T/M    VoteChange        append-only change log
//   n/T        MnCountPoint      nl  latest MnCountPoint
//   g/T        PollGap           meta/last_poll  PollStatus
//   s/H/T      Visibility        written on change: blind intervals
//   f/F        FinalizedBudget   latest per finalized-budget hash F
//   fv/F/T     FinalizedVotes    only when changed: Core picks the winner per
//                                height by CURRENT VoteCount, so past leads
//                                exist only in this log
//
// H = 32-byte proposal hash, M = collateral txid (32) || vout u32 BE,
// T = unix seconds u64 BE. The d/ prefix is reserved for delivery reports.
//
// meta/schema holds SCHEMA_VERSION (u32 BE); open() refuses a mismatch.
// meta/baselined holds the ts (u64 BE) of the first fully successful poll.
// Every c/ row with T <= that ts is baseline (votes first seen, not changes);
// failed earlier polls can leave several baseline timestamps. Per proposal,
// c/ rows with T <= ProposalMeta.votes_since are first sightings; use that,
// not the global marker, for anything shown as movement.

use rocksdb::{Direction, IteratorMode, DB};
use serde::{Deserialize, Serialize};

pub type Hash = [u8; 32];
pub type Outpoint = [u8; 36];

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProposalMeta {
    pub hash: Hash,
    pub name: String,
    pub url: String,
    pub fee_hash: Hash,
    pub payment_address: String,
    pub monthly_payment_sats: i64,
    pub block_start: u32,
    pub block_end: u32,
    pub total_payment_count: u32,
    pub first_seen: u64,
    pub last_seen: u64,
    /// Set when the node stops knowing the proposal (expired or pruned).
    pub gone_at: Option<u64>,
    pub visibility: Visibility,
    /// ts of the first poll that read this proposal's votes. Its c/ rows at
    /// or before it are first sightings, not movement. None until then.
    pub votes_since: Option<u64>,
}

/// Whether the poller can read a proposal's votes. Core's by-name RPCs
/// resolve only the highest-net proposal of a name (budgetmanager.cpp
/// FindProposalByName), and the no-arg list omits invalid proposals.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Readable,
    /// Listed, but its name resolves elsewhere or nowhere: totals only.
    Shadowed,
    /// Not listed and its name resolves to another hash: deleted or invalid,
    /// cannot tell which. last_seen stops at the last real sighting.
    Unresolvable,
}

/// One `mnfinalbudget show` entry. Core keeps a finalized budget until
/// BlockEnd + 200 (finalizedbudget.cpp updateExpired), and `proposals` is
/// the payment order: block BlockStart + i pays entry i. Names, or a hash
/// for a proposal the node no longer holds; comma-joined by Core, kept raw
/// because names may contain ", " (legal in SanitizeString), so a split is
/// a set of candidate readings, not an identity list. Core builds the string
/// once when the budget is added; a hash token does not mean the proposal
/// is gone now.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FinalizedBudget {
    pub hash: Hash,
    pub name: String,
    pub block_start: u32,
    pub block_end: u32,
    pub proposals: String,
    pub vote_count: u32,
    pub status: String,
    pub is_valid: bool,
    pub first_seen: u64,
    pub last_seen: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FinalizedVotes {
    pub vote_count: u32,
    pub is_valid: bool,
    pub status: String,
}

impl FinalizedBudget {
    pub fn votes(&self) -> FinalizedVotes {
        FinalizedVotes {
            vote_count: self.vote_count,
            is_valid: self.is_valid,
            status: self.status.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TotalsPoint {
    pub yeas: u32,
    pub nays: u32,
    pub abstains: u32,
    pub is_valid: bool,
    pub is_established: bool,
    pub enabled_mns: u32,
    pub remaining_payment_count: i32,
    /// From getbudgetprojection, a separate RPC a few ms after getbudgetinfo:
    /// at an establishment or masternode-count boundary the two can disagree
    /// for one poll. Each field is the node's own answer at its call.
    pub in_projection: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct VoteState {
    /// Core nVote: 0 abstain, 1 yes, 2 no.
    pub vote: u8,
    pub n_time: i64,
    pub n_hash: Hash,
    pub f_valid: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoteChangeKind {
    New,
    Changed,
    /// Same choice re-signed (new nHash). Never counts as movement.
    Resigned,
    ValidityFlip,
    Removed,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VoteChange {
    pub kind: VoteChangeKind,
    pub from: Option<u8>,
    pub to: Option<u8>,
    /// The vote's own signed time; signer-controlled, up to 1h ahead.
    pub n_time: i64,
    pub f_valid: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct MnCountPoint {
    pub total: u32,
    pub stable: u32,
    pub enabled: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PollStatus {
    pub ts: u64,
    pub height: u32,
    pub ok: bool,
    pub proposals: u32,
    pub votes: u32,
    pub ms: u32,
    /// First poll ever: every vote is recorded as New and must not be read as
    /// movement.
    pub baseline: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PollGap {
    pub reason: String,
}

pub const LAST_POLL: &[u8] = b"meta/last_poll";
pub const LAST_MNCOUNT: &[u8] = b"nl";
/// ts (u64 BE) of the poll in progress; cleared by the status write. Present
/// at startup = the previous poll was interrupted.
pub const POLL_STARTED: &[u8] = b"meta/poll_started";

fn key(prefix: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut k = prefix.to_vec();
    for p in parts {
        k.extend_from_slice(p);
    }
    k
}

pub fn proposal_key(h: &Hash) -> Vec<u8> {
    key(b"p/", &[h])
}
pub fn totals_key(h: &Hash, ts: u64) -> Vec<u8> {
    key(b"t/", &[h, &ts.to_be_bytes()])
}
pub fn latest_totals_key(h: &Hash) -> Vec<u8> {
    key(b"tl/", &[h])
}
pub fn vote_prefix(h: &Hash) -> Vec<u8> {
    key(b"v/", &[h])
}
pub fn vote_key(h: &Hash, m: &Outpoint) -> Vec<u8> {
    key(b"v/", &[h, m])
}
pub fn change_key(h: &Hash, ts: u64, m: &Outpoint) -> Vec<u8> {
    key(b"c/", &[h, &ts.to_be_bytes(), m])
}
pub fn mncount_key(ts: u64) -> Vec<u8> {
    key(b"n/", &[&ts.to_be_bytes()])
}
pub fn gap_key(ts: u64) -> Vec<u8> {
    key(b"g/", &[&ts.to_be_bytes()])
}
pub fn visibility_key(h: &Hash, ts: u64) -> Vec<u8> {
    key(b"s/", &[h, &ts.to_be_bytes()])
}
pub fn finalized_key(f: &Hash) -> Vec<u8> {
    key(b"f/", &[f])
}
pub fn finalized_votes_key(f: &Hash, ts: u64) -> Vec<u8> {
    key(b"fv/", &[f, &ts.to_be_bytes()])
}

/// Core's mnId "<txid hex>-<n>" as a fixed-width outpoint. None on any
/// malformation (bad hex, wrong length, missing or non-numeric index).
pub fn parse_mn_id(s: &str) -> Option<Outpoint> {
    let (txid, n) = s.rsplit_once('-')?;
    let txid = hex::decode(txid).ok()?;
    if txid.len() != 32 {
        return None;
    }
    let n: u32 = n.parse().ok()?;
    let mut out = [0u8; 36];
    out[..32].copy_from_slice(&txid);
    out[32..].copy_from_slice(&n.to_be_bytes());
    Some(out)
}

pub fn parse_hash(s: &str) -> Option<Hash> {
    let b = hex::decode(s).ok()?;
    b.try_into().ok()
}

pub fn get<T: for<'de> Deserialize<'de>>(db: &DB, k: &[u8]) -> Result<Option<T>, String> {
    match db.get(k).map_err(|e| e.to_string())? {
        Some(v) => bincode::deserialize(&v).map(Some).map_err(|e| {
            format!(
                "gov.db decode {}: {e}",
                String::from_utf8_lossy(&k[..2.min(k.len())])
            )
        }),
        None => Ok(None),
    }
}

pub fn enc<T: Serialize>(v: &T) -> Result<Vec<u8>, String> {
    bincode::serialize(v).map_err(|e| e.to_string())
}

/// All (key, value) pairs under a prefix, in key order.
pub fn scan(db: &DB, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
    let mut out = Vec::new();
    for item in db.iterator(IteratorMode::From(prefix, Direction::Forward)) {
        let (k, v) = item.map_err(|e| e.to_string())?;
        if !k.starts_with(prefix) {
            break;
        }
        out.push((k.to_vec(), v.to_vec()));
    }
    Ok(out)
}

pub const SCHEMA: &[u8] = b"meta/schema";
/// Bump on any change to a stored struct. bincode has no field tags, so an
/// old record read as a new struct fails to decode, and a decode error in a
/// poll stops the poller: the mismatch must fail open() with a reason instead.
pub const SCHEMA_VERSION: u32 = 1;

/// Opens (creating if needed) the governance store at `path`.
pub fn open(path: &str) -> Result<DB, String> {
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.set_max_open_files(64);
    let db = DB::open(&opts, path).map_err(|e| format!("open gov.db at {path}: {e}"))?;
    let want = SCHEMA_VERSION.to_be_bytes();
    match db.get(SCHEMA).map_err(|e| e.to_string())? {
        None => db.put(SCHEMA, want).map_err(|e| e.to_string())?,
        Some(v) if v.as_slice() == want => {}
        Some(v) => {
            return Err(format!(
                "gov.db at {path} is schema {}, this build expects {SCHEMA_VERSION}: \
                 migrate it, or point paths.gov_db_path at a new store and keep the old one",
                <[u8; 4]>::try_from(v.as_slice())
                    .map(|b| u32::from_be_bytes(b).to_string())
                    .unwrap_or_else(|_| hex::encode(&v))
            ))
        }
    }
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mn_id_round_trip_and_rejects() {
        let tx = "72f5cd6f1def6630a1d0b9425576957e1d3895dbeb6a1d0d484ee814d09eb002";
        let o = parse_mn_id(&format!("{tx}-1")).unwrap();
        assert_eq!(&o[..32], &hex::decode(tx).unwrap()[..]);
        assert_eq!(&o[32..], &1u32.to_be_bytes());
        assert!(parse_mn_id(tx).is_none());
        assert!(parse_mn_id(&format!("{tx}-x")).is_none());
        assert!(parse_mn_id("abcd-1").is_none());
    }

    #[test]
    fn schema_mismatch_refuses_to_open() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().to_str().unwrap();
        open(p).unwrap().put(SCHEMA, 2u32.to_be_bytes()).unwrap();
        let e = open(p).unwrap_err();
        assert!(e.contains("schema 2") && e.contains("expects 1"), "{e}");
    }

    #[test]
    fn time_keys_sort_chronologically() {
        let h = [7u8; 32];
        assert!(totals_key(&h, 255) < totals_key(&h, 256));
        assert!(gap_key(1_000) < gap_key(70_000));
    }
}
