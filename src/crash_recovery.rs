//! Startup self-heal for a hard crash that died mid-block-connect.
//!
//! monitor.rs claims a height with a 'P' (processing) marker before applying it
//! and clears it via an RAII guard on exit. A hard crash (kill -9 / OOM / power
//! loss) skips Drop, so a leftover 'P' marker is the "died mid-block" signal.
//!
//! On re-applying that block, the address index is mostly idempotent — the 't'
//! list and 'a' UTXO set both have existence checks — but the 'r'/'s' totals are
//! NOT (`r += received_delta`), so re-applying double-counts r/s for that block's
//! addresses. We can't subtract the partial pre-crash amount (we don't know how
//! many txs landed), but we CAN recompute each affected address's r/s from the
//! now-idempotently-complete 't'/'a' indexes — bounded to the crashed tail.

use std::collections::HashSet;
use std::sync::Arc;
use rocksdb::DB;

use crate::constants::is_canonical_height;
use crate::parser::{deserialize_transaction, deserialize_utxos};

/// Per-address cap on the recovery recompute's full-history scan. A crashed block
/// that touched a very high-volume address (exchange/treasury) would otherwise make
/// startup recovery deserialize that address's entire history. Above this many txs
/// we skip the recompute and warn — the uncorrected double-count is bounded to the
/// crashed block's contribution and self-heals on the next full enrichment, which is
/// far preferable to a multi-minute startup stall. Reuses the API's env knob.
fn recovery_scan_cap() -> usize {
    std::env::var("RUSTYBLOX_ADDR_MAX_TX_SCAN")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(50_000)
}

/// Scan chain_state for leftover 'P' + height(4 LE) processing markers and return
/// the heights (sorted). Empty on a clean shutdown.
pub fn scan_processing_markers(db: &Arc<DB>) -> Vec<i32> {
    let cf_state = match db.cf_handle("chain_state") {
        Some(cf) => cf,
        None => return Vec::new(),
    };
    let mut heights = Vec::new();
    let iter = db.prefix_iterator_cf(&cf_state, b"P");
    for item in iter {
        let (key, _) = match item {
            Ok(kv) => kv,
            Err(_) => break,
        };
        // prefix_iterator is a seek+scan; stop once we leave the 'P' prefix.
        if key.first() != Some(&b'P') {
            break;
        }
        // A processing marker is exactly 'P' + height(4 LE). Ignore anything else.
        if key.len() == 5 {
            heights.push(i32::from_le_bytes([key[1], key[2], key[3], key[4]]));
        }
    }
    heights.sort_unstable();
    heights
}

