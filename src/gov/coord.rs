// Groups of masternodes that cast the same set of choices across several
// proposals within seconds of each other. Input is gov.db vote movement (c/
// rows past each proposal's votes_since); output states what was observed,
// never who is behind it: collaterals in a group share no on-chain link.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::gov::store::{Hash, Outpoint, VoteChangeKind};

/// One vote movement as the poller recorded it.
#[derive(Debug, Clone)]
pub struct Vote {
    pub proposal: Hash,
    pub mn: Outpoint,
    pub kind: VoteChangeKind,
    pub from: Option<u8>,
    pub to: u8,
    /// Signed time; the batching key, since poll time only bounds it.
    pub n_time: i64,
    /// Core counts only valid votes; invalid ones move no tally.
    pub f_valid: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub members: Vec<Outpoint>,
    /// Proposal and choice (Core nVote) every member cast.
    pub choices: Vec<(Hash, u8)>,
    pub first: i64,
    pub last: i64,
    /// Per proposal, change in yeas - nays from the group's votes.
    pub swing: Vec<(Hash, i64)>,
    pub new: u32,
    pub changed: u32,
    /// Changed votes whose prior choice is unknown; left out of `swing`.
    pub unknown_from: u32,
}

/// Votes one masternode signs within this many seconds form one batch.
pub const BATCH_GAP: i64 = 120;
/// Smallest group reported; below it identical ballots are common chance.
pub const MIN_GROUP: usize = 10;

fn value(v: u8) -> i64 {
    match v {
        1 => 1,
        2 => -1,
        _ => 0,
    }
}

/// yeas - nays movement of one vote: new yes +1, yes to no -2, yes to abstain -1.
pub fn swing(from: Option<u8>, to: u8) -> i64 {
    value(to) - from.map_or(0, value)
}

