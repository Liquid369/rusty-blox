// Pure governance logic: parse getbudgetvotes rows, diff them against stored
// state, and derive pass rules and vote batches. No I/O, so fixtures drive it.

use std::collections::{BTreeMap, HashMap};

use super::store::{parse_hash, parse_mn_id, Outpoint, VoteChange, VoteChangeKind, VoteState};

/// Core nVote values as getbudgetvotes prints them.
pub fn parse_vote(s: &str) -> Option<u8> {
    match s {
        "ABSTAIN" => Some(0),
        "YES" => Some(1),
        "NO" => Some(2),
        _ => None,
    }
}

/// getbudgetvotes array into (collateral, state). Any malformed row fails the
/// whole proposal: a partial set would read as Removed votes.
pub fn parse_votes(v: &serde_json::Value) -> Result<BTreeMap<Outpoint, VoteState>, String> {
    let rows = v.as_array().ok_or("getbudgetvotes: not an array")?;
    let mut out = BTreeMap::new();
    for r in rows {
        let mn = r
            .get("mnId")
            .and_then(|x| x.as_str())
            .and_then(parse_mn_id)
            .ok_or_else(|| format!("getbudgetvotes: bad mnId in {r}"))?;
        let vote = r
            .get("Vote")
            .and_then(|x| x.as_str())
            .and_then(parse_vote)
            .ok_or_else(|| format!("getbudgetvotes: bad Vote in {r}"))?;
        let n_time = r
            .get("nTime")
            .and_then(|x| x.as_i64())
            .ok_or_else(|| format!("getbudgetvotes: bad nTime in {r}"))?;
        let n_hash = r
            .get("nHash")
            .and_then(|x| x.as_str())
            .and_then(parse_hash)
            .ok_or_else(|| format!("getbudgetvotes: bad nHash in {r}"))?;
        let f_valid = r
            .get("fValid")
            .and_then(|x| x.as_bool())
            .ok_or_else(|| format!("getbudgetvotes: bad fValid in {r}"))?;
        // Core emits one row per collateral (it iterates its vote map); a
        // duplicate would silently overwrite and fake a change.
        if out.contains_key(&mn) {
            return Err(format!("getbudgetvotes: duplicate mnId in {r}"));
        }
        out.insert(
            mn,
            VoteState {
                vote,
                n_time,
                n_hash,
                f_valid,
            },
        );
    }
    Ok(out)
}

/// One collateral's transition: the new current state (None = delete) and the
/// log entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub mn: Outpoint,
    pub state: Option<VoteState>,
    pub change: VoteChange,
}

/// Stored vs polled votes. Output is in collateral order, so writes are
/// deterministic; identical input yields no transitions.
pub fn diff_votes(
    prev: &HashMap<Outpoint, VoteState>,
    cur: &BTreeMap<Outpoint, VoteState>,
) -> Vec<Transition> {
    let mut out = Vec::new();
    for (mn, c) in cur {
        let kind = match prev.get(mn) {
            None => Some(VoteChangeKind::New),
            Some(p) if p.n_hash != c.n_hash && p.vote != c.vote => Some(VoteChangeKind::Changed),
            // Same choice under a new signature. Snapshot polling cannot tell a
            // re-sign from two changes that cancel between polls (YES, NO, YES).
            Some(p) if p.n_hash != c.n_hash => Some(VoteChangeKind::Resigned),
            Some(p) if p.f_valid != c.f_valid => Some(VoteChangeKind::ValidityFlip),
            Some(_) => None,
        };
        if let Some(kind) = kind {
            out.push(Transition {
                mn: *mn,
                state: Some(*c),
                change: VoteChange {
                    kind,
                    from: prev.get(mn).map(|p| p.vote),
                    to: Some(c.vote),
                    n_time: c.n_time,
                    f_valid: c.f_valid,
                },
            });
        }
    }
    let mut gone: Vec<_> = prev
        .iter()
        .filter(|(mn, _)| !cur.contains_key(*mn))
        .collect();
    gone.sort_by_key(|(mn, _)| **mn);
    for (mn, p) in gone {
        out.push(Transition {
            mn: *mn,
            state: None,
            change: VoteChange {
                kind: VoteChangeKind::Removed,
                from: Some(p.vote),
                to: None,
                n_time: p.n_time,
                f_valid: p.f_valid,
            },
        });
    }
    out
}

