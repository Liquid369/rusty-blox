// Live governance views read straight from the node and the block index, for
// the overview page until the gov.db read API exists.

use rocksdb::DB;
use serde::Serialize;
use serde_json::Value;

use super::diff::parse_votes;
use super::poller::{is_unknown_proposal, rows, tallies_match, Row, Rpc};
use crate::tx_type::{detect_transaction_type, TransactionType};
use crate::types::CTransaction;

/// Mainnet only. Earliest superblock the mint rule classifies exactly: all 87 from here to
/// 5,443,200 verified (2026-10-19). Earlier eras break it: zPoS coinstakes
/// (1,153,160..1,686,240) have null prevouts; pre-2019 wallets paid a fee out
/// of the stake (excess = payment - fee); 302,400 and 345,600 were funded from
/// the staker's own output (excess <= 0). ponytail: refuse those eras; extend
/// with zPoS denominations and a <= 1 PIV fee band if history before is needed.
pub const FIRST_SUPPORTED_SUPERBLOCK: u32 = 1_728_000;
/// A finalized budget holds at most 100 payments, one per block from the
/// superblock (finalizedbudget.h MAX_PROPOSALS_PER_CYCLE).
pub const MAX_PAYMENTS: u32 = 100;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Payout {
    pub height: u32,
    pub txid: String,
    pub vout: u32,
    pub address: Option<String>,
    #[serde(serialize_with = "sats_str")]
    pub amount_sats: i64,
    #[serde(serialize_with = "sats_str")]
    pub excess_sats: i64,
    /// "paid": minted excess equals the last coinstake output, the layout
    /// Core's block template produces. "unclassified": the block minted more
    /// than the block reward but not as that layout; shown, never dropped.
    pub kind: String,
    pub name: Option<String>,
    pub hash: Option<String>,
    /// Several current proposals share this payee and amount.
    pub candidates: Vec<String>,
}

/// Satoshis go out as strings, the API's rule for money (BigInt-safe).
fn sats_str<S: serde::Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(v)
}

fn parse_record(rec: &[u8]) -> Result<CTransaction, String> {
    // Record = version(4) ++ height(4) ++ raw tx; the parser expects a 4-byte
    // header in front of the raw tx.
    let mut framed = vec![0u8; 4];
    framed.extend_from_slice(&rec[8..]);
    crate::parser::deserialize_transaction_blocking(&framed).map_err(|e| e.to_string())
}

fn read_tx(db: &DB, cf: &rocksdb::ColumnFamily, display_hex: &str) -> Result<CTransaction, String> {
    let bytes = hex::decode(display_hex).map_err(|e| format!("txid {display_hex}: {e}"))?;
    let rec = crate::api::transactions::read_valid_tx_record(db, cf, &bytes)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("tx {display_hex} not indexed"))?;
    parse_record(&rec)
}

