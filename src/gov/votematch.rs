// Recovers the masternode behind a vote hash from the sidecar log. Core's vote
// hash covers the collateral input, the object hash, the choice and the signed
// time, but not the signature (budgetvote.cpp:38-46,
// finalizedbudgetvote.cpp:35-42), so a hash names its masternode once the other
// fields are known or few enough to try.
//
// Byte order: gov.db and the sidecar keep hashes and txids in display order
// (hex::decode of what RPC and the log print); Core hashes internal order.

use rocksdb::DB;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use super::store::{scan, Hash, Outpoint, VoteChange, VoteState};

fn rev32(h: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in h[..32].iter().rev().enumerate() {
        out[i] = *b;
    }
    out
}

fn sha256d_display(b: &[u8]) -> Hash {
    let once: [u8; 32] = Sha256::digest(b).into();
    let twice: [u8; 32] = Sha256::digest(&once).into();
    rev32(&twice)
}

/// CTxIn(collateral) as Core serialises it: prevout, empty scriptSig,
/// SEQUENCE_FINAL (primitives/transaction.h:105,108).
fn vin(out: &Outpoint) -> [u8; 41] {
    let mut b = [0u8; 41];
    b[..32].copy_from_slice(&rev32(&out[..32]));
    let n = u32::from_be_bytes(out[32..36].try_into().unwrap_or_default());
    b[32..36].copy_from_slice(&n.to_le_bytes());
    b[36] = 0;
    b[37..41].copy_from_slice(&u32::MAX.to_le_bytes());
    b
}

/// CBudgetVote::GetHash: vin, proposal hash, (int) nVote, int64 nTime.
pub fn vote_hash(out: &Outpoint, proposal: &Hash, vote: u8, n_time: i64) -> Hash {
    let mut b = Vec::with_capacity(41 + 32 + 4 + 8);
    b.extend_from_slice(&vin(out));
    b.extend_from_slice(&rev32(proposal));
    b.extend_from_slice(&i32::from(vote).to_le_bytes());
    b.extend_from_slice(&n_time.to_le_bytes());
    sha256d_display(&b)
}

/// CFinalizedBudgetVote::GetHash: vin, budget hash, int64 nTime (no choice).
pub fn finalized_vote_hash(out: &Outpoint, budget: &Hash, n_time: i64) -> Hash {
    let mut b = Vec::with_capacity(41 + 32 + 8);
    b.extend_from_slice(&vin(out));
    b.extend_from_slice(&rev32(budget));
    b.extend_from_slice(&n_time.to_le_bytes());
    sha256d_display(&b)
}

pub fn outpoint_str(o: &Outpoint) -> String {
    let n = u32::from_be_bytes(o[32..36].try_into().unwrap_or_default());
    format!("{}-{n}", hex::encode(&o[..32]))
}

/// How a masternode was attributed. Each variant is a different strength of
/// evidence and is shown as such, never blended.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Resolution {
    /// The hash is a vote gov.db has already seen, first at `seen_at` (poll ts).
    Known {
        outpoint: String,
        object: String,
        vote: Option<u8>,
        n_time: i64,
        seen_at: Option<u64>,
    },
    /// Exact preimage found by trying candidate fields. `signed` is true only
    /// for too-soon rejections, which Core refuses after the signature check
    /// (budgetmanager.cpp:1157-1162). A future-dated vote is refused before
    /// any signature check (:1099): the match names the collateral it claims.
    Matched {
        outpoint: String,
        object: String,
        vote: Option<u8>,
        n_time: i64,
        signed: bool,
    },
    /// Only a 10-hex collateral prefix was logged; every matching collateral.
    Prefix {
        outpoints: Vec<String>,
    },
    /// The node's own copy of the vote, read back by getdata
    /// (gov::p2pvote). Exact; `canonical` false means a nonstandard vin.
    Recovered {
        outpoint: String,
        object: String,
        vote: u32,
        n_time: i64,
        canonical: bool,
    },
    NotIdentifiable {
        reason: String,
    },
}

/// One bounded search.
#[derive(Debug, PartialEq)]
pub enum Attempt {
    Found(Resolution),
    NotFound,
    /// The per-request hash budget could not cover this search.
    Skipped,
}

/// Everything known about votes and masternodes.
pub struct Index {
    known: HashMap<Hash, (Outpoint, Hash, Option<u8>, i64, Option<u64>)>,
    /// Every (proposal, collateral, signed time) gov.db has held.
    history: Vec<(Hash, Outpoint, i64)>,
    pub collaterals: Vec<Outpoint>,
    pub proposals: Vec<Hash>,
    pub finalized: Vec<Hash>,
}