/// Core's funding line: integer `enabled / 10`, and a proposal needs net votes
/// strictly greater (budgetproposal.cpp:211).
pub fn threshold(enabled: u32) -> i64 {
    (enabled / 10) as i64
}

/// Net-down votes beyond this make a started proposal invalid
/// (budgetproposal.cpp:79).
pub fn downvote_invalid_line(enabled: u32) -> i64 {
    (3 * enabled as i64) / 10
}

/// Mainnet budget cycle (chainparams nBudgetCycleBlocks).
pub const MAINNET_CYCLE: u32 = 43_200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassReason {
    Passing,
    Invalid,
    /// Payment window does not cover the whole next cycle.
    OutOfCycle,
    BelowThreshold,
    NotEstablished,
}

/// A proposal's payment window and the chain tip, for Core's cycle check.
#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub block_start: u32,
    pub block_end: u32,
    pub height: u32,
    pub cycle: u32,
}

/// Core CBudgetProposal::IsPassing, same check order, against the cycle that
/// starts at the next superblock `height - height % cycle + cycle`.
pub fn pass_reason(
    yeas: u32,
    nays: u32,
    enabled: u32,
    is_valid: bool,
    established: bool,
    w: Window,
) -> PassReason {
    let sb = w.height - w.height % w.cycle + w.cycle;
    let cycle_end = sb + w.cycle - 1;
    if !is_valid {
        PassReason::Invalid
    } else if w.block_start > sb || w.block_end < cycle_end {
        PassReason::OutOfCycle
    } else if yeas as i64 - nays as i64 <= threshold(enabled) {
        PassReason::BelowThreshold
    } else if !established {
        PassReason::NotEstablished
    } else {
        PassReason::Passing
    }
}