/// Budget payments in blocks `sb ..= min(sb + 99, tip)`, from the block index.
/// Any missing index entry, record or prevout is an error: a skipped block
/// would read as "not paid".
pub fn scan_payouts(db: &DB, sb: u32, tip: u32) -> Result<Vec<Payout>, String> {
    let cf = db
        .cf_handle("transactions")
        .ok_or("transactions column family missing")?;
    let mut out = Vec::new();
    for h in sb..=tip.min(sb + MAX_PAYMENTS - 1) {
        let mut prefix = vec![b'B'];
        prefix.extend(&(h as i32).to_le_bytes());
        // The 'B' index has two live key families per height (known key-width
        // debt): idx(4) -> 32 raw bytes in internal order, idx(8) -> 64 hex
        // chars in display order. Both list the same txids, so collect a set.
        let mut txids = std::collections::BTreeSet::new();
        for item in db.prefix_iterator_cf(&cf, &prefix) {
            let (k, v) = item.map_err(|e| e.to_string())?;
            if !k.starts_with(&prefix) {
                break;
            }
            txids.insert(match v.len() {
                32 => hex::encode(v.iter().rev().copied().collect::<Vec<u8>>()),
                64 => std::str::from_utf8(&v)
                    .map_err(|e| format!("height {h}: {e}"))?
                    .to_string(),
                n => return Err(format!("height {h}: block index value of {n} bytes")),
            });
        }
        let seen = txids.len();
        let mut stakes = Vec::new();
        for txid in txids {
            let tx = read_tx(db, cf, &txid)?;
            // B-key order is not vtx order (LE index bytes, degraded rebuilds),
            // so find the coinstake by type, as Core's empty-first-output rule.
            if detect_transaction_type(&tx) == TransactionType::Coinstake {
                stakes.push((txid, tx));
            }
        }
        if seen == 0 {
            return Err(format!("height {h}: no block index entries"));
        }
        let [(txid, cs)] = <[_; 1]>::try_from(stakes)
            .map_err(|v: Vec<_>| format!("height {h}: {} coinstakes", v.len()))?;
        let mut in_sum: i64 = 0;
        for i in &cs.inputs {
            let p = i
                .prevout
                .as_ref()
                .ok_or_else(|| format!("height {h}: coinstake input without prevout"))?;
            let prev = read_tx(db, cf, &p.hash)?;
            in_sum += prev
                .outputs
                .get(p.n as usize)
                .ok_or_else(|| format!("height {h}: prevout {}:{} out of range", p.hash, p.n))?
                .value;
        }
        let out_sum: i64 = cs.outputs.iter().map(|o| o.value).sum();
        // PoS fees are destroyed, not paid to the staker, and a budget block's
        // staker keeps the masternode share, so the excess is the payment.
        let excess = out_sum - in_sum - crate::emission::era_block_reward(h as i32);
        if excess <= 0 {
            continue;
        }
        let last = cs
            .outputs
            .last()
            .ok_or_else(|| format!("height {h}: empty coinstake"))?;
        out.push(Payout {
            height: h,
            txid,
            vout: (cs.outputs.len() - 1) as u32,
            address: last.address.first().cloned(),
            amount_sats: last.value,
            excess_sats: excess,
            kind: if excess == last.value {
                "paid"
            } else {
                "unclassified"
            }
            .to_string(),
            name: None,
            hash: None,
            candidates: Vec::new(),
        });
    }
    Ok(out)
}

/// Names each paid row by the one current proposal whose payee, amount and
/// payment window match. The window check keeps a recurring proposal that
/// reuses payee and amount off an older cycle's payment. Unmatched rows stay
/// unnamed: the payee address is then their identity.
pub(crate) fn name_payouts(payouts: &mut [Payout], proposals: &[Row], sb: u32) {
    for p in payouts.iter_mut().filter(|p| p.kind == "paid") {
        let m: Vec<&Row> = proposals
            .iter()
            .filter(|r| {
                Some(&r.payment_address) == p.address.as_ref()
                    && r.monthly_payment_sats == p.amount_sats
                    && r.block_start <= sb
                    && sb <= r.block_end
            })
            .collect();
        match m.as_slice() {
            [one] => {
                p.name = Some(one.name.clone());
                p.hash = Some(hex::encode(one.hash));
            }
            many => p.candidates = many.iter().map(|r| r.name.clone()).collect(),
        }
    }
}

#[derive(Serialize, Debug, Clone, Default, PartialEq)]
pub struct Counts {
    pub yes: u32,
    pub no: u32,
    pub abstain: u32,
}

#[derive(Serialize, Debug, Clone)]
pub struct VoteCount {
    pub hash: String,
    pub name: String,
    /// False when the name resolves to another proposal or the votes moved
    /// during the read; counts are then absent, never another proposal's.
    pub readable: bool,
    pub reason: Option<String>,
    /// Every vote the node holds, valid or not.
    pub held: Option<Counts>,
    /// Votes Core counts: those whose masternode the node sees as enabled.
    pub counting: Option<Counts>,
}