/// Grouping is by ballot alone, not time, so members who spread identical
/// ballots over hours still group; `first`/`last` show the real spread. Each
/// member contributes its latest batch per ballot, once.
/// ponytail: exact ballot match only. A group whose members differ by one
/// proposal splits in two; merge near-identical ballots (Jaccard) if real
/// groups show up fragmented.
pub fn groups(votes: &[Vote], batch_gap: i64, min_size: usize) -> Vec<Group> {
    // Resigned and validity flips move no tally and say nothing about intent.
    let mut by_mn: HashMap<Outpoint, Vec<&Vote>> = HashMap::new();
    for v in votes {
        if v.f_valid && matches!(v.kind, VoteChangeKind::New | VoteChangeKind::Changed) {
            by_mn.entry(v.mn).or_default().push(v);
        }
    }
    type Ballot = Vec<(Hash, u8)>;
    let mut by_ballot: HashMap<Ballot, Vec<Vec<&Vote>>> = HashMap::new();
    for (_, mut vs) in by_mn {
        vs.sort_by_key(|v| v.n_time);
        let mut batches: Vec<Vec<&Vote>> = Vec::new();
        for v in vs {
            match batches.last_mut() {
                Some(b) if b.last().is_some_and(|p| v.n_time - p.n_time <= batch_gap) => b.push(v),
                _ => batches.push(vec![v]),
            }
        }
        // Latest batch per ballot; inside a batch, the last vote per proposal.
        let mut latest: HashMap<Ballot, Vec<&Vote>> = HashMap::new();
        for b in batches {
            let mut last: BTreeMap<Hash, &Vote> = BTreeMap::new();
            for v in b {
                last.insert(v.proposal, v);
            }
            if last.len() >= 2 {
                let ballot = last.iter().map(|(h, v)| (*h, v.to)).collect();
                latest.insert(ballot, last.into_values().collect());
            }
        }
        for (ballot, b) in latest {
            by_ballot.entry(ballot).or_default().push(b);
        }
    }
    let mut out: Vec<Group> = by_ballot
        .into_iter()
        .filter_map(|(choices, batches)| {
            let members: BTreeSet<Outpoint> = batches.iter().map(|b| b[0].mn).collect();
            if members.len() < min_size {
                return None;
            }
            let all = batches.iter().flatten();
            let mut swing_by: BTreeMap<Hash, i64> = BTreeMap::new();
            let (mut new, mut changed, mut unknown_from) = (0, 0, 0);
            for v in all.clone() {
                let s = swing_by.entry(v.proposal).or_default();
                match (v.kind, v.from) {
                    (VoteChangeKind::New, _) => {
                        new += 1;
                        *s += swing(None, v.to);
                    }
                    (_, Some(f)) => {
                        changed += 1;
                        *s += swing(Some(f), v.to);
                    }
                    (_, None) => {
                        changed += 1;
                        unknown_from += 1;
                    }
                }
            }
            Some(Group {
                members: members.into_iter().collect(),
                choices,
                first: all.clone().map(|v| v.n_time).min()?,
                last: all.map(|v| v.n_time).max()?,
                swing: swing_by.into_iter().collect(),
                new,
                changed,
                unknown_from,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then(b.last.cmp(&a.last))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gov::store::{parse_hash, parse_mn_id};

    #[test]
    fn swing_table() {
        assert_eq!(swing(Some(1), 2), -2);
        assert_eq!(swing(Some(2), 1), 2);
        assert_eq!(swing(None, 1), 1);
        assert_eq!(swing(None, 2), -1);
        assert_eq!(swing(Some(1), 0), -1);
        assert_eq!(swing(Some(0), 2), -1);
    }

    /// The 2026-10-20 group that took MrAliMMFund01 past Core's heavily-
    /// downvoted line: 213 collaterals, 7 proposals, signed within 88 s.
    #[test]
    fn finds_the_2026_10_20_group() {
        let p = format!(
            "{}/tests/fixtures/gov/bloc_2026-10-20.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let fx: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let mrali =
            parse_hash("71f8dc4e21a11740bc7d9478b3c28b73d622a4262882d7854fd358595ac75aef").unwrap();
        let mut votes: Vec<Vote> = fx["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let proposal = parse_hash(r["proposal"].as_str().unwrap()).unwrap();
                let changed = r["kind"] == "changed";
                Vote {
                    proposal,
                    mn: parse_mn_id(r["mn"].as_str().unwrap()).unwrap(),
                    kind: if changed {
                        VoteChangeKind::Changed
                    } else {
                        VoteChangeKind::New
                    },
                    // Core logs no from-state; MrAli's is proven by its tally
                    // (YES 218 -> 5). Other changed rows stay unknown.
                    from: (proposal == mrali).then_some(1),
                    to: r["to"].as_u64().unwrap() as u8,
                    n_time: r["n_time"].as_i64().unwrap(),
                    f_valid: true,
                }
            })
            .collect();
        // Nine voters sharing a different ballot stay below MIN_GROUP.
        let noise = votes[..7].to_vec();
        for i in 0..9u8 {
            votes.extend(noise.iter().cloned().enumerate().map(|(j, mut v)| {
                v.mn = [i; 36];
                if j == 0 {
                    v.to = 0;
                }
                v
            }));
        }
        // A member re-sending its ballot an hour later counts once (latest).
        let again: Vec<Vote> = votes[7..14]
            .iter()
            .cloned()
            .map(|mut v| {
                v.n_time += 3_600;
                v
            })
            .collect();
        votes.extend(again);
        // An invalid vote moves no tally and joins no group.
        let mut bad = votes[14].clone();
        bad.f_valid = false;
        bad.to = 0;
        votes.push(bad);
        let g = groups(&votes, BATCH_GAP, MIN_GROUP);
        assert_eq!(g.len(), 1, "{g:#?}");
        let g = &g[0];
        assert_eq!(g.members.len(), 213);
        assert_eq!(g.choices.len(), 7);
        // The re-sent ballot replaces that member's first batch, so the span
        // now reaches an hour past the group.
        assert!(g.last - g.first > 3_600, "{}", g.last - g.first);
        assert_eq!((g.new, g.changed), (639, 852));
        // Fixture Changed rows other than MrAli's carry no prior choice.
        assert_eq!(g.unknown_from, 852 - 213);
        let m = g.swing.iter().find(|(h, _)| *h == mrali).unwrap().1;
        assert_eq!(m, -426);
    }
}