/// Net votes over the funding line; negative means short of it.
pub fn margin(yeas: u32, nays: u32, enabled: u32) -> i64 {
    yeas as i64 - nays as i64 - threshold(enabled)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Batch {
    pub start: i64,
    pub end: i64,
    pub count: u32,
    pub yes: u32,
    pub no: u32,
    pub abstain: u32,
}

/// Runs of votes whose signed times are each within `gap_secs` of the previous
/// one. Signed time, not poll time, so the grouping does not depend on the poll
/// interval.
pub fn batches(votes: &[(i64, u8)], gap_secs: i64) -> Vec<Batch> {
    let mut v = votes.to_vec();
    v.sort_unstable();
    let mut out: Vec<Batch> = Vec::new();
    for (t, vote) in v {
        let extend = out
            .last()
            .is_some_and(|b| t.saturating_sub(b.end) <= gap_secs);
        if !extend {
            out.push(Batch {
                start: t,
                end: t,
                count: 0,
                yes: 0,
                no: 0,
                abstain: 0,
            });
        }
        if let Some(b) = out.last_mut() {
            b.end = t;
            b.count += 1;
            match vote {
                1 => b.yes += 1,
                2 => b.no += 1,
                _ => b.abstain += 1,
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/gov/budgetvotes_MrAliMMFund01.json"
        );
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn st(vote: u8, h: u8, valid: bool) -> VoteState {
        VoteState {
            vote,
            n_time: 100,
            n_hash: [h; 32],
            f_valid: valid,
        }
    }

    #[test]
    fn parses_live_fixture() {
        let v = parse_votes(&fixture()).unwrap();
        assert_eq!(v.len(), 999);
        assert_eq!(v.values().filter(|s| s.vote == 1).count(), 600);
        assert_eq!(v.values().filter(|s| s.vote == 2).count(), 399);
    }

    #[test]
    fn malformed_row_fails_whole_set() {
        let bad = serde_json::json!([{"mnId": "zz-1", "Vote": "YES", "nTime": 1, "nHash": "00", "fValid": true}]);
        assert!(parse_votes(&bad).is_err());
    }

    #[test]
    fn every_change_kind() {
        let (a, b, c, d, e, f) = (
            [1u8; 36], [2u8; 36], [3u8; 36], [4u8; 36], [5u8; 36], [6u8; 36],
        );
        let prev: HashMap<_, _> = [
            (a, st(1, 1, true)), // changed: new hash, new vote
            (b, st(1, 2, true)), // resigned: new hash, same vote
            (c, st(2, 3, true)), // validity flip: same hash
            (d, st(2, 4, true)), // removed
            (e, st(1, 5, true)), // untouched
        ]
        .into();
        let cur: BTreeMap<_, _> = [
            (a, st(2, 11, true)),
            (b, st(1, 12, true)),
            (c, st(2, 3, false)),
            (e, st(1, 5, true)),
            (f, st(0, 6, true)), // new
        ]
        .into();
        let t = diff_votes(&prev, &cur);
        let kinds: Vec<_> = t.iter().map(|x| (x.mn[0], x.change.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                (1, VoteChangeKind::Changed),
                (2, VoteChangeKind::Resigned),
                (3, VoteChangeKind::ValidityFlip),
                (6, VoteChangeKind::New),
                (4, VoteChangeKind::Removed),
            ]
        );
        assert_eq!((t[0].change.from, t[0].change.to), (Some(1), Some(2)));
        assert!(t[4].state.is_none());
    }

    #[test]
    fn identical_input_is_idempotent() {
        let cur = parse_votes(&fixture()).unwrap();
        let prev: HashMap<_, _> = cur.iter().map(|(k, v)| (*k, *v)).collect();
        assert!(diff_votes(&prev, &cur).is_empty());
    }

    // Tip 5,617,473 -> next superblock 5,659,200, cycle runs to 5,702,399.
    const W: Window = Window {
        block_start: 5_616_000,
        block_end: 5_745_603,
        height: 5_617_473,
        cycle: MAINNET_CYCLE,
    };

    #[test]
    fn strict_threshold_and_integer_division() {
        // 2962 enabled -> line 296; net 296 fails, 297 passes.
        assert_eq!(threshold(2962), 296);
        assert_eq!(
            pass_reason(296, 0, 2962, true, true, W),
            PassReason::BelowThreshold
        );
        assert_eq!(
            pass_reason(297, 0, 2962, true, true, W),
            PassReason::Passing
        );
        assert_eq!(margin(297, 0, 2962), 1);
        assert_eq!(
            pass_reason(999, 0, 2962, false, true, W),
            PassReason::Invalid
        );
        assert_eq!(
            pass_reason(999, 0, 2962, true, false, W),
            PassReason::NotEstablished
        );
        assert_eq!(downvote_invalid_line(2962), 888);
    }

    #[test]
    fn cycle_bounds_match_core() {
        // Ends at 5,659,203: covers the superblock but not the cycle end.
        let ends_early = Window {
            block_end: 5_659_203,
            ..W
        };
        assert_eq!(
            pass_reason(999, 0, 2962, true, true, ends_early),
            PassReason::OutOfCycle
        );
        let starts_late = Window {
            block_start: 5_659_201,
            ..W
        };
        assert_eq!(
            pass_reason(999, 0, 2962, true, true, starts_late),
            PassReason::OutOfCycle
        );
        let exact = Window {
            block_start: 5_659_200,
            block_end: 5_702_399,
            ..W
        };
        assert_eq!(
            pass_reason(999, 0, 2962, true, true, exact),
            PassReason::Passing
        );
    }

    #[test]
    fn finds_the_no_burst() {
        let v = parse_votes(&fixture()).unwrap();
        let pts: Vec<_> = v.values().map(|s| (s.n_time, s.vote)).collect();
        let b = batches(&pts, 120);
        let burst = b.iter().find(|x| x.no > 0).unwrap();
        // 2026-10-07 15:09:21..15:11:05 UTC
        assert_eq!((burst.start, burst.end), (1_791_385_761, 1_791_385_865));
        assert_eq!((burst.count, burst.no, burst.yes), (399, 399, 0));
        assert_eq!(b.iter().map(|x| x.count).sum::<u32>(), 999);
    }

    #[test]
    fn duplicate_collateral_rejected_and_extreme_times_do_not_overflow() {
        let row = |t: i64| serde_json::json!({"mnId": format!("{}-1", "ab".repeat(32)), "Vote": "YES", "nTime": t, "nHash": "cd".repeat(32), "fValid": true});
        assert!(parse_votes(&serde_json::json!([row(1), row(2)])).is_err());
        let b = batches(&[(i64::MIN, 1), (i64::MAX, 2)], 120);
        assert_eq!(b.len(), 2);
    }
}