/// Recompute (totalReceived, totalSent) for one address from the authoritative
/// 't'/'a' indexes, independent of the (possibly double-counted) persisted r/s.
///
/// totalReceived = sum of every output crediting this address across its 't' list
/// (skipping orphaned txs). balance = sum of its unspent 'a' UTXO values. By the
/// UTXO identity totalSent = totalReceived - balance. This matches the attribution
/// the API and enrichment use (an output credits each address in output.address,
/// so a P2CS owner/staker each count it).
pub async fn recompute_address_rs(
    db: &Arc<DB>,
    address: &str,
) -> Result<(i64, i64), Box<dyn std::error::Error + Send + Sync>> {
    let cf_ai = db.cf_handle("addr_index").ok_or("addr_index CF not found")?;
    let cf_tx = db.cf_handle("transactions").ok_or("transactions CF not found")?;

    // totalReceived: scan the 't' list (display-order 32-byte txids).
    let mut t_key = vec![b't'];
    t_key.extend_from_slice(address.as_bytes());
    let t_list = db.get_cf(&cf_ai, &t_key)?.unwrap_or_default();

    let mut total_received: i64 = 0;
    for txid in t_list.chunks_exact(32) {
        let mut tk = vec![b't'];
        tk.extend_from_slice(txid);
        if let Some(txd) = db.get_cf(&cf_tx, &tk)? {
            if txd.len() >= 8 {
                let h = i32::from_le_bytes([txd[4], txd[5], txd[6], txd[7]]);
                // Skip non-canonical (orphaned/unresolved) txs, exactly as enrichment
                // does via is_canonical_height — so the recomputed total matches the
                // authoritative aggregate.
                if !is_canonical_height(h) {
                    continue;
                }
                let mut with_header = vec![0u8; 4];
                with_header.extend_from_slice(&txd[8..]);
                if let Ok(tx) = deserialize_transaction(&with_header).await {
                    for out in &tx.outputs {
                        if out.address.iter().any(|a| a == address) {
                            total_received += out.value;
                        }
                    }
                }
            }
        }
    }

    // balance: sum the unspent 'a' UTXO output values (includes immature outputs,
    // matching the reference explorer / the API's balance definition).
    let mut a_key = vec![b'a'];
    a_key.extend_from_slice(address.as_bytes());
    let a_data = db.get_cf(&cf_ai, &a_key)?.unwrap_or_default();
    let utxos = deserialize_utxos(&a_data).await;

    let mut balance: i64 = 0;
    for (txid, vout) in &utxos {
        let mut tk = vec![b't'];
        tk.extend_from_slice(txid);
        if let Some(txd) = db.get_cf(&cf_tx, &tk)? {
            if txd.len() >= 8 {
                let mut with_header = vec![0u8; 4];
                with_header.extend_from_slice(&txd[8..]);
                if let Ok(tx) = deserialize_transaction(&with_header).await {
                    if let Some(out) = tx.outputs.get(*vout as usize) {
                        balance += out.value;
                    }
                }
            }
        }
    }

    Ok((total_received, total_received - balance))
}

