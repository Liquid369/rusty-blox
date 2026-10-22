// Condenses rejected votes for the Vote Log: clusters of identical rejections
// (same reason, proposal, choice and evidence) and the collateral funding
// transactions behind them. Pure; the API resolves each rejection first.

use std::collections::{BTreeMap, BTreeSet};

use crate::gov::store::{Hash, Outpoint};

/// How strongly a rejection is tied to its collateral's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// Signature verified; signed at or after the stored vote and within an
    /// hour of it (Core's too-soon test, budgetproposal.cpp:237).
    Signed,
    /// Exactly these vote fields are already in gov.db history. The hash
    /// excludes the signature (budgetvote.cpp:38), so this cannot tell a
    /// re-send from a re-signing of identical fields.
    Seen,
    /// Signature verified, signed time older than the stored vote: a re-sent
    /// old vote or a newly signed backdated one; the data cannot tell which.
    Older,
    /// Names the collateral; the signature was never verified.
    Claimed,
}

#[derive(Debug, Clone)]
pub struct Reject {
    pub reason: String,
    pub object: Option<Hash>,
    pub choice: Option<u32>,
    pub evidence: Option<Evidence>,
    pub collaterals: Vec<Outpoint>,
    /// Arrival (log) time.
    pub ts: u64,
    /// Signed time, when the vote is known.
    pub n_time: Option<i64>,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FundingGroup {
    pub txid: String,
    pub vouts: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Cluster {
    pub reason: String,
    pub object: Option<String>,
    pub choice: Option<u32>,
    pub evidence: Option<Evidence>,
    pub count: u32,
    pub masternodes: usize,
    pub first: u64,
    pub last: u64,
    pub signed_from: Option<i64>,
    pub signed_to: Option<i64>,
    /// Members by collateral funding transaction, largest first, cut at the
    /// caller's group and vout caps.
    pub funding: Vec<FundingGroup>,
    pub funding_total: usize,
    pub vouts_total: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FundingRow {
    pub txid: String,
    /// Cut at the caller's running vout cap; `vouts_total` is the full count.
    pub vouts: Vec<u32>,
    pub vouts_total: usize,
    pub signed: u32,
    pub seen: u32,
    pub older: u32,
    pub claimed: u32,
    pub untraced: u32,
    pub last: u64,
}

fn split(o: &Outpoint) -> (Hash, u32) {
    let mut t = [0u8; 32];
    t.copy_from_slice(&o[..32]);
    (t, u32::from_be_bytes([o[32], o[33], o[34], o[35]]))
}

/// Funding groups largest first, stopping at `cap` groups or once `vouts_cap`
/// vouts are emitted (the group that crosses it is cut).
fn groups(
    members: &BTreeSet<Outpoint>,
    cap: usize,
    vouts_cap: usize,
) -> (Vec<FundingGroup>, usize) {
    let mut by_tx: BTreeMap<Hash, Vec<u32>> = BTreeMap::new();
    for m in members {
        let (t, n) = split(m);
        by_tx.entry(t).or_default().push(n);
    }
    let total = by_tx.len();
    let mut g: Vec<FundingGroup> = by_tx
        .into_iter()
        .map(|(t, vouts)| FundingGroup {
            txid: hex::encode(t),
            vouts,
        })
        .collect();
    g.sort_by(|a, b| b.vouts.len().cmp(&a.vouts.len()).then(a.txid.cmp(&b.txid)));
    g.truncate(cap);
    let mut left = vouts_cap;
    g.retain_mut(|x| {
        if left == 0 {
            return false;
        }
        x.vouts.truncate(left);
        left -= x.vouts.len();
        true
    });
    (g, total)
}

/// Clusters keyed (reason, object, choice, evidence), most rejections first,
/// each listing at most `groups_cap` funding transactions.
pub fn clusters(rows: &[Reject], groups_cap: usize, vouts_cap: usize) -> Vec<Cluster> {
    type Key = (String, Option<Hash>, Option<u32>, Option<Evidence>);
    let mut acc: BTreeMap<Key, (u32, BTreeSet<Outpoint>, u64, u64, Option<i64>, Option<i64>)> =
        BTreeMap::new();
    for r in rows {
        let e = acc
            .entry((r.reason.clone(), r.object, r.choice, r.evidence))
            .or_insert((0, BTreeSet::new(), u64::MAX, 0, None, None));
        e.0 += r.count.max(1);
        e.1.extend(r.collaterals.iter().copied());
        e.2 = e.2.min(r.ts);
        e.3 = e.3.max(r.ts);
        if let Some(t) = r.n_time {
            e.4 = Some(e.4.map_or(t, |x: i64| x.min(t)));
            e.5 = Some(e.5.map_or(t, |x: i64| x.max(t)));
        }
    }
    let mut out: Vec<Cluster> = acc
        .into_iter()
        .map(
            |((reason, object, choice, evidence), (count, members, first, last, sf, st))| {
                let (funding, funding_total) = groups(&members, groups_cap, vouts_cap);
                Cluster {
                    reason,
                    object: object.map(hex::encode),
                    choice,
                    evidence,
                    count,
                    masternodes: members.len(),
                    first,
                    last,
                    signed_from: sf,
                    signed_to: st,
                    funding,
                    funding_total,
                    vouts_total: members.len(),
                }
            },
        )
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(b.last.cmp(&a.last)));
    out
}

/// One row per collateral funding transaction across all rejections, at most
/// `rows_cap` rows and `vouts_cap` vouts in total (the row crossing it is cut).
pub fn funding(rows: &[Reject], rows_cap: usize, vouts_cap: usize) -> Vec<FundingRow> {
    let mut acc: BTreeMap<Hash, (BTreeSet<u32>, [u32; 5], u64)> = BTreeMap::new();
    for r in rows {
        for m in &r.collaterals {
            let (t, n) = split(m);
            let e = acc.entry(t).or_insert((BTreeSet::new(), [0; 5], 0));
            e.0.insert(n);
            let i = match r.evidence {
                Some(Evidence::Signed) => 0,
                Some(Evidence::Seen) => 1,
                Some(Evidence::Older) => 2,
                Some(Evidence::Claimed) => 3,
                None => 4,
            };
            e.1[i] += r.count.max(1);
            e.2 = e.2.max(r.ts);
        }
    }
    let mut out: Vec<FundingRow> = acc
        .into_iter()
        .map(|(t, (vouts, c, last))| FundingRow {
            txid: hex::encode(t),
            vouts_total: vouts.len(),
            vouts: vouts.into_iter().collect(),
            signed: c[0],
            seen: c[1],
            older: c[2],
            claimed: c[3],
            untraced: c[4],
            last,
        })
        .collect();
    out.sort_by(|a, b| {
        b.vouts_total
            .cmp(&a.vouts_total)
            .then(
                (b.signed + b.seen + b.older + b.claimed)
                    .cmp(&(a.signed + a.seen + a.older + a.claimed)),
            )
            .then(b.last.cmp(&a.last))
    });
    out.truncate(rows_cap);
    let mut left = vouts_cap;
    out.retain_mut(|r| {
        if left == 0 {
            return false;
        }
        r.vouts.truncate(left);
        left -= r.vouts.len();
        true
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(tx: u8, n: u32) -> Outpoint {
        let mut o = [0u8; 36];
        o[..32].copy_from_slice(&[tx; 32]);
        o[32..].copy_from_slice(&n.to_be_bytes());
        o
    }

    fn rej(
        reason: &str,
        obj: u8,
        ev: Option<Evidence>,
        mn: Outpoint,
        ts: u64,
        n_time: i64,
    ) -> Reject {
        Reject {
            reason: reason.into(),
            object: Some([obj; 32]),
            choice: Some(1),
            evidence: ev,
            collaterals: vec![mn],
            ts,
            n_time: Some(n_time),
            count: 1,
        }
    }

    #[test]
    fn clusters_and_funding() {
        let older = "older than the stored vote";
        let rows = vec![
            // Three outputs of one funding tx replay the same stale ballot.
            rej(older, 7, Some(Evidence::Older), op(1, 214), 100, 10),
            rej(older, 7, Some(Evidence::Older), op(1, 158), 105, 12),
            rej(older, 7, Some(Evidence::Older), op(1, 218), 110, 11),
            rej(older, 7, Some(Evidence::Older), op(2, 0), 120, 9),
            // The same collateral twice counts twice, member once.
            rej(older, 7, Some(Evidence::Older), op(1, 214), 130, 10),
            // A different proposal is a different cluster.
            rej(
                "under 1 h after the stored vote",
                8,
                Some(Evidence::Signed),
                op(3, 1),
                140,
                50,
            ),
        ];
        let c = clusters(&rows, 10, 100);
        assert_eq!(c.len(), 2);
        let a = &c[0];
        assert_eq!((a.count, a.masternodes, a.first, a.last), (5, 4, 100, 130));
        assert_eq!((a.signed_from, a.signed_to), (Some(9), Some(12)));
        assert_eq!(a.funding_total, 2);
        assert_eq!(a.funding[0].vouts, vec![158, 214, 218]);
        assert_eq!(clusters(&rows, 1, 100)[0].funding.len(), 1);
        // The vout cap cuts the group that crosses it, then stops.
        let capped = &clusters(&rows, 10, 2)[0];
        assert_eq!(
            (
                capped.funding.len(),
                capped.funding[0].vouts.len(),
                capped.vouts_total
            ),
            (1, 2, 4)
        );

        let f = funding(&rows, 10, 100);
        assert_eq!(funding(&rows, 1, 100).len(), 1);
        let cut = funding(&rows, 10, 2);
        // The cap stops the table: no row is emitted with an empty vout list.
        assert_eq!((cut.len(), cut[0].vouts.clone()), (1, vec![158, 214]));
        assert_eq!(f.len(), 3);
        assert_eq!((f[0].vouts_total, f[0].older, f[0].last), (3, 4, 130));
        assert_eq!(
            f.iter().find(|r| r.signed == 1).map(|r| r.vouts.clone()),
            Some(vec![1])
        );
    }
}