fn key_outpoint(k: &[u8], at: usize) -> Option<Outpoint> {
    k.get(at..at + 36)?.try_into().ok()
}

impl Index {
    /// From gov.db plus collaterals the node lists now (listmasternodes).
    pub fn build(gov: &DB, listed: &[Outpoint]) -> Result<Index, String> {
        let mut known = HashMap::new();
        let mut history = Vec::new();
        let mut coll: std::collections::BTreeSet<Outpoint> = listed.iter().copied().collect();
        // v/H/M: current votes carry their own hash.
        for (k, v) in scan(gov, b"v/")? {
            let (Some(h), Some(m)) = (
                k.get(2..34).and_then(|b| <Hash>::try_from(b).ok()),
                key_outpoint(&k, 34),
            ) else {
                continue;
            };
            let s: VoteState = bincode::deserialize(&v).map_err(|e| e.to_string())?;
            known
                .entry(s.n_hash)
                .or_insert((m, h, Some(s.vote), s.n_time, None));
            history.push((h, m, s.n_time));
            coll.insert(m);
        }
        // c/H/T/M: earlier votes, hashed from their fields.
        for (k, v) in scan(gov, b"c/")? {
            let (Some(h), Some(t), Some(m)) = (
                k.get(2..34).and_then(|b| <Hash>::try_from(b).ok()),
                k.get(34..42)
                    .and_then(|b| b.try_into().ok())
                    .map(u64::from_be_bytes),
                key_outpoint(&k, 42),
            ) else {
                continue;
            };
            let c: VoteChange = bincode::deserialize(&v).map_err(|e| e.to_string())?;
            let Some(to) = c.to else { continue };
            let vh = vote_hash(&m, &h, to, c.n_time);
            let e = known
                .entry(vh)
                .or_insert((m, h, Some(to), c.n_time, Some(t)));
            e.4 = Some(e.4.map_or(t, |x| x.min(t)));
            history.push((h, m, c.n_time));
            coll.insert(m);
        }
        history.sort_unstable();
        history.dedup();
        let proposals = scan(gov, b"p/")?
            .into_iter()
            .filter_map(|(k, _)| k.get(2..34).and_then(|b| <Hash>::try_from(b).ok()))
            .collect();
        let finalized = scan(gov, b"f/")?
            .into_iter()
            .filter_map(|(k, _)| k.get(2..34).and_then(|b| <Hash>::try_from(b).ok()))
            .collect();
        Ok(Index {
            known,
            history,
            collaterals: coll.into_iter().collect(),
            proposals,
            finalized,
        })
    }

    /// Without gov.db: only listed collaterals, for prefix matches.
    pub fn build_empty(mut listed: Vec<Outpoint>) -> Index {
        listed.sort_unstable();
        listed.dedup();
        Index {
            known: HashMap::new(),
            history: Vec::new(),
            collaterals: listed,
            proposals: Vec::new(),
            finalized: Vec::new(),
        }
    }

    pub fn known(&self, vote: &Hash) -> Option<Resolution> {
        self.known
            .get(vote)
            .map(|(m, h, v, t, seen)| Resolution::Known {
                outpoint: outpoint_str(m),
                object: hex::encode(h),
                vote: *v,
                n_time: *t,
                seen_at: *seen,
            })
    }

    /// Future-time rejection: the log gives nTime; try every collateral,
    /// object and choice. `budget` counts hashes tried across a request.
    pub fn match_time(
        &self,
        vote: &Hash,
        finalized: bool,
        n_time: i64,
        budget: &mut u64,
    ) -> Attempt {
        let per_out = if finalized {
            self.finalized.len() as u64
        } else {
            self.proposals.len() as u64 * 3
        };
        let cost = per_out * self.collaterals.len() as u64;
        if cost > *budget {
            return Attempt::Skipped;
        }
        *budget -= cost;
        for m in &self.collaterals {
            if finalized {
                for f in &self.finalized {
                    if &finalized_vote_hash(m, f, n_time) == vote {
                        return Attempt::Found(matched(m, f, None, n_time, false));
                    }
                }
            } else {
                for p in &self.proposals {
                    for v in 0..3u8 {
                        if &vote_hash(m, p, v, n_time) == vote {
                            return Attempt::Found(matched(m, p, Some(v), n_time, false));
                        }
                    }
                }
            }
        }
        Attempt::NotFound
    }