async fn owner(rpc: &Rpc, name: &str) -> Result<Option<[u8; 32]>, String> {
    match rpc("getbudgetinfo", Value::Array(vec![name.into()])).await {
        Ok(v) => {
            let mut gaps = Vec::new();
            let r = rows(&v, "getbudgetinfo(name)", &mut gaps);
            match (gaps.is_empty(), r.first()) {
                (true, Some(o)) => Ok(Some(o.hash)),
                _ => Err(format!("getbudgetinfo({name}): malformed response")),
            }
        }
        Err(e) if is_unknown_proposal(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Held vs counting votes per listed proposal. Same identity checks as the
/// poller: owner by name before and after the read, and the valid votes must
/// reproduce the listed tallies.
pub async fn vote_counts(rpc: &Rpc) -> Result<Vec<VoteCount>, String> {
    let list = rpc("getbudgetinfo", Value::Array(vec![])).await?;
    let mut gaps = Vec::new();
    let listed = rows(&list, "getbudgetinfo", &mut gaps);
    if !gaps.is_empty() {
        return Err(gaps.join("; "));
    }
    let mut out = Vec::with_capacity(listed.len());
    for r in &listed {
        let mut row = VoteCount {
            hash: hex::encode(r.hash),
            name: r.name.clone(),
            readable: false,
            reason: None,
            held: None,
            counting: None,
        };
        if owner(rpc, &r.name).await? != Some(r.hash) {
            row.reason = Some("name resolves to another proposal".into());
            out.push(row);
            continue;
        }
        let votes =
            parse_votes(&rpc("getbudgetvotes", Value::Array(vec![r.name.clone().into()])).await?)?;
        if !tallies_match(&votes, r) || owner(rpc, &r.name).await? != Some(r.hash) {
            row.reason = Some("votes changed during the read".into());
            out.push(row);
            continue;
        }
        let (mut held, mut counting) = (Counts::default(), Counts::default());
        for s in votes.values() {
            for c in [Some(&mut held), s.f_valid.then_some(&mut counting)]
                .into_iter()
                .flatten()
            {
                match s.vote {
                    1 => c.yes += 1,
                    2 => c.no += 1,
                    _ => c.abstain += 1,
                }
            }
        }
        row.readable = true;
        row.held = Some(held);
        row.counting = Some(counting);
        out.push(row);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gov::poller::RpcFut;
    use std::sync::Arc;

    fn fx(name: &str) -> Value {
        let p = format!("{}/tests/fixtures/gov/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    /// getbudgetinfo list from the fixture, other proposals' tallies zeroed to
    /// match their empty vote sets; by-name lookups pick the highest net.
    fn node(info: Value, votes: Value) -> Box<Rpc> {
        let info = Arc::new(info);
        let votes = Arc::new(votes);
        Box::new(move |m: &'static str, p: Value| -> RpcFut {
            let info = Arc::clone(&info);
            let votes = Arc::clone(&votes);
            Box::pin(async move {
                let arg = p.get(0).and_then(|x| x.as_str()).map(str::to_string);
                match (m, arg) {
                    ("getbudgetinfo", None) => Ok((*info).clone()),
                    ("getbudgetinfo", Some(n)) => info
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|r| r["Name"] == n.as_str())
                        .max_by_key(|r| r["Yeas"].as_i64().unwrap() - r["Nays"].as_i64().unwrap())
                        .map(|r| Value::Array(vec![r.clone()]))
                        .ok_or_else(|| "Unknown proposal name".to_string()),
                    ("getbudgetvotes", Some(n)) if n == "MrAliMMFund01" => Ok((*votes).clone()),
                    ("getbudgetvotes", Some(_)) => Ok(Value::Array(vec![])),
                    _ => Err(format!("unexpected {m}")),
                }
            })
        })
    }

    fn info() -> Value {
        let mut info = fx("budgetinfo.json");
        for r in info.as_array_mut().unwrap() {
            if r["Name"] != "MrAliMMFund01" {
                r["Yeas"] = 0.into();
                r["Nays"] = 0.into();
                r["Abstains"] = 0.into();
            }
        }
        info
    }

    #[tokio::test]
    async fn counts_held_and_counting_votes() {
        let mut votes = fx("budgetvotes_MrAliMMFund01.json");
        // Two NO votes the node holds but does not count (masternode not
        // enabled); a real node's listed tallies count valid votes only.
        let mut flipped = 0;
        for v in votes.as_array_mut().unwrap() {
            if flipped < 2 && v["Vote"] == "NO" {
                v["fValid"] = false.into();
                flipped += 1;
            }
        }
        let mut info = info();
        for r in info.as_array_mut().unwrap() {
            if r["Name"] == "MrAliMMFund01" {
                r["Nays"] = (r["Nays"].as_i64().unwrap() - 2).into();
            }
        }
        let rows = vote_counts(&*node(info, votes)).await.unwrap();
        let mrali = rows.iter().find(|r| r.name == "MrAliMMFund01").unwrap();
        assert!(mrali.readable);
        let (held, counting) = (mrali.held.clone().unwrap(), mrali.counting.clone().unwrap());
        assert_eq!((held.yes, held.no), (600, 399));
        assert_eq!((counting.yes, counting.no), (600, 397));
    }

    #[tokio::test]
    async fn shared_name_never_borrows_another_proposals_votes() {
        let mut info = info();
        let mut dup = info
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["Name"] == "MrAliMMFund01")
            .unwrap()
            .clone();
        dup["Hash"] = "cd".repeat(32).into();
        dup["Yeas"] = 0.into();
        dup["Nays"] = 0.into();
        info.as_array_mut().unwrap().push(dup);
        let rows = vote_counts(&*node(info, fx("budgetvotes_MrAliMMFund01.json")))
            .await
            .unwrap();
        let dup = rows.iter().find(|r| r.hash == "cd".repeat(32)).unwrap();
        assert!(!dup.readable && dup.held.is_none());
    }

    fn row(addr: &str, sats: i64, name: &str) -> Row {
        let mut info = info();
        let r = &mut info.as_array_mut().unwrap()[0];
        r["PaymentAddress"] = addr.into();
        r["MonthlyPayment"] = (sats as f64 / 1e8).into();
        r["Name"] = name.into();
        let mut gaps = Vec::new();
        rows(&Value::Array(vec![r.clone()]), "t", &mut gaps).remove(0)
    }

    fn payout(addr: &str, sats: i64, kind: &str) -> Payout {
        Payout {
            height: 1,
            txid: String::new(),
            vout: 2,
            address: Some(addr.into()),
            amount_sats: sats,
            excess_sats: sats,
            kind: kind.into(),
            name: None,
            hash: None,
            candidates: Vec::new(),
        }
    }

    #[test]
    fn payouts_named_only_by_a_unique_payee_and_amount() {
        let props = [
            row("DA", 7_000_000_000_000, "A"),
            row("DB", 100, "B"),
            row("DB", 100, "B2"),
        ];
        let mut p = [
            payout("DA", 7_000_000_000_000, "paid"),
            payout("DA", 1, "paid"),
            payout("DB", 100, "paid"),
            payout("DA", 7_000_000_000_000, "unclassified"),
        ];
        name_payouts(&mut p, &props, props[0].block_start);
        assert_eq!(p[0].name.as_deref(), Some("A"));
        assert_eq!(
            p[1].name, None,
            "wrong amount: unnamed, address is its identity"
        );
        assert_eq!((p[2].name.clone(), p[2].candidates.len()), (None, 2));
        assert_eq!(p[3].name, None, "unclassified rows are never named");
    }

    #[test]
    fn payout_outside_a_proposals_window_is_not_named_after_it() {
        let r = row("DA", 100, "A");
        let mut p = [payout("DA", 100, "paid")];
        name_payouts(&mut p, std::slice::from_ref(&r), r.block_start - 43_200);
        assert_eq!(p[0].name, None, "same payee and amount, earlier cycle");
    }

    /// Prints the payouts of SB (env GOV_SB) from the local DB, to diff against
    /// another explorer's block data for the same heights.
    #[test]
    #[ignore = "needs a main DB at config paths.db_path"]
    fn print_superblock() {
        let _ = crate::config::init_global_config();
        let path = crate::config::get_global_config()
            .get_string("paths.db_path")
            .unwrap();
        let cfs = DB::list_cf(&rocksdb::Options::default(), &path).unwrap();
        let db =
            DB::open_cf_for_read_only(&rocksdb::Options::default(), &path, &cfs, false).unwrap();
        let tip = crate::chain_state::get_chain_state(&Arc::new(db))
            .unwrap()
            .height as u32;
        let sb: u32 = std::env::var("GOV_SB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(tip - tip % 43_200);
        eprintln!("tip {tip} sb {sb}");
        let db =
            DB::open_cf_for_read_only(&rocksdb::Options::default(), &path, &cfs, false).unwrap();
        for p in scan_payouts(&db, sb, tip).unwrap() {
            eprintln!(
                "PAYOUT {} {} {} {} {}",
                p.height,
                p.address.unwrap_or_default(),
                p.amount_sats,
                p.excess_sats,
                p.kind
            );
        }
    }

    /// Reference: SB 5,616,000 paid 11 proposals in blocks 5,616,000..=5,616,010
    /// (read from explorer.pivx.org block-detail, 2026-10-19).
    /// cargo test --lib gov::live::tests::superblock_oracle -- --ignored --nocapture
    #[test]
    #[ignore = "needs a synced main DB at config paths.db_path"]
    fn superblock_oracle() {
        let _ = crate::config::init_global_config();
        let path = crate::config::get_global_config()
            .get_string("paths.db_path")
            .unwrap();
        let cfs = DB::list_cf(&rocksdb::Options::default(), &path).unwrap();
        let db =
            DB::open_cf_for_read_only(&rocksdb::Options::default(), &path, &cfs, false).unwrap();
        let got = scan_payouts(&db, 5_616_000, 5_616_099).unwrap();
        let want: [(u32, &str, i64); 11] = [
            (5_616_000, "DMmVeKPyHwSu22Lp8iYWUnvoiFdfCH2zLs", 70_000),
            (5_616_001, "DRCftD2Xqmoud7x5EdhnBnMjof5qfE4rEn", 45_000),
            (5_616_002, "DLabsktzGMnsK5K9uRTMCF6NoYNY6ET4Bb", 16_500),
            (5_616_003, "DBh9o9uRGohcDKpeiEyRiwtmTaTL3xDdev", 20_000),
            (5_616_004, "DRENSLHv5PVwQtoitDwEyoATGZbfBqeHoF", 55_000),
            (5_616_005, "DMG765eiDD9cGgnDNeQEJGjNYQDy4VjxaX", 45_000),
            (5_616_006, "DDgF6BcoC93kF3GeTeP2F7tt6c3KuT2UKi", 20_050),
            (5_616_007, "DE4gavfKV3KE7naHvhaD9zZUnznF8tF6k9", 85_400),
            (5_616_008, "DJ8J3qyjSJMk3oc2Pqz632b9PepWxZfg2P", 45_000),
            (5_616_009, "DHfQokecuNZjEkzaZJ5KMKUc1ZLievPYJa", 15_000),
            (5_616_010, "DU2N8FoczSsLVphR7pZC1nxBMvNdibr9C2", 15_000),
        ];
        let got: Vec<(u32, String, i64, String)> = got
            .into_iter()
            .map(|p| {
                (
                    p.height,
                    p.address.unwrap_or_default(),
                    p.amount_sats,
                    p.kind,
                )
            })
            .collect();
        let want: Vec<(u32, String, i64, String)> = want
            .iter()
            .map(|(h, a, piv)| (*h, a.to_string(), piv * 100_000_000, "paid".to_string()))
            .collect();
        assert_eq!(got, want);
    }
}