/// Collect every address involved in a block's transactions (outputs credited +
/// inputs' spent prevout addresses), reading the block's txids from the 'B' index.
async fn block_involved_addresses(
    db: &Arc<DB>,
    height: i32,
) -> Result<HashSet<String>, Box<dyn std::error::Error + Send + Sync>> {
    let cf_tx = db.cf_handle("transactions").ok_or("transactions CF not found")?;

    let mut prefix = vec![b'B'];
    prefix.extend_from_slice(&height.to_le_bytes());

    // 'B' + height(4) + index(8); value is the display-order txid hex string.
    let mut txids: Vec<Vec<u8>> = Vec::new();
    let iter = db.prefix_iterator_cf(&cf_tx, &prefix);
    for item in iter {
        let (key, value) = match item {
            Ok(kv) => kv,
            Err(_) => break,
        };
        if key.len() != 13 || key.first() != Some(&b'B') || key[1..5] != prefix[1..5] {
            break;
        }
        if let Ok(txid) = hex::decode(String::from_utf8_lossy(&value).as_ref()) {
            txids.push(txid);
        }
    }

    let mut addresses: HashSet<String> = HashSet::new();
    for txid in &txids {
        let mut tk = vec![b't'];
        tk.extend_from_slice(txid);
        let txd = match db.get_cf(&cf_tx, &tk)? {
            Some(d) if d.len() >= 8 => d,
            _ => continue,
        };
        let mut with_header = vec![0u8; 4];
        with_header.extend_from_slice(&txd[8..]);
        let tx = match deserialize_transaction(&with_header).await {
            Ok(tx) => tx,
            Err(_) => continue,
        };
        for out in &tx.outputs {
            for a in &out.address {
                if !a.is_empty() {
                    addresses.insert(a.clone());
                }
            }
        }
        for inp in &tx.inputs {
            if inp.coinbase.is_some() {
                continue;
            }
            if let Some(prevout) = &inp.prevout {
                if let Ok(ptxid) = hex::decode(&prevout.hash) {
                    let mut ptk = vec![b't'];
                    ptk.extend_from_slice(&ptxid);
                    if let Some(ptxd) = db.get_cf(&cf_tx, &ptk)? {
                        if ptxd.len() >= 8 {
                            let mut pwh = vec![0u8; 4];
                            pwh.extend_from_slice(&ptxd[8..]);
                            if let Ok(ptx) = deserialize_transaction(&pwh).await {
                                if let Some(pout) = ptx.outputs.get(prevout.n as usize) {
                                    for a in &pout.address {
                                        if !a.is_empty() {
                                            addresses.insert(a.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(addresses)
}

/// Recompute and OVERWRITE r/s for every address involved in `height`'s block,
/// correcting any double-count from re-applying a crash-interrupted block. Returns
/// the number of addresses repaired.
pub async fn repair_block_addresses_rs(
    db: &Arc<DB>,
    height: i32,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let addresses = block_involved_addresses(db, height).await?;
    let cf_ai = db.cf_handle("addr_index").ok_or("addr_index CF not found")?;
    let cap = recovery_scan_cap();

    let mut repaired = 0usize;
    for address in &addresses {
        // Bound the recompute: a pathological high-volume address would force a full
        // history rescan at startup. Skip + warn; the bounded double-count self-heals
        // on the next enrichment.
        let mut t_key = vec![b't'];
        t_key.extend_from_slice(address.as_bytes());
        let tx_count = db.get_cf(&cf_ai, &t_key)?.map(|v| v.len() / 32).unwrap_or(0);
        if tx_count > cap {
            tracing::warn!(address = %address, txs = tx_count, cap,
                "address exceeds crash-recovery scan cap; r/s left as-is (bounded double-count self-heals on next enrichment)");
            continue;
        }

        let (r, s) = recompute_address_rs(db, address).await?;
        let mut rk = vec![b'r'];
        rk.extend_from_slice(address.as_bytes());
        db.put_cf(&cf_ai, &rk, r.to_le_bytes())?;
        let mut sk = vec![b's'];
        sk.extend_from_slice(address.as_bytes());
        db.put_cf(&cf_ai, &sk, s.to_le_bytes())?;
        repaired += 1;
    }
    Ok(repaired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocksdb::{DB, Options};
    use crate::parser::serialize_utxos;

    // The P2PKH address that the script below decodes to (see parser p2pkh_matches_core).
    const ADDR: &str = "DRN9vVxE9WNQM5XxS1RxdfH2NqqKG4VS1A";
    const P2PKH_SCRIPT_HEX: &str = "76a914dddabf603190714c8db2e837e191b83a3a520ba588ac";

    /// Minimal valid PIVX tx body (what is stored after the 8-byte header): one
    /// normal input and one P2PKH output of `value` sats to ADDR.
    fn tx_body(value: i64) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&[0x01, 0x00]); // version u16 = 1 (non-sapling)
        b.extend_from_slice(&[0x00, 0x00]); // tx type u16 = 0
        b.push(0x01); // input count = 1
        b.extend_from_slice(&[0x11u8; 32]); // prevout hash (non-zero => not coinbase)
        b.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // prevout n = 0
        b.push(0x00); // input scriptSig len = 0
        b.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]); // sequence
        b.push(0x01); // output count = 1
        b.extend_from_slice(&value.to_le_bytes()); // value i64 LE
        b.push(0x19); // output script len = 25
        b.extend_from_slice(&hex::decode(P2PKH_SCRIPT_HEX).unwrap());
        b.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // locktime
        b
    }

    fn open(cfs: &[&str]) -> (tempfile::TempDir, std::sync::Arc<DB>) {
        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = std::sync::Arc::new(DB::open_cf(&opts, temp.path(), cfs).unwrap());
        (temp, db)
    }

    #[test]
    fn scan_processing_markers_finds_only_p_height_keys() {
        let (_t, db) = open(&["chain_state"]);
        let cf = db.cf_handle("chain_state").unwrap();
        // Two real processing markers + decoy keys that must be ignored.
        for h in [4242i32, 99i32] {
            let mut k = vec![b'P'];
            k.extend(&h.to_le_bytes());
            db.put_cf(&cf, &k, h.to_le_bytes()).unwrap();
        }
        db.put_cf(&cf, b"sync_height", 1i32.to_le_bytes()).unwrap(); // not 'P'
        db.put_cf(&cf, b"Padding_not_a_marker", b"x").unwrap(); // 'P' but wrong length
        assert_eq!(scan_processing_markers(&db), vec![99, 4242]);
    }

    #[tokio::test]
    async fn recompute_address_rs_sums_received_and_balance() {
        let (_t, db) = open(&["addr_index", "transactions"]);
        let cf_ai = db.cf_handle("addr_index").unwrap();
        let cf_tx = db.cf_handle("transactions").unwrap();

        let txid = vec![0x22u8; 32];
        // tx record: version(4) + height(4) + body. height must not be HEIGHT_ORPHAN.
        let mut rec = vec![1u8, 0, 0, 0];
        rec.extend_from_slice(&7000i32.to_le_bytes());
        rec.extend_from_slice(&tx_body(500_000_000)); // 5 PIV to ADDR
        let mut tkey = vec![b't'];
        tkey.extend_from_slice(&txid);
        db.put_cf(&cf_tx, &tkey, &rec).unwrap();

        // 't' list = [txid]; 'a' set = [(txid,0)] unspent.
        let mut t_list_key = vec![b't'];
        t_list_key.extend_from_slice(ADDR.as_bytes());
        db.put_cf(&cf_ai, &t_list_key, &txid).unwrap();
        let mut a_key = vec![b'a'];
        a_key.extend_from_slice(ADDR.as_bytes());
        db.put_cf(&cf_ai, &a_key, serialize_utxos(&vec![(txid.clone(), 0u64)]).await).unwrap();

        // received = 500M (from the 't' output), balance = 500M (UTXO unspent), sent = 0.
        assert_eq!(recompute_address_rs(&db, ADDR).await.unwrap(), (500_000_000, 0));
    }

    #[tokio::test]
    async fn repair_block_corrects_double_counted_rs() {
        let (_t, db) = open(&["addr_index", "transactions"]);
        let cf_ai = db.cf_handle("addr_index").unwrap();
        let cf_tx = db.cf_handle("transactions").unwrap();
        let height = 7000i32;

        let txid = vec![0x22u8; 32];
        let mut rec = vec![1u8, 0, 0, 0];
        rec.extend_from_slice(&height.to_le_bytes());
        rec.extend_from_slice(&tx_body(500_000_000));
        let mut tkey = vec![b't'];
        tkey.extend_from_slice(&txid);
        db.put_cf(&cf_tx, &tkey, &rec).unwrap();

        // 'B' index entry pointing at the block's tx (display hex value).
        let mut bkey = vec![b'B'];
        bkey.extend(&height.to_le_bytes());
        bkey.extend(&0u64.to_le_bytes());
        db.put_cf(&cf_tx, &bkey, hex::encode(&txid).as_bytes()).unwrap();

        // Idempotent indexes are correct; r/s are DOUBLE-COUNTED (the crash symptom).
        let mut t_list_key = vec![b't'];
        t_list_key.extend_from_slice(ADDR.as_bytes());
        db.put_cf(&cf_ai, &t_list_key, &txid).unwrap();
        let mut a_key = vec![b'a'];
        a_key.extend_from_slice(ADDR.as_bytes());
        db.put_cf(&cf_ai, &a_key, serialize_utxos(&vec![(txid.clone(), 0u64)]).await).unwrap();
        let mut rk = vec![b'r'];
        rk.extend_from_slice(ADDR.as_bytes());
        db.put_cf(&cf_ai, &rk, 1_000_000_000i64.to_le_bytes()).unwrap(); // doubled
        let mut sk = vec![b's'];
        sk.extend_from_slice(ADDR.as_bytes());
        db.put_cf(&cf_ai, &sk, 7_777i64.to_le_bytes()).unwrap(); // garbage

        let repaired = repair_block_addresses_rs(&db, height).await.unwrap();
        assert_eq!(repaired, 1, "one address involved in the block");

        let r = i64::from_le_bytes(db.get_cf(&cf_ai, &rk).unwrap().unwrap().as_slice().try_into().unwrap());
        let s = i64::from_le_bytes(db.get_cf(&cf_ai, &sk).unwrap().unwrap().as_slice().try_into().unwrap());
        assert_eq!((r, s), (500_000_000, 0), "r/s recomputed from t/a, double-count corrected");
    }
}