    /// Too-soon rejection: nTime = the stored vote's time + the logged gap.
    /// Every time gov.db ever held for a (proposal, collateral) is tried,
    /// since the copy is up to one poll older than what the node held.
    pub fn match_gap(&self, vote: &Hash, gap: i64, budget: &mut u64) -> Attempt {
        let cost = self.history.len() as u64 * 3;
        if cost > *budget {
            return Attempt::Skipped;
        }
        *budget -= cost;
        for (p, m, t) in &self.history {
            for v in 0..3u8 {
                if &vote_hash(m, p, v, t + gap) == vote {
                    return Attempt::Found(matched(m, p, Some(v), t + gap, true));
                }
            }
        }
        Attempt::NotFound
    }

    /// "COutPoint(<10 hex>, n)" as Core prints a legacy collateral.
    pub fn prefix(&self, text: &str) -> Option<Resolution> {
        let (_, rest) = text.split_once("COutPoint(")?;
        let (txid10, rest) = rest.split_once(", ")?;
        let n: u32 = rest.split(')').next()?.trim().parse().ok()?;
        if txid10.len() != 10 || !txid10.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let outpoints: Vec<String> = self
            .collaterals
            .iter()
            .filter(|o| {
                hex::encode(&o[..5]) == txid10.to_ascii_lowercase()
                    && u32::from_be_bytes(o[32..36].try_into().unwrap_or_default()) == n
            })
            .map(outpoint_str)
            .collect();
        Some(if outpoints.is_empty() {
            Resolution::NotIdentifiable {
                reason: format!("no known collateral starts with {txid10}:{n}"),
            }
        } else {
            Resolution::Prefix { outpoints }
        })
    }
}

fn matched(m: &Outpoint, object: &Hash, vote: Option<u8>, n_time: i64, signed: bool) -> Resolution {
    Resolution::Matched {
        outpoint: outpoint_str(m),
        object: hex::encode(object),
        vote,
        n_time,
        signed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gov::store::{parse_hash, parse_mn_id};

    fn fx(name: &str) -> serde_json::Value {
        let p = format!("{}/tests/fixtures/gov/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn recomputes_core_vote_hashes_from_a_live_capture() {
        let info = fx("budgetinfo.json");
        let p = info
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["Name"] == "MrAliMMFund01")
            .unwrap();
        let proposal = parse_hash(p["Hash"].as_str().unwrap()).unwrap();
        let votes = fx("budgetvotes_MrAliMMFund01.json");
        let mut n = 0;
        for v in votes.as_array().unwrap() {
            let m = parse_mn_id(v["mnId"].as_str().unwrap()).unwrap();
            let choice = match v["Vote"].as_str().unwrap() {
                "ABSTAIN" => 0,
                "YES" => 1,
                _ => 2,
            };
            let want = parse_hash(v["nHash"].as_str().unwrap()).unwrap();
            assert_eq!(
                vote_hash(&m, &proposal, choice, v["nTime"].as_i64().unwrap()),
                want,
                "{v}"
            );
            n += 1;
        }
        assert_eq!(n, 999);
    }

    #[test]
    fn brute_force_and_prefix_resolution() {
        let m = parse_mn_id("72f5cd6f1def6630a1d0b9425576957e1d3895dbeb6a1d0d484ee814d09eb002-1")
            .unwrap();
        let other =
            parse_mn_id("0c58c42e763de6ec0000000000000000000000000000000000000000000000aa-12")
                .unwrap();
        let p = [7u8; 32];
        let idx = Index {
            known: HashMap::new(),
            history: vec![(p, m, 1_000)],
            collaterals: vec![m, other],
            proposals: vec![[1u8; 32], p],
            finalized: vec![[9u8; 32]],
        };
        let mut budget = 1_000_000;
        let fut = vote_hash(&m, &p, 2, 5_000);
        assert!(matches!(
            idx.match_time(&fut, false, 5_000, &mut budget),
            Attempt::Found(Resolution::Matched {
                vote: Some(2),
                signed: false,
                ..
            })
        ));
        let soon = vote_hash(&m, &p, 1, 1_000 + 1200);
        assert!(matches!(
            idx.match_gap(&soon, 1200, &mut budget),
            Attempt::Found(Resolution::Matched {
                n_time: 2200,
                signed: true,
                ..
            })
        ));
        let fb = finalized_vote_hash(&other, &[9u8; 32], 7);
        assert!(matches!(
            idx.match_time(&fb, true, 7, &mut budget),
            Attempt::Found(Resolution::Matched { vote: None, .. })
        ));
        assert_eq!(
            idx.prefix("signature from masternode COutPoint(0c58c42e76, 12) invalid"),
            Some(Resolution::Prefix {
                outpoints: vec![outpoint_str(&other)]
            })
        );
        // Budget exhausted: no attempt, reported by the caller.
        let mut tiny = 1;
        assert_eq!(
            idx.match_time(&fut, false, 5_000, &mut tiny),
            Attempt::Skipped
        );
    }
}
