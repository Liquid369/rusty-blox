/// Address Enrichment Module - Transaction Database Approach
///
/// **Purpose:** Builds address index from our RocksDB transaction database
///
/// **When to use:**
/// - Normal sync operations (automatically called after fast_sync)
/// - Incremental address index rebuilding
/// - Recovery when address index is corrupted but transactions are intact
///
/// **Algorithm:**
/// - Pass 1: Scan all transactions to identify spent outputs
/// - Pass 2: Index only UNSPENT outputs per address
///
/// **Advantages:**
/// - Works with our own database (no dependency on PIVX Core)
use crate::constants::should_index_transaction;
use crate::metrics;
use crate::parser::{
    deserialize_transaction, deserialize_transaction_blocking, serialize_addr_txs,
    serialize_addr_utxos,
};
use crate::tx_keys::{txid_from_hex, txid_from_key};
use crate::types::{CTransaction, CTxOut, ScriptClassification};
use rocksdb::DB;
use std::collections::{HashMap, HashSet};
/// - Fast incremental updates
/// - Proper UTXO tracking (spent vs unspent)
///
/// **Alternative Approach:**
/// See `enrich_from_chainstate.rs` for verification using PIVX Core's chainstate
/// as the authoritative source of truth. That approach is best for one-time
/// verification or recovery but requires PIVX Core to be stopped.
use std::sync::Arc;
use tracing::{info, info_span, warn};

/// Classify output script for correct PIVX Core attribution.
/// Classification is driven by the SCRIPT structure (like PIVX Core's Solver()),
/// not by guessing from base58 prefixes of the extracted address strings.
pub(crate) fn classify_output(output: &CTxOut) -> ScriptClassification {
    if output.address.is_empty() {
        return ScriptClassification::Nonstandard;
    }

    // Check for special markers
    if output.address.iter().any(|a| a == "CoinBaseTx") {
        return ScriptClassification::Coinbase;
    }
    if output.address.iter().any(|a| a == "CoinStakeTx") {
        return ScriptClassification::Coinstake;
    }
    if output.address.iter().any(|a| a == "Nonstandard") {
        return ScriptClassification::Nonstandard;
    }

    // OP_RETURN check (empty script or starts with 0x6a)
    if output.value == 0
        && (output.script_pubkey.script.is_empty() || output.script_pubkey.script[0] == 0x6a)
    {
        return ScriptClassification::OpReturn;
    }

    match crate::parser::get_script_type(&output.script_pubkey.script) {
        "coldstake" if output.address.len() == 2 => ScriptClassification::ColdStake {
            staker: output.address[0].clone(),
            owner: output.address[1].clone(),
        },
        "pubkeyhash" | "exchangeaddress" if output.address.len() == 1 => {
            ScriptClassification::P2PKH(output.address[0].clone())
        }
        "scripthash" if output.address.len() == 1 => {
            ScriptClassification::P2SH(output.address[0].clone())
        }
        "pubkey" if output.address.len() == 1 => {
            ScriptClassification::P2PK(output.address[0].clone())
        }
        _ => {
            // Unknown script with a single extracted address — attribute it rather
            // than silently dropping value from the index.
            if output.address.len() == 1 {
                ScriptClassification::P2PKH(output.address[0].clone())
            } else {
                ScriptClassification::Nonstandard
            }
        }
    }
}

/// Packed, deserialization-free representation of a single transaction output.
///
/// Built ONCE in Pass 1 by running the SAME `classify_output` logic that Pass 2
/// used to run, so Pass 2 / Pass 2b / the HODL snapshot can read attribution and
/// value directly without ever re-deserializing or re-classifying. Addresses are
/// already interned to dense u32 ids (Step 3); `value`/`vout` are copied verbatim
/// from the source `CTxOut` so every packed value is byte-identical to the output.
///
/// `kind`:
///   0 = None      — no address attribution (OP_RETURN, zerocoin mint, the empty
///                   coinstake vout[0] marker, nonstandard). Both ids are NO_ADDR.
///   1 = Single    — P2PKH / P2SH / P2PK / unknown-single / EXM exchangeaddress.
///                   addr_a = the single id, addr_b = NO_ADDR.
///   2 = ColdStake — P2CS. addr_a = staker id, addr_b = owner id.
#[derive(Clone)]
struct PackedOut {
    value: i64,
    addr_a: u32,
    addr_b: u32,
    vout: u32,
    kind: u8,
}

/// Packed, deserialization-free representation of a whole indexed transaction.
///
/// `ty` is the `detect_transaction_type` result encoded as a u8 (see `ty_to_u8`),
/// computed in Pass 1 while the inputs are still alive (it depends on the inputs'
/// null/zerocoin prevout). `value_balance` is the SIGNED Sapling net value carried
/// forward for the fee calculation. `outs` holds EVERY output of the tx (one
/// PackedOut per `CTxOut`, including the kind=0 coinstake vout[0] marker), so
/// `outs.len() == tx.outputs.len()` and `out_sum` is unchanged.
#[derive(Clone)]
struct PackedTx {
    height: i32,
    ty: u8,
    value_balance: i64,
    outs: Box<[PackedOut]>,
}

/// Sentinel "no address" interned id (PackedOut.addr_a / addr_b when absent).
const NO_ADDR: u32 = u32::MAX;

/// Pack a single output: run the SAME `classify_output` logic Pass 2 used to run,
/// intern the resulting address string(s) to u32 ids, and copy value/vout verbatim.
/// This is the one site that maps a classification to a (kind, addr_a, addr_b):
///   Single  (P2PKH/P2SH/P2PK/unknown-single/EXM) => kind=1, addr_a=id, addr_b=NO_ADDR
///   ColdStake (P2CS)                              => kind=2, addr_a=staker, addr_b=owner
///   None (OP_RETURN/coinbase/coinstake/nonstandard/zerocoin) => kind=0, both NO_ADDR
/// `value` and `vout` are copied EXACTLY from the source output (INVARIANT #1).
fn pack_output(
    output: &CTxOut,
    addr_intern: &mut HashMap<String, u32>,
    addr_rev: &mut Vec<String>,
) -> PackedOut {
    let (kind, addr_a, addr_b) = match classify_output(output) {
        ScriptClassification::P2PKH(addr)
        | ScriptClassification::P2SH(addr)
        | ScriptClassification::P2PK(addr) => (1u8, intern(addr_intern, addr_rev, &addr), NO_ADDR),
        ScriptClassification::ColdStake { staker, owner } => {
            let staker_id = intern(addr_intern, addr_rev, &staker);
            let owner_id = intern(addr_intern, addr_rev, &owner);
            (2u8, staker_id, owner_id)
        }
        ScriptClassification::OpReturn
        | ScriptClassification::Coinbase
        | ScriptClassification::Coinstake
        | ScriptClassification::Nonstandard => (0u8, NO_ADDR, NO_ADDR),
    };
    PackedOut {
        value: output.value, // INVARIANT #1: exact, verbatim i64
        addr_a,
        addr_b,
        vout: output.index as u32, // parser sets index == position
        kind,
    }
}

/// Insert-or-get an address string in the intern table. Returns the existing dense
/// id if present, else assigns the next id. Free fn (not a closure) so it can borrow
/// both maps mutably at each call site. Used by `pack_output` and the enrichment pass.
fn intern(addr_intern: &mut HashMap<String, u32>, addr_rev: &mut Vec<String>, s: &str) -> u32 {
    if let Some(&id) = addr_intern.get(s) {
        return id;
    }
    let id = addr_rev.len() as u32;
    addr_intern.insert(s.to_string(), id);
    addr_rev.push(s.to_string());
    id
}

/// Per-shard (or whole-DB serial) Pass-1 accumulator. Shards build these over
/// disjoint txid ranges (`tx_shard_bounds`) and the merge concatenates them in
/// shard order (`merge_pass1_shards`). Address interning is LOCAL to each shard —
/// dense ids are purely in-memory and resolved to base58 strings only at write
/// time, so shards never share mutable state and the merge remaps each shard's
/// ids into one global table without changing any on-disk byte.
#[derive(Default)]
struct Pass1Shard {
    spent_outputs: HashSet<([u8; 32], u32)>,
    packed: Vec<PackedTx>,
    tx_rev: Vec<[u8; 32]>,
    addr_intern: HashMap<String, u32>,
    addr_rev: Vec<String>,
    // Metrics (summed across shards at merge; logging only, never on disk).
    tx_total: u64,
    tx_deserialized: u64,
    tx_failed: u64,
    inputs_processed: u64,
    sapling_count: u64,
}

/// Run the Pass-1 body for ONE already-deserialized, should-index transaction:
/// pack its outputs (interning into the shard's LOCAL table), append the
/// PackedTx + its DISPLAY-order txid, and fold its non-coinbase inputs into the
/// spent-outpoint set. This is the single source of Pass-1 per-tx logic shared
/// by the serial scan and every parallel shard, so the two cannot diverge.
///
/// `tx_index` is intentionally NOT built here: it is rebuilt once from `tx_rev`
/// after Pass 1 (identical content — last-writer-wins per txid), which frees
/// shards from sharing a global slot counter.
fn pass1_index_tx(sh: &mut Pass1Shard, key: &[u8], tx: &CTransaction, height: i32) {
    // INVARIANT #2: detect the type while inputs[0]'s null/zerocoin prevout is
    // still alive. INVARIANT #4: carry the SIGNED Sapling net value forward.
    let ty = crate::tx_type::ty_to_u8(crate::tx_type::detect_transaction_type(tx));
    let value_balance = tx
        .sapling_data
        .as_ref()
        .map(|s| s.value_balance)
        .unwrap_or(0);

    // PUSH EVERY output (incl. kind=0 markers / zerocoin / nonstandard) so
    // outs.len() == tx.outputs.len() and out_sum is identical (INVARIANT #1, #8).
    let mut outs: Vec<PackedOut> = Vec::with_capacity(tx.outputs.len());
    for output in &tx.outputs {
        outs.push(pack_output(output, &mut sh.addr_intern, &mut sh.addr_rev));
    }

    // Append under the DISPLAY-order txid (INVARIANT #7); the slot is the
    // position in `packed`/`tx_rev`, assigned globally by concat order at merge.
    let txid_bytes = txid_from_key(key);
    if !txid_bytes.is_empty() {
        if let Ok(txid_arr) = <[u8; 32]>::try_from(txid_bytes.as_slice()) {
            sh.tx_rev.push(txid_arr);
            sh.packed.push(PackedTx {
                height,
                ty,
                value_balance,
                outs: outs.into_boxed_slice(),
            });
        }
    }

    // Scan inputs into the spent-outpoint set (then the caller drops the tx).
    for input in &tx.inputs {
        if input.coinbase.is_some() {
            continue;
        }
        sh.inputs_processed += 1;
        if let Some(prevout) = &input.prevout {
            // prevout.hash from parser.rs is DISPLAY (reversed) format — the same
            // byte order as DB keys / tx_index / tx_rev (INVARIANT #7).
            if let Ok(prev_txid_display) = txid_from_hex(&prevout.hash) {
                let t: [u8; 32] = prev_txid_display
                    .as_slice()
                    .try_into()
                    .expect("spent-set prevout txid must be 32 bytes");
                sh.spent_outputs.insert((t, prevout.n));
            }
        }
    }
}

/// Split the 't'-prefixed (0x74) transaction keyspace into `n` contiguous,
/// disjoint `[lower, upper)` byte-bound pairs whose union is EXACTLY the set of
/// keys the serial Pass-1 iterator visits — every `'t'`+txid key, ascending. The
/// `'B'`-prefixed block-tx-index keys sort below 0x74 and are excluded by the
/// lower bound; the final shard's exclusive upper `0x75` stops past the entire
/// `'t'` prefix.
///
/// Sharding on the FIRST txid byte keeps shards ~even (txids are uniform hashes)
/// and — because shard k's range sorts entirely before shard k+1's —
/// concatenating shard results in index order reproduces serial SLOT ORDER
/// byte-for-byte, which the unsorted `'a'` UTXO-list ordering depends on.
///
/// `n` is clamped to `[1, 4]`: the cap bounds concurrent enrichment RAM. Each
/// shard holds ~1/n of the packed store, but transient merge overhead and
/// per-thread buffers grow with n, and the full-sync RSS already sits at the
/// 8 GiB target.
fn tx_shard_bounds(n: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    let n = n.clamp(1, 4);
    (0..n)
        .map(|k| {
            let lo = vec![b't', (k * 256 / n) as u8];
            let hi = if k + 1 == n {
                vec![b't' + 1] // 0x75: exclusive upper past the whole 't' prefix
            } else {
                vec![b't', ((k + 1) * 256 / n) as u8]
            };
            (lo, hi)
        })
        .collect()
}

/// Per-shard (or whole-DB serial) Pass-2b accumulator. The two byte-exact
/// outputs are `totals_sent` (i64 — plain `+=` is associative, so the merge is
/// order-independent across any shard count) and `txs_map_adds` (the spend-side
/// txids, which are sorted+deduped at write time, so push order is irrelevant).
/// `day_joins` / `coinstake_treasury` are analytics only (persisted to
/// chain_state, excluded from the address-index byte-diff gate), so their f64 /
/// Vec accumulation tolerates shard-order reordering.
#[derive(Default)]
struct Pass2bShard {
    /// id -> total spent (the ONLY writer of totals_sent; Pass 2 fills received).
    totals_sent: HashMap<u32, i64>,
    /// id -> spend-side txids to APPEND to the global txs_map (Pass 2 fills the
    /// received side); deduped+sorted at write, so order does not matter.
    txs_map_adds: HashMap<u32, Vec<Vec<u8>>>,
    day_joins: HashMap<String, DayJoinAgg>,
    coinstake_treasury: Vec<TreasuryPayout>,
    // Metrics (logging only).
    tx_total: u64,
    tx_deserialized: u64,
    tx_failed: u64,
    coinstake_skipped: u64,
    inputs_processed: u64,
    prevout_resolved: u64,
    input_processed: u64,
}

/// Run the Pass-2b body for ONE already-deserialized, slot-resolved transaction:
/// Already-resolved per-tx inputs to the shared join arithmetic. The PREVOUT
/// RESOLUTION (value/kind/height per input) is done by the CALLER — the full
/// enrich via the frozen `packed`/`tx_index`, the live updater via the
/// `transactions` CF — so this struct is resolver-source-agnostic.
#[derive(Debug, Clone)]
pub(crate) struct TxJoinInputs {
    pub height: i32,
    pub ty: crate::tx_type::TransactionType,
    /// Σ output values of THIS tx.
    pub out_sum: i64,
    /// Σ this tx's P2CS (kind==2) output values.
    pub p2cs_created: i64,
    /// Signed Sapling value balance of this tx.
    pub value_balance: i64,
    /// Serialized tx byte length (for `normal_tx_bytes`).
    pub value_len: usize,
    pub n_outputs: u32,
    /// Σ resolved input prevout values.
    pub input_sum: i64,
    /// Non-coinbase inputs carrying a prevout.
    pub inputs_with_prevout: u64,
    /// Of those, how many resolved (clamp basis; an unresolved/zPoS input is NOT
    /// counted so the clamp suppresses fee/minted).
    pub inputs_resolved: u64,
    /// Σ coin-days destroyed by this tx's resolved inputs.
    pub coin_days: f64,
    /// Σ resolved-input prevout values whose kind==2 (cold-stake spent).
    pub p2cs_spent: i64,
}

/// Per-tx join contributions to a day aggregate. The UNCONDITIONAL group
/// (coin_days/p2cs/normal_tx_bytes) is always emitted; fee/minted are clamp-gated
/// (zero when not all inputs resolved). `treasury` carries a budget payout if any.
#[derive(Clone, Default)]
pub(crate) struct TxJoinContribution {
    pub p2cs_created: i64,
    pub p2cs_spent: i64,
    pub coin_days: f64,
    pub normal_tx_bytes: u64,
    pub fees_total: i64,
    pub rewards_total: i64,
    pub staker_rewards_total: i64,
    pub treasury: Option<TreasuryPayout>,
}

/// THE shared per-tx join arithmetic — the single source of truth for the
/// daily-analytics join fields, called identically by the full enrich
/// (`pass2b_process_tx`) and the live updater. Byte-for-byte mirrors the original
/// inline pass2b logic: unconditional coin_days/p2cs/normal_tx_bytes, clamp-gated
/// fee (Normal) and minted/era split (Coinstake) with both clamps verbatim, the
/// >100 PIV budget threshold, and the era reward functions.
pub(crate) fn compute_tx_join(inp: &TxJoinInputs, date: &str) -> TxJoinContribution {
    let mut c = TxJoinContribution {
        p2cs_created: inp.p2cs_created,
        p2cs_spent: inp.p2cs_spent,
        coin_days: inp.coin_days,
        ..Default::default()
    };
    match inp.ty {
        crate::tx_type::TransactionType::Normal => {
            c.normal_tx_bytes = (inp.value_len as u64).saturating_sub(8);
            // Fee = transparent_in + valueBalance - transparent_out. Credit only
            // when every transparent input resolved, clamped to a sane ceiling.
            if inp.inputs_with_prevout > 0 && inp.inputs_resolved == inp.inputs_with_prevout {
                let fee = inp.input_sum + inp.value_balance - inp.out_sum;
                if fee > 0 && fee < 1_000 * crate::emission::COIN {
                    c.fees_total = fee;
                }
            }
        }
        crate::tx_type::TransactionType::Coinstake => {
            if inp.inputs_resolved == inp.inputs_with_prevout {
                let minted = inp.out_sum.saturating_sub(inp.input_sum);
                if minted > 0 {
                    let expected = crate::emission::era_block_reward(inp.height);
                    let excess = minted.saturating_sub(expected);
                    if excess > 100 * crate::emission::COIN {
                        c.treasury = Some(TreasuryPayout {
                            height: inp.height,
                            date: date.to_string(),
                            total_paid_sats: excess,
                            n_outputs: inp.n_outputs,
                        });
                        c.rewards_total = expected;
                    } else {
                        c.rewards_total = minted;
                    }
                    c.staker_rewards_total =
                        crate::emission::era_staker_reward(inp.height).min(minted);
                }
            }
        }
        crate::tx_type::TransactionType::Coinbase => {}
    }
    c
}

/// THE shared per-tx INLINE accumulation (counts / volumes / type-specific),
/// called identically by the full enrich (`persist_tx_daily_series`) and the live
/// updater (Lane I) so neither can drift on the subtle bits — the `coldstake`
/// script test and the >10 PIV PoW-coinbase budget threshold. Updates `agg` in
/// place and RETURNS a PoW-era coinbase budget `TreasuryPayout` if this coinbase
/// minted more than 10 PIV over the era reward. Does NOT touch the set fields
/// (stakers / active / first-seen), which are Lane R (window recompute).
pub(crate) fn accumulate_tx_inline(
    agg: &mut TxDayAgg,
    tx: &CTransaction,
    tx_type: crate::tx_type::TransactionType,
    raw_len: usize,
    height: i32,
    date: &str,
) -> Option<TreasuryPayout> {
    agg.tx_count += 1;
    agg.tx_bytes += raw_len as u64;
    if tx.sapling_data.is_some() {
        agg.sapling_txs += 1;
    }
    let mut treasury = None;
    match tx_type {
        crate::tx_type::TransactionType::Coinbase => {
            agg.coinbase += 1;
            // PoW-era budget payouts ride in the coinbase as value minted in
            // excess of the era block reward; >=10 PIV excludes tx-fee noise.
            if height > 0 {
                let total: i64 = tx.outputs.iter().map(|o| o.value).sum();
                let excess = total.saturating_sub(crate::emission::era_block_reward(height));
                if excess > 10 * crate::emission::COIN {
                    treasury = Some(TreasuryPayout {
                        height,
                        date: date.to_string(),
                        total_paid_sats: excess,
                        n_outputs: tx.outputs.len() as u32,
                    });
                }
            }
        }
        crate::tx_type::TransactionType::Coinstake => {
            agg.coinstake += 1;
            // A delegated (P2CS) stake re-mints its principal to the same P2CS
            // script, so any coldstake output marks this as cold staking.
            if tx
                .outputs
                .iter()
                .any(|o| crate::parser::get_script_type(&o.script_pubkey.script) == "coldstake")
            {
                agg.coldstake_txs += 1;
            }
            agg.stake_volume = agg
                .stake_volume
                .saturating_add(tx.outputs.iter().map(|o| o.value).sum());
        }
        crate::tx_type::TransactionType::Normal => {
            agg.payment += 1;
            agg.volume = agg
                .volume
                .saturating_add(tx.outputs.iter().map(|o| o.value).sum());
        }
    }
    treasury
}

/// join each input to its prevout through the FROZEN `tx_index`/`packed`
/// (read-only, so shards share them by Arc), debit the spending address(es)
/// in `sh.totals_sent` + append the txid to `sh.txs_map_adds`, and bucket the
/// Tier-2 prevout-join aggregates (fees, rewards, coin-days, cold-staking flows,
/// budget payouts) by the SPENDER's block date via the shared `compute_tx_join`.
/// Single source of Pass-2b per-tx logic shared by the serial scan + every shard.
#[allow(clippy::too_many_arguments)]
fn pass2b_process_tx(
    sh: &mut Pass2bShard,
    packed: &[PackedTx],
    tx_index: &HashMap<[u8; 32], u32>,
    block_times: &[u32],
    current_txid_bytes: &[u8],
    cur_slot: u32,
    tx: &CTransaction,
    height: i32,
    value_len: usize,
) {
    // Coinstake inputs ARE counted as "sent" (UTXO accounting, Blockbook parity):
    // the staked output is consumed and its principal re-minted in the coinstake
    // outputs (counted as received). (ty was computed in Pass 1; ty==1 => Coinstake.)
    if packed[cur_slot as usize].ty == 1 {
        sh.coinstake_skipped += 1; // metric retained: counts coinstakes seen
    }

    // Tier-2 accumulators for this spending transaction (prevout joins).
    let mut input_sum: i64 = 0;
    let mut inputs_with_prevout: u64 = 0;
    let mut inputs_resolved: u64 = 0;
    let mut tx_coin_days: f64 = 0.0;
    let mut tx_p2cs_spent: i64 = 0;

    // For every input, find the prevout's addresses and attribute this tx to them.
    for input in &tx.inputs {
        if input.coinbase.is_some() {
            continue;
        }
        sh.inputs_processed += 1;
        if let Some(prevout) = &input.prevout {
            inputs_with_prevout += 1;
            // prevout.hash from parser.rs is in DISPLAY (reversed) format — the
            // same byte order as tx_index keys / tx_rev (INVARIANT #7).
            if let Ok(prev_txid_display) = txid_from_hex(&prevout.hash) {
                // INVARIANT #3 (zPoS null-prevout): resolve ONLY through tx_index.
                // A zPoS coinstake's first input carries an all-zero/zerocoin prevout
                // txid that was never inserted into tx_index, so the lookup MISSES,
                // inputs_resolved is NOT incremented (while inputs_with_prevout IS),
                // and the clamp `inputs_with_prevout>0 && inputs_resolved==inputs_with_prevout`
                // correctly FAILS -> fee/minted suppressed for zPoS.
                let prev = <[u8; 32]>::try_from(prev_txid_display.as_slice())
                    .ok()
                    .and_then(|a| tx_index.get(&a).copied())
                    .map(|s| {
                        let p = &packed[s as usize];
                        (p, p.height)
                    });

                if let Some((prev_tx, prev_height)) = prev {
                    if let Some(prev_out) = prev_tx.outs.get(prevout.n as usize) {
                        // Tier 2: spend-side joins — input value sum (fees / rewards),
                        // coin age, and cold-staking principal spent.
                        inputs_resolved += 1;
                        sh.prevout_resolved += 1;
                        input_sum = input_sum.saturating_add(prev_out.value);
                        if prev_height >= 0 && height >= prev_height {
                            let age_days = (height - prev_height) as f64 / 1440.0;
                            tx_coin_days += (prev_out.value as f64 / 100_000_000.0) * age_days;
                        }
                        // coldstake-spent test <= (prev_out.kind == 2), which coincides
                        // exactly with the old get_script_type=="coldstake".
                        if prev_out.kind == 2 {
                            tx_p2cs_spent = tx_p2cs_spent.saturating_add(prev_out.value);
                        }

                        // Attribution from the packed kind / interned ids.
                        match prev_out.kind {
                            1 => {
                                // Standard: address is spending.
                                let id = prev_out.addr_a;
                                *sh.totals_sent.entry(id).or_insert(0) += prev_out.value;
                                sh.txs_map_adds
                                    .entry(id)
                                    .or_default()
                                    .push(current_txid_bytes.to_vec());
                            }
                            2 => {
                                // Cold stake spend: debit BOTH addresses, mirroring the
                                // credit-both in Pass 2. Staker and owner are DISTINCT ids.
                                let staker_id = prev_out.addr_a;
                                let owner_id = prev_out.addr_b;
                                *sh.totals_sent.entry(staker_id).or_insert(0) += prev_out.value;
                                *sh.totals_sent.entry(owner_id).or_insert(0) += prev_out.value;
                                sh.txs_map_adds
                                    .entry(staker_id)
                                    .or_default()
                                    .push(current_txid_bytes.to_vec());
                                sh.txs_map_adds
                                    .entry(owner_id)
                                    .or_default()
                                    .push(current_txid_bytes.to_vec());
                            }
                            _ => {
                                // No attribution for nonstandard/OP_RETURN/etc.
                            }
                        }
                    }
                }
            }
        }
    }

    // Tier 2: bucket this tx's join aggregates by the SPENDER's block date,
    // through the SHARED `compute_tx_join` arithmetic (same code the live updater
    // calls; the only difference is HOW the inputs above were resolved).
    if height >= 0 && (height as usize) < block_times.len() && block_times[height as usize] != 0 {
        let date = unix_to_date(block_times[height as usize] as u64);
        let cur = &packed[cur_slot as usize];
        let out_sum: i64 = cur.outs.iter().map(|o| o.value).sum();
        let p2cs_created: i64 = cur
            .outs
            .iter()
            .filter(|o| o.kind == 2)
            .map(|o| o.value)
            .sum();
        let contrib = compute_tx_join(
            &TxJoinInputs {
                height,
                ty: crate::tx_type::u8_to_ty(cur.ty),
                out_sum,
                p2cs_created,
                value_balance: cur.value_balance,
                value_len,
                n_outputs: cur.outs.len() as u32,
                input_sum,
                inputs_with_prevout,
                inputs_resolved,
                coin_days: tx_coin_days,
                p2cs_spent: tx_p2cs_spent,
            },
            &date,
        );
        let agg = sh.day_joins.entry(date.clone()).or_default();
        agg.p2cs_created = agg.p2cs_created.saturating_add(contrib.p2cs_created);
        agg.p2cs_spent = agg.p2cs_spent.saturating_add(contrib.p2cs_spent);
        agg.coin_days_destroyed += contrib.coin_days;
        agg.normal_tx_bytes += contrib.normal_tx_bytes;
        agg.fees_total = agg.fees_total.saturating_add(contrib.fees_total);
        agg.rewards_total = agg.rewards_total.saturating_add(contrib.rewards_total);
        agg.staker_rewards_total = agg
            .staker_rewards_total
            .saturating_add(contrib.staker_rewards_total);
        if let Some(t) = contrib.treasury {
            sh.coinstake_treasury.push(t);
        }
    }
}

/// Number of enrichment range-shards from `sync.enrich_parallel_shards`.
/// Default 1 => the serial path (today's behavior, untouched). Clamped to
/// `[1, 4]` (the RAM cap — see `tx_shard_bounds`).
fn effective_enrich_shards() -> usize {
    match crate::config::get_global_config().get_int("sync.enrich_parallel_shards") {
        Ok(n) if n >= 1 => (n as usize).clamp(1, 4),
        _ => 1,
    }
}

/// PASS 1, parallel: one std::thread per `tx_shard_bounds` range, each scanning a
/// DISJOINT, contiguous slice of the 't' keyspace with the blocking deserializer
/// (byte-identical to the async one) into its own `Pass1Shard`. Threads share
/// nothing mutable, so this is embarrassingly parallel; `merge_pass1_shards`
/// reassembles them in shard order. A panicked shard aborts enrichment rather
/// than silently dropping its transactions (fix: propagate, never swallow).
fn run_pass1_parallel(db: Arc<DB>, n: usize) -> Result<Pass1Shard, String> {
    let bounds = tx_shard_bounds(n);
    let mut handles = Vec::with_capacity(bounds.len());
    for (lo, hi) in bounds {
        let db = Arc::clone(&db);
        handles.push(std::thread::spawn(move || -> Pass1Shard {
            let cf = db
                .cf_handle("transactions")
                .expect("transactions CF not found");
            let mut ro = rocksdb::ReadOptions::default();
            ro.set_iterate_lower_bound(lo);
            ro.set_iterate_upper_bound(hi);
            let mut sh = Pass1Shard::default();
            let iter = db.iterator_cf_opt(&cf, ro, rocksdb::IteratorMode::Start);
            for item in iter {
                let (key, value) = match item {
                    Ok(kv) => kv,
                    Err(_) => continue,
                };
                // Defensive guards (the bounds already exclude 'B' < 0x74).
                if key.first() == Some(&b'B') {
                    continue;
                }
                if value.len() < 8 {
                    continue;
                }
                let height = i32::from_le_bytes(value[4..8].try_into().unwrap_or([0, 0, 0, 0]));
                if !should_index_transaction(height) {
                    continue;
                }
                let raw_tx = &value[8..];
                let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
                tx_with_header.extend_from_slice(&[0u8; 4]);
                tx_with_header.extend_from_slice(raw_tx);
                sh.tx_total += 1;
                let tx = match deserialize_transaction_blocking(&tx_with_header) {
                    Ok(tx) => {
                        sh.tx_deserialized += 1;
                        if tx.sapling_data.is_some() {
                            sh.sapling_count += 1;
                        }
                        tx
                    }
                    Err(_) => {
                        sh.tx_failed += 1;
                        continue;
                    }
                };
                pass1_index_tx(&mut sh, &key, &tx, height);
            }
            sh
        }));
    }
    // Join IN SHARD ORDER so the merge concatenates slots ascending; propagate panics.
    let mut shards = Vec::with_capacity(handles.len());
    for h in handles {
        match h.join() {
            Ok(sh) => shards.push(sh),
            Err(_) => return Err("Pass 1 shard thread panicked".to_string()),
        }
    }
    Ok(merge_pass1_shards(shards))
}

/// Reassemble per-shard Pass-1 results into one global `Pass1Shard`, byte-exact
/// with the serial scan:
///  - `packed`/`tx_rev` concatenated in SHARD ORDER (== ascending txid == serial
///    slot order); each shard's global base is the running length (realized,
///    post-filter counts — empty shards contribute nothing).
///  - each shard's LOCAL interned ids remapped to GLOBAL ids (NO_ADDR preserved,
///    BOTH addr_a and addr_b remapped for P2CS). Global id assignment differs from
///    serial, but ids are in-memory only and resolve to the SAME base58 strings,
///    so every on-disk byte is identical.
///  - `spent_outputs` unioned (a set — order-independent).
fn merge_pass1_shards(shards: Vec<Pass1Shard>) -> Pass1Shard {
    let total_packed: usize = shards.iter().map(|s| s.packed.len()).sum();
    let mut out = Pass1Shard {
        packed: Vec::with_capacity(total_packed),
        tx_rev: Vec::with_capacity(total_packed),
        ..Default::default()
    };
    for mut sh in shards {
        // Build this shard's LOCAL id -> GLOBAL id remap by re-interning its
        // strings (in local-id order) into the global table.
        let mut remap = vec![0u32; sh.addr_rev.len()];
        for (local_id, s) in sh.addr_rev.iter().enumerate() {
            remap[local_id] = intern(&mut out.addr_intern, &mut out.addr_rev, s);
        }
        // Remap every packed out's addr_a/addr_b (skip NO_ADDR; P2CS uses addr_b)
        // and append in shard order — the global slot base is out.packed.len().
        for mut ptx in sh.packed.drain(..) {
            for o in ptx.outs.iter_mut() {
                if o.addr_a != NO_ADDR {
                    o.addr_a = remap[o.addr_a as usize];
                }
                if o.addr_b != NO_ADDR {
                    o.addr_b = remap[o.addr_b as usize];
                }
            }
            out.packed.push(ptx);
        }
        out.tx_rev.append(&mut sh.tx_rev);
        // Union the spent-outpoint set (order-independent).
        if out.spent_outputs.is_empty() {
            out.spent_outputs = std::mem::take(&mut sh.spent_outputs);
        } else {
            for e in sh.spent_outputs.drain() {
                out.spent_outputs.insert(e);
            }
        }
        out.tx_total += sh.tx_total;
        out.tx_deserialized += sh.tx_deserialized;
        out.tx_failed += sh.tx_failed;
        out.inputs_processed += sh.inputs_processed;
        out.sapling_count += sh.sapling_count;
    }
    out
}

/// PASS 2b, parallel: one std::thread per shard, each scanning its 't'-keyspace
/// slice and joining prevouts through the FROZEN (Arc, read-only) `packed` /
/// `tx_index` / `block_times`, accumulating into its own `Pass2bShard`. The
/// frozen globals are shared by Arc (no duplication of the ~4 GB packed store).
/// Panics propagate. `merge_pass2b_shards` combines the results.
fn run_pass2b_parallel(
    db: Arc<DB>,
    n: usize,
    packed: Arc<Vec<PackedTx>>,
    tx_index: Arc<HashMap<[u8; 32], u32>>,
    block_times: Arc<Vec<u32>>,
) -> Result<Pass2bShard, String> {
    let bounds = tx_shard_bounds(n);
    let mut handles = Vec::with_capacity(bounds.len());
    for (lo, hi) in bounds {
        let db = Arc::clone(&db);
        let packed = Arc::clone(&packed);
        let tx_index = Arc::clone(&tx_index);
        let block_times = Arc::clone(&block_times);
        handles.push(std::thread::spawn(move || -> Pass2bShard {
            let cf = db
                .cf_handle("transactions")
                .expect("transactions CF not found");
            let mut ro = rocksdb::ReadOptions::default();
            ro.set_iterate_lower_bound(lo);
            ro.set_iterate_upper_bound(hi);
            let mut sh = Pass2bShard::default();
            let iter = db.iterator_cf_opt(&cf, ro, rocksdb::IteratorMode::Start);
            for item in iter {
                let (key, value) = match item {
                    Ok(kv) => kv,
                    Err(_) => continue,
                };
                if key.first() == Some(&b'B') {
                    continue;
                }
                if value.len() < 8 {
                    continue;
                }
                let height = i32::from_le_bytes(value[4..8].try_into().unwrap_or([0, 0, 0, 0]));
                if !should_index_transaction(height) {
                    continue;
                }
                let current_txid_bytes = txid_from_key(&key);
                if current_txid_bytes.is_empty() {
                    continue;
                }
                let cur_slot = match <[u8; 32]>::try_from(current_txid_bytes.as_slice())
                    .ok()
                    .and_then(|a| tx_index.get(&a).copied())
                {
                    Some(s) => s,
                    None => continue,
                };
                sh.tx_total += 1;
                let raw_tx = &value[8..];
                let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
                tx_with_header.extend_from_slice(&[0u8; 4]);
                tx_with_header.extend_from_slice(raw_tx);
                let tx = match deserialize_transaction_blocking(&tx_with_header) {
                    Ok(tx) => {
                        sh.tx_deserialized += 1;
                        tx
                    }
                    Err(_) => {
                        sh.tx_failed += 1;
                        continue;
                    }
                };
                pass2b_process_tx(
                    &mut sh,
                    &packed,
                    &tx_index,
                    &block_times,
                    &current_txid_bytes,
                    cur_slot,
                    &tx,
                    height,
                    value.len(),
                );
                sh.input_processed += 1;
            }
            sh
        }));
    }
    let mut shards = Vec::with_capacity(handles.len());
    for h in handles {
        match h.join() {
            Ok(sh) => shards.push(sh),
            Err(_) => return Err("Pass 2b shard thread panicked".to_string()),
        }
    }
    Ok(merge_pass2b_shards(shards))
}

/// Combine per-shard Pass-2b results. The two byte-exact outputs merge
/// order-independently: `totals_sent` by plain `+=` (i64 — associative), and the
/// `txs_map_adds` by append (sorted+deduped at write). `day_joins` /
/// `coinstake_treasury` are analytics (chain_state, excluded from the
/// address-index byte-diff gate), so their f64 / Vec accumulation tolerates the
/// shard-order reordering.
fn merge_pass2b_shards(shards: Vec<Pass2bShard>) -> Pass2bShard {
    let mut out = Pass2bShard::default();
    for mut sh in shards {
        for (id, v) in sh.totals_sent.drain() {
            *out.totals_sent.entry(id).or_insert(0) += v;
        }
        for (id, mut adds) in sh.txs_map_adds.drain() {
            out.txs_map_adds.entry(id).or_default().append(&mut adds);
        }
        for (date, j) in sh.day_joins.drain() {
            let agg = out.day_joins.entry(date).or_default();
            agg.fees_total = agg.fees_total.saturating_add(j.fees_total);
            agg.normal_tx_bytes += j.normal_tx_bytes;
            agg.rewards_total = agg.rewards_total.saturating_add(j.rewards_total);
            agg.staker_rewards_total = agg
                .staker_rewards_total
                .saturating_add(j.staker_rewards_total);
            agg.coin_days_destroyed += j.coin_days_destroyed;
            agg.p2cs_created = agg.p2cs_created.saturating_add(j.p2cs_created);
            agg.p2cs_spent = agg.p2cs_spent.saturating_add(j.p2cs_spent);
        }
        out.coinstake_treasury.append(&mut sh.coinstake_treasury);
        out.tx_total += sh.tx_total;
        out.tx_deserialized += sh.tx_deserialized;
        out.tx_failed += sh.tx_failed;
        out.coinstake_skipped += sh.coinstake_skipped;
        out.inputs_processed += sh.inputs_processed;
        out.prevout_resolved += sh.prevout_resolved;
        out.input_processed += sh.input_processed;
    }
    out
}

/// Build address index from all transactions
/// This creates the addr_index CF entries for address lookups
pub async fn enrich_all_addresses(db: Arc<DB>) -> Result<(), Box<dyn std::error::Error>> {
    let _span = info_span!("enrich_all_addresses").entered();
    info!("Building address index from transactions");

    let cf_transactions = db
        .cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    let cf_addr_index = db
        .cf_handle("addr_index")
        .ok_or("addr_index CF not found")?;
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;

    // Build the height -> block-time index ONCE, up front: Pass 2b buckets its
    // prevout-join aggregates (fees, rewards, coin-days-destroyed, cold-staking
    // flows) by the SPENDER's block date, and persist_tx_daily_series reuses
    // the same index afterwards instead of rebuilding it.
    let tip = db
        .get_cf(&cf_state, b"sync_height")?
        .filter(|b| b.len() >= 4)
        .map(|b| i32::from_le_bytes(b[0..4].try_into().unwrap_or([0; 4])))
        .unwrap_or(0);
    let (block_times, block_bits) = if tip > 0 {
        info!(tip = tip, "Building block-time index for analytics");
        build_block_times(&db, tip)?
    } else {
        (Vec::new(), Vec::new())
    };

    // Enrichment parallelism: 1 => serial (default, today's path untouched),
    // 2..=4 => range-sharded Pass 1 / Pass 2b. The dump-addr-snapshot byte-diff
    // gate must pass at every shard count before the default is raised.
    let shards = effective_enrich_shards();
    if shards >= 2 {
        info!(
            shards = shards,
            "Enrichment: parallel range-sharded Pass 1 / Pass 2b enabled"
        );
    }

    let mut processed = 0;
    let mut indexed_outputs = 0;
    let batch_size = 10000;

    info!("Pass 1: Building complete spent outputs set");

    // PASS 1: Build the packed tx store + the complete spent-outpoint set by
    // scanning every indexed transaction. The per-tx body lives in
    // `pass1_index_tx` so this serial scan and each parallel shard share ONE
    // implementation and cannot diverge. Interning is local to `sh`; `tx_index`
    // is rebuilt from `tx_rev` after the scan (identical content, no shared slot
    // counter), and the working set is destructured back out below.
    //   spent_outputs : ([u8;32] DISPLAY txid, u32 vout) inline keys, ~half RAM
    //   packed/tx_rev : slot -> PackedTx / DISPLAY-order txid (slot == position)
    //   addr_intern/addr_rev : base58 string <-> dense u32 id (in-memory only)
    let sh = if shards >= 2 {
        // Parallel range-sharded scan on a blocking-pool thread: the per-shard
        // threads each hold a RocksDB iterator across the blocking deserializer,
        // so they must not run on the async executor.
        let dbc = Arc::clone(&db);
        let merged = tokio::task::spawn_blocking(move || run_pass1_parallel(dbc, shards)).await??;
        processed = merged.tx_deserialized as i32; // transactions_scanned (log only)
        merged
    } else {
        let mut sh = Pass1Shard::default();
        let iter1 = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);

        for item in iter1 {
            let (key, value) = item?;
            // Skip block transaction index keys.
            if key.first() == Some(&b'B') {
                continue;
            }
            // Skip invalid transactions.
            if value.len() < 8 {
                continue;
            }
            // Check height: skip orphaned and unresolved transactions.
            let height_bytes: [u8; 4] = value[4..8].try_into().unwrap_or([0, 0, 0, 0]);
            let height = i32::from_le_bytes(height_bytes);
            if !should_index_transaction(height) {
                continue;
            }
            let raw_tx = &value[8..];
            let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
            tx_with_header.extend_from_slice(&[0u8; 4]);
            tx_with_header.extend_from_slice(raw_tx);

            sh.tx_total += 1;
            let tx = match deserialize_transaction(&tx_with_header).await {
                Ok(tx) => {
                    sh.tx_deserialized += 1;
                    // Count Sapling transactions (version >= 3 with Sapling data).
                    if tx.sapling_data.is_some() {
                        sh.sapling_count += 1;
                    }
                    tx
                }
                Err(e) => {
                    sh.tx_failed += 1;
                    // CRITICAL: Log deserialization failures.
                    let txid_bytes = txid_from_key(&key);
                    let txid_hex = hex::encode(&txid_bytes);
                    warn!(txid = %txid_hex, height = height, error = ?e, "Pass 1: Failed to deserialize transaction");
                    continue;
                }
            };

            pass1_index_tx(&mut sh, &key, &tx, height);
            processed += 1;
        }
        sh
    };

    // Destructure the shard back into the per-pass working set and rebuild
    // tx_index (DISPLAY txid -> slot) from tx_rev — same content the old inline
    // insert produced (last-writer-wins per txid).
    let Pass1Shard {
        spent_outputs,
        packed,
        tx_rev,
        addr_intern,
        addr_rev,
        tx_total: pass1_tx_total,
        tx_deserialized: pass1_tx_deserialized,
        tx_failed: pass1_tx_failed,
        inputs_processed: pass1_inputs_processed,
        sapling_count: pass1_sapling_count,
    } = sh;
    let mut tx_index: HashMap<[u8; 32], u32> = HashMap::with_capacity(tx_rev.len());
    for (slot, txid) in tx_rev.iter().enumerate() {
        tx_index.insert(*txid, slot as u32);
    }

    // Freeze the packed store + tx_index behind Arc so the parallel Pass 2b shards
    // share them read-only (no ~4 GB duplication). Pass 2, the serial Pass 2b and
    // the write loop all index through the Arc transparently (Deref coercion).
    let packed = Arc::new(packed);
    let tx_index = Arc::new(tx_index);

    info!(
        transactions_scanned = processed,
        spent_outputs_found = spent_outputs.len(),
        pass1_total = pass1_tx_total,
        pass1_deserialized = pass1_tx_deserialized,
        pass1_failed = pass1_tx_failed,
        pass1_inputs = pass1_inputs_processed,
        pass1_sapling = pass1_sapling_count,
        packed_entries = packed.len(),
        "Pass 1 complete: Spent outputs set built"
    );

    info!("Pass 2: Indexing outputs with spent flags");

    // Reset counter for pass 2
    processed = 0;

    // (addr_intern / addr_rev / intern are declared ABOVE Pass 1 now, since Pass 1
    // classifies + interns every output while building the packed store.)

    // PASS 2: Build address map with spent flags (outputs -> address_map).
    // Value is now (slot, vout) — purely in-memory ids resolved back to the
    // 32-byte DISPLAY txid (tx_rev[slot]) only at on-disk serialize time, so the
    // UTXO bytes stay byte-identical (INVARIANT #7).
    let mut address_map: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    // Also maintain a txs_map to collect all txids involving an address (received OR sent)
    let mut txs_map: HashMap<u32, Vec<Vec<u8>>> = HashMap::new();
    // NEW: Track total received and sent per address during Pass 2 (much faster!)
    let mut totals_received: HashMap<u32, i64> = HashMap::new();
    let mut totals_sent: HashMap<u32, i64> = HashMap::new();

    // Phase 2 Instrumentation: Track Pass 2 metrics. Pass 2 now reads the packed
    // store built in Pass 1 (no DB re-read, no re-deserialize, no re-classify):
    // every slot is an already-indexed, already-deserialized tx, so total ==
    // packed.len() and failures == 0 (kept for the cross-pass divergence checks).
    let mut pass2_tx_total = 0;
    let pass2_tx_failed = 0;

    for slot in 0..packed.len() {
        let ptx = &packed[slot];

        pass2_tx_total += 1;

        // Track which addresses are involved in this transaction (for txs_map),
        // keyed by interned id.
        let mut tx_addresses: HashSet<u32> = HashSet::new();

        // Detect if this is a coinstake transaction (ty was computed in Pass 1).
        let tx_is_coinstake = ptx.ty == 1;

        for out in ptx.outs.iter() {
            // PIVX Core Rule: Skip vout[0] in coinstake (OP_RETURN marker).
            if tx_is_coinstake && out.vout == 0 {
                continue;
            }

            // Attribution already resolved in Pass 1 (kind + interned ids).
            match out.kind {
                1 => {
                    // Standard single-address output (P2PKH / P2SH / P2PK / unknown).
                    let id = out.addr_a;
                    tx_addresses.insert(id);
                    *totals_received.entry(id).or_insert(0) += out.value;

                    // Index UTXO if non-zero value
                    if out.value > 0 {
                        address_map
                            .entry(id)
                            .or_default()
                            .push((slot as u32, out.vout));
                        indexed_outputs += 1;
                    }
                }

                2 => {
                    // Cold staking (P2CS): the output is indexed under BOTH the staker
                    // (S-address) and the owner (D-address), each credited with the full
                    // value — the same convention Blockbook uses for multi-address
                    // outputs, and what wallets/explorers expect when querying either
                    // side of a delegation. The spend side (Pass 2b) debits BOTH
                    // addresses symmetrically, so balance == received - sent holds for
                    // each address independently. Staker and owner are DISTINCT strings
                    // and so intern to DISTINCT ids.
                    let staker_id = out.addr_a;
                    let owner_id = out.addr_b;
                    *totals_received.entry(staker_id).or_insert(0) += out.value;
                    *totals_received.entry(owner_id).or_insert(0) += out.value;

                    // Both addresses appear in transaction list
                    tx_addresses.insert(staker_id);
                    tx_addresses.insert(owner_id);

                    // Both get UTXO entry for tracking
                    if out.value > 0 {
                        address_map
                            .entry(staker_id)
                            .or_default()
                            .push((slot as u32, out.vout));
                        address_map
                            .entry(owner_id)
                            .or_default()
                            .push((slot as u32, out.vout));
                        indexed_outputs += 2; // Count both
                    }
                }

                _ => {
                    // kind == 0: no address attribution (OP_RETURN / coinbase /
                    // coinstake marker / zerocoin mint / nonstandard).
                }
            }
        }

        // Add this transaction to txs_map for ALL addresses involved (by id).
        // KEEP txs_map values as the 32-byte DISPLAY txid (tx_rev[slot]) so the
        // unique_txids.sort() in the write loop still sorts txids, not slots.
        for address_id in tx_addresses {
            txs_map
                .entry(address_id)
                .or_default()
                .push(tx_rev[slot].to_vec());
        }

        processed += 1;
    }

    info!(
        pass2_total = pass2_tx_total,
        pass2_failed = pass2_tx_failed,
        "Pass 2 complete"
    );

    // CRITICAL: Detect asymmetric failures between passes
    if pass1_tx_total != pass2_tx_total {
        warn!(
            pass1_total = pass1_tx_total,
            pass2_total = pass2_tx_total,
            diff = (pass1_tx_total as i64 - pass2_tx_total as i64).abs(),
            "Pass divergence: Transaction count mismatch"
        );
    }
    if pass1_tx_failed != pass2_tx_failed {
        warn!(
            pass1_failed = pass1_tx_failed,
            pass2_failed = pass2_tx_failed,
            diff = (pass1_tx_failed as i64 - pass2_tx_failed as i64).abs(),
            "Asymmetric failures between passes - will cause balance errors"
        );
    }

    info!(
        unique_addresses = address_map.len(),
        spent_outputs = spent_outputs.len(),
        "Writing address index to database"
    );
    info!("Pass 2b: Scanning inputs to include sent transactions and calculate totals");

    // Pass 2b accumulators live in a shard so this serial scan and each parallel
    // shard share ONE per-tx implementation (pass2b_process_tx) and merge the
    // same way. The byte-exact outputs are totals_sent (i64, plain += is
    // associative) and the txs_map adds (sorted+deduped at write); day_joins /
    // coinstake_treasury are analytics (gate-excluded). Prevout joins resolve
    // through the frozen tx_index -> packed; the current tx is deserialized once
    // for its INPUTS while its outs / ty / value_balance come from packed[slot].
    let sh2 = if shards >= 2 {
        // Parallel Pass 2b on a blocking-pool thread. Shards read the frozen
        // packed / tx_index by Arc (no ~4 GB duplication); block_times (~22 MB) is
        // cloned once into an Arc so the original stays available for the daily
        // series persist below.
        let dbc = Arc::clone(&db);
        let packed = Arc::clone(&packed);
        let tx_index = Arc::clone(&tx_index);
        let block_times_arc = Arc::new(block_times.clone());
        tokio::task::spawn_blocking(move || {
            run_pass2b_parallel(dbc, shards, packed, tx_index, block_times_arc)
        })
        .await??
    } else {
        let mut sh2 = Pass2bShard::default();
        let iter3 = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
        for item in iter3 {
            let (key, value) = item?;
            if key.first() == Some(&b'B') {
                continue;
            }
            if value.len() < 8 {
                continue;
            }
            let height_bytes: [u8; 4] = value[4..8].try_into().unwrap_or([0, 0, 0, 0]);
            let height = i32::from_le_bytes(height_bytes);
            if !should_index_transaction(height) {
                continue;
            }

            // Extract current txid from key
            let current_txid_bytes = txid_from_key(&key);
            if current_txid_bytes.is_empty() {
                continue;
            }

            // Resolve this tx's slot in the packed store (built in Pass 1). Outputs,
            // ty and value_balance come from packed[slot]; the raw inputs come from
            // the deserialize below (inputs are not packed).
            let cur_slot = match <[u8; 32]>::try_from(current_txid_bytes.as_slice())
                .ok()
                .and_then(|a| tx_index.get(&a).copied())
            {
                Some(s) => s,
                None => continue, // not indexed in Pass 1 (failed deserialize) — skip
            };

            sh2.tx_total += 1;

            // Deserialize the current tx ONCE for its INPUTS (not held in packed).
            let raw_tx = &value[8..];
            let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
            tx_with_header.extend_from_slice(&[0u8; 4]);
            tx_with_header.extend_from_slice(raw_tx);
            let tx = match deserialize_transaction(&tx_with_header).await {
                Ok(tx) => {
                    sh2.tx_deserialized += 1;
                    tx
                }
                Err(e) => {
                    sh2.tx_failed += 1;
                    let txid_hex = hex::encode(&current_txid_bytes);
                    warn!(txid = %txid_hex, height = height, error = ?e, "Pass 2b: Failed to deserialize transaction");
                    continue;
                }
            };

            pass2b_process_tx(
                &mut sh2,
                &packed,
                &tx_index,
                &block_times,
                &current_txid_bytes,
                cur_slot,
                &tx,
                height,
                value.len(),
            );
            sh2.input_processed += 1;
        }
        sh2
    };

    // Merge the shard into the global per-address maps. totals_sent was empty
    // (Pass 2 fills only totals_received), so its entries move in directly; the
    // txs_map adds extend Pass 2's received-side lists (sorted+deduped at write,
    // so the order they arrive in does not matter).
    let Pass2bShard {
        totals_sent: sh2_totals_sent,
        txs_map_adds,
        day_joins,
        coinstake_treasury,
        tx_total: pass2b_tx_total,
        tx_deserialized: pass2b_tx_deserialized,
        tx_failed: pass2b_tx_failed,
        coinstake_skipped: pass2b_coinstake_skipped,
        inputs_processed: pass2b_inputs_processed,
        prevout_resolved: pass2b_prevout_resolved,
        input_processed,
    } = sh2;
    for (id, v) in sh2_totals_sent {
        *totals_sent.entry(id).or_insert(0) += v;
    }
    for (id, adds) in txs_map_adds {
        txs_map.entry(id).or_default().extend(adds);
    }

    info!(
        input_processed = input_processed,
        pass2b_total = pass2b_tx_total,
        pass2b_deserialized = pass2b_tx_deserialized,
        pass2b_failed = pass2b_tx_failed,
        pass2b_coinstake_skipped = pass2b_coinstake_skipped,
        pass2b_inputs = pass2b_inputs_processed,
        pass2b_prevout_resolved = pass2b_prevout_resolved,
        "Pass 2b complete"
    );

    // Interning round-trip invariant: every id resolves back to the exact string
    // that was interned, and the forward/reverse maps stay the same size.
    debug_assert_eq!(
        addr_intern.len(),
        addr_rev.len(),
        "intern/rev size mismatch"
    );
    if let Some((s, &id)) = addr_intern.iter().next() {
        debug_assert_eq!(&addr_rev[id as usize], s, "intern round-trip mismatch");
    }

    // CRITICAL: Final divergence check across all passes
    if pass1_tx_total != pass2_tx_total || pass2_tx_total != pass2b_tx_total {
        warn!(
            pass1_total = pass1_tx_total,
            pass2_total = pass2_tx_total,
            pass2b_total = pass2b_tx_total,
            "TX count mismatch across passes"
        );
    }

    if pass1_tx_failed > 0 || pass2_tx_failed > 0 || pass2b_tx_failed > 0 {
        if pass1_tx_failed != pass2_tx_failed || pass2_tx_failed != pass2b_tx_failed {
            warn!(
                pass1_failed = pass1_tx_failed,
                pass2_failed = pass2_tx_failed,
                pass2b_failed = pass2b_tx_failed,
                "Asymmetric deserialization failures - will cause balance errors"
            );
        } else {
            info!(
                failed = pass1_tx_failed,
                "Deserialization failures (consistent across passes)"
            );
        }
    }

    info!(
        unique_addresses = address_map.len(),
        "Writing address index to database"
    );

    // Write address mappings to database
    let mut batch = rocksdb::WriteBatch::default();
    let mut written = 0;
    let total_addresses = address_map.len(); // Cache length before consuming map
    let mut total_utxos_checked = 0;
    let mut total_spent_found = 0;

    // HODL / dormancy accumulators, fed by the SAME deduped unspent sets that
    // back the 'a' entries (the path verified against the reference explorer).
    // Restricting the snapshot to address-attributed UTXOs is what keeps the
    // tracked total at the transparent supply: outputs with no address are
    // dominated by OP_ZEROCOINMINT scripts, whose zerocoin spends consume the
    // accumulator by serial number — never the outpoint — so they would sit in
    // the spent-set-based walk as "unspent" forever (~744M phantom PIV).
    // hodl_seen dedupes outpoints across addresses because P2CS UTXOs are
    // indexed under BOTH the staker and the owner.
    let mut hodl_sums = [0i64; HODL_BANDS.len()];
    let mut hodl_total: i64 = 0;
    let mut hodl_seen: HashSet<([u8; 32], u32)> = HashSet::new();

    for (id, utxos) in address_map {
        // Resolve the interned id back to its exact base58 string ONLY here, where
        // the on-disk key bytes are built — reproduces the original byte-for-byte.
        let address = &addr_rev[id as usize];
        let mut key = vec![b'a'];
        key.extend_from_slice(address.as_bytes());

        // Build the v2 'a' record: txid(32)+vout(8)+value(8)+kind(1)=49B per unspent
        // entry. value/kind are immutable functions of the source tx (the frozen
        // packed store), so the on-disk bytes are reproducible across enriches.
        let mut utxos_unspent: Vec<(Vec<u8>, u64, i64, u8)> = Vec::new();

        for (slot, vout) in utxos.iter() {
            total_utxos_checked += 1;

            // Resolve the in-memory (slot, vout) back to the 32-byte DISPLAY txid
            // (tx_rev[slot]) for the spent check, the on-disk UTXO bytes, and HODL.
            // INVARIANT #7: same byte order Pass 1 used to fill spent_outputs.
            let txid32: [u8; 32] = tx_rev[*slot as usize];
            let is_spent = spent_outputs.contains(&(txid32, *vout));

            if is_spent {
                total_spent_found += 1;
            }

            if !is_spent {
                // Source value + kind from the frozen packed tx (the same store HODL
                // reads). kind = PackedTx.ty (already the ty_to_u8 byte). Defensive
                // None on an out-of-range vout (never expected) -> value 0.
                let ptx = &packed[*slot as usize];
                let value = ptx.outs.get(*vout as usize).map(|o| o.value).unwrap_or(0);
                utxos_unspent.push((txid32.to_vec(), *vout as u64, value, ptx.ty));

                // HODL: bucket this UTXO's value by coin age, counting each
                // outpoint exactly once (pure in-memory lookups; no DB reads).
                if tip > 0 && hodl_seen.insert((txid32, *vout)) {
                    let h = ptx.height;
                    if h >= 0 && h <= tip {
                        // Parser sets output.index == position, so direct indexing
                        // by vout is exact. Value comes from packed[slot].outs[vout]
                        // — identical to the original tx.outputs[vout].value.
                        if let Some(out) = ptx.outs.get(*vout as usize) {
                            let band_idx = hodl_band_index(tip, h);
                            hodl_sums[band_idx] = hodl_sums[band_idx].saturating_add(out.value);
                            hodl_total = hodl_total.saturating_add(out.value);
                        }
                    }
                }
            }
        }

        // Serialize the v2 49-byte 'a' records (txid+vout+value+kind).
        let serialized_utxos = serialize_addr_utxos(&utxos_unspent).await;
        batch.put_cf(&cf_addr_index, &key, &serialized_utxos);

        // Get pre-calculated totals from Pass 2 and 2b (MUCH faster than recalculating!)
        let total_received = *totals_received.get(&id).unwrap_or(&0);
        let total_sent = *totals_sent.get(&id).unwrap_or(&0);

        // Write transaction list ('t' + address) as v2 36-byte records:
        // txid(32) + canonical height(i32 LE). Height resolved via the frozen
        // tx_index -> packed[slot].height. The inline height is AUTHORITATIVE (it is
        // not re-read from the tx CF), which is what makes /address O(page) on read.
        if let Some(txids) = txs_map.get(&id) {
            let mut unique_txids = txids.clone();
            unique_txids.sort();
            unique_txids.dedup();

            let mut tx_entries: Vec<(Vec<u8>, i32)> = Vec::with_capacity(unique_txids.len());
            for txid in unique_txids {
                // Resolve the txid to its canonical height (missing -> 0, never expected).
                let height = <[u8; 32]>::try_from(txid.as_slice())
                    .ok()
                    .and_then(|t| tx_index.get(&t).copied())
                    .map(|slot| packed[slot as usize].height)
                    .unwrap_or(0);
                tx_entries.push((txid, height));
            }
            let txs_serialized = serialize_addr_txs(&tx_entries).await;
            let mut tx_list_key = vec![b't'];
            tx_list_key.extend_from_slice(address.as_bytes());
            batch.put_cf(&cf_addr_index, &tx_list_key, &txs_serialized);
        }

        // Write total received ('r' + address) - i64 LE bytes
        let mut key_r = vec![b'r'];
        key_r.extend_from_slice(address.as_bytes());
        batch.put_cf(&cf_addr_index, &key_r, total_received.to_le_bytes());

        // Write total sent ('s' + address) - i64 LE bytes
        let mut key_s = vec![b's'];
        key_s.extend_from_slice(address.as_bytes());
        batch.put_cf(&cf_addr_index, &key_s, total_sent.to_le_bytes());

        written += 1;

        if batch.len() >= batch_size {
            db.write(batch)?;
            batch = rocksdb::WriteBatch::default();
        }
    }

    // Write final batch
    if !batch.is_empty() {
        db.write(batch)?;
    }

    let spent_rate = if total_utxos_checked > 0 {
        (total_spent_found as f64 / total_utxos_checked as f64) * 100.0
    } else {
        0.0
    };

    info!(
        transactions_scanned = processed,
        outputs_indexed = indexed_outputs,
        spent_outputs = spent_outputs.len(),
        unique_addresses = written,
        total_utxos_checked = total_utxos_checked,
        spent_found = total_spent_found,
        spent_rate = spent_rate,
        "Address index building complete"
    );

    // UPDATE METRICS: Set current counts for Grafana/Prometheus
    let total_unspent_utxos = total_utxos_checked - total_spent_found;
    metrics::set_total_addresses_indexed(total_addresses as u64);
    metrics::set_total_utxos_tracked(total_unspent_utxos as u64);
    metrics::set_sapling_transactions_count(pass1_sapling_count);
    metrics::increment_sapling_transactions(pass1_sapling_count);

    info!(
        metric_addresses = total_addresses,
        metric_utxos_tracked = total_unspent_utxos,
        metric_sapling_tx = pass1_sapling_count,
        "Metrics updated: addresses, UTXOs, and Sapling transactions"
    );

    // Persist metrics to database after enrichment completes
    if let Err(e) = metrics::save_metrics_to_db(&db) {
        warn!(error = %e, "Failed to persist metrics to database after enrichment");
    } else {
        info!("Metrics persisted to database after enrichment");
    }

    // Free the big enrichment working set before the persist passes. The packed
    // tx store (packed/tx_index/tx_rev, ~4GB), the spent-outpoint set and the
    // HODL-dedup set are all dead once the write loop has run, and
    // persist_tx_daily_series does its OWN independent iterator pass (it does not
    // read any of these). Dropping them here keeps the peak at the Pass-2b crest
    // (~6.6GB) instead of letting the persist phase stack on ~5GB of dead memory.
    drop(packed);
    drop(tx_index);
    drop(tx_rev);
    drop(spent_outputs);
    drop(hodl_seen);

    // Precompute the rich list and wealth distribution from the per-address
    // totals we already have. balance == received - sent (verified to match
    // Blockbook), so this needs no extra DB reads and produces the TRUE top
    // holders — replacing the old O(addresses) full-scan endpoints that only
    // sampled the first 10k addresses.
    let wealth_ok = match persist_wealth_analytics(&db, &totals_received, &totals_sent, &addr_rev) {
        Ok(()) => true,
        Err(e) => {
            warn!(error = %e, "Failed to persist wealth analytics");
            false
        }
    };

    // HODL / dormancy snapshot: value of the final unspent UTXO set bucketed
    // by coin age, accumulated above from the same deduped unspent sets that
    // back the 'a' entries (the balance path verified against the reference
    // explorer — see the comment at the accumulators).
    if let Err(e) = persist_hodl_snapshot(&db, &hodl_sums, hodl_total, tip) {
        warn!(error = %e, "Failed to persist HODL snapshot");
    }

    // The full enrich rebuilt a/r/s/t, so durably (in the MAIN pass, not the
    // crash-prone detached daily-series tail) clear any reorg-stale flag — the
    // periodic recompute can resume. Advance its watermark to this tip ONLY if the
    // fresh richlist/wealth blob actually wrote; otherwise just clear the flag and
    // let the next recompute refresh the blob (never let the watermark outrun it).
    if wealth_ok {
        crate::analytics_recompute::mark_index_clean(&db, tip);
    } else {
        crate::analytics_recompute::clear_dirty_only(&db);
    }

    // [Lever 2] Defer the daily transaction time-series (~56 min -- a full
    // independent rescan of all 12M txs) OFF the critical path: run it on a
    // detached background thread so the explorer goes live ~56 min sooner. It
    // writes only analytics_tx_day* / treasury keys, which the balance/tx API
    // never reads (analytics readers are null-safe), so balances are unaffected.
    // The owned accumulators are MOVED into the thread (otherwise dropped at
    // return). A detached std::thread + current-thread runtime is used (not
    // tokio::spawn) because persist_tx_daily_series holds a RocksDB iterator
    // across .await and so is !Send. On success it sets `analytics_complete`.
    // NOTE: a crash during this background pass leaves the daily-series unbuilt
    // until a re-enrichment -- analytics only, never a balance.
    // Live-analytics gate (DESIGN-live-analytics-update.md §0): park the live
    // updater by clearing `analytics_live_ready` BEFORE the detached pass, so live
    // can never write a day blob concurrently with this enrich nor onto a partial
    // baseline. The pass re-greens it strictly-last on success (below).
    if let Some(cf_state) = db.cf_handle("chain_state") {
        // Mark the join daily-series IN-FLIGHT before the detached thread starts.
        // If the process restarts before the thread sets analytics_complete (below),
        // startup sees this marker and recovers as a FULL re-enrich rather than the
        // degraded fallback (which would zero the join fields AND set
        // analytics_complete, permanently wedging the DB in degraded state).
        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(&cf_state, crate::analytics_live::K_READY, [0u8]);
        batch.put_cf(&cf_state, b"analytics_in_progress", [1u8]);
        let _ = db.write(batch);
    }
    let db_bg = Arc::clone(&db);
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                warn!(error = %e, "Background: failed to build runtime for daily series");
                return;
            }
        };
        rt.block_on(async move {
            info!("Background: precomputing transaction daily series");
            if let Err(e) = persist_tx_daily_series(
                &db_bg,
                tip,
                &block_times,
                &block_bits,
                &day_joins,
                &coinstake_treasury,
            )
            .await
            {
                warn!(error = %e, "Background: failed to persist transaction daily series");
                return;
            }
            // Strictly-last handoff: flip the live gate green WITH the watermark
            // (= this enrich's frozen tip; live takes over at tip+1) and the
            // degraded gate, in ONE batch. Live resumes only on this valid,
            // join-complete baseline.
            if let Some(cf_state) = db_bg.cf_handle("chain_state") {
                let mut batch = rocksdb::WriteBatch::default();
                batch.put_cf(&cf_state, b"analytics_complete", [1u8]);
                batch.put_cf(
                    &cf_state,
                    crate::analytics_live::K_WATERMARK,
                    tip.to_le_bytes(),
                );
                batch.put_cf(&cf_state, crate::analytics_live::K_READY, [1u8]);
                // Join series is durably persisted — clear the in-flight marker so a
                // later restart does NOT trigger the interrupted-enrich recovery, and
                // reset the recovery-attempt counter (the circuit-breaker budget).
                batch.delete_cf(&cf_state, b"analytics_in_progress");
                batch.delete_cf(&cf_state, b"analytics_recovery_attempts");
                let _ = db_bg.write(batch);
            }
            info!("Background: transaction daily series complete - analytics ready");
        });
    });

    Ok(())
}

/// Daily transaction aggregate stored per date in chain_state.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct TxDayAgg {
    pub tx_count: u64,
    pub coinstake: u64,
    pub coinbase: u64,
    pub payment: u64,
    pub volume: i64,
    /// Sum of coinstake output values (stake principal re-mint + rewards).
    #[serde(default)]
    pub stake_volume: i64,
    /// Unique staker addresses seen in coinstakes this day.
    #[serde(default)]
    pub unique_stakers: u64,
    /// Average PoS difficulty across the day's blocks (from header nBits).
    #[serde(default)]
    pub avg_difficulty: f64,
    /// Canonical blocks this day.
    #[serde(default)]
    pub blocks: u64,
    /// Sum of raw transaction bytes this day (block size ~= tx_bytes + headers).
    #[serde(default)]
    pub tx_bytes: u64,
    /// Stale (non-canonical) blocks observed in blk files dated this day.
    #[serde(default)]
    pub orphan_blocks: u64,
    /// Unique addresses appearing in any output this day.
    #[serde(default)]
    pub active_addresses: u64,
    /// Addresses seen for the first time ever on this day (first-seen date).
    #[serde(default)]
    pub new_addresses: u64,
    /// Transactions carrying Sapling (shield) data this day.
    #[serde(default)]
    pub sapling_txs: u64,
    /// 95th-percentile block interval (seconds) across the day's blocks.
    #[serde(default)]
    pub interval_p95_secs: u64,
    /// Longest block interval (seconds) across the day's blocks.
    #[serde(default)]
    pub interval_max_secs: u64,
    /// Blocks won by the day's top-10 stakers (concentration metric).
    #[serde(default)]
    pub top10_blocks: u64,
    /// Total fees paid by the day's Normal txs (sats; prevout-joined).
    #[serde(default)]
    pub fees_total: i64,
    /// Raw byte total of the day's Normal txs (avg fee/byte basis).
    #[serde(default)]
    pub normal_tx_bytes: u64,
    /// Total minted staking rewards this day (coinstake outputs - inputs,
    /// sats), EXCLUDING budget/superblock payouts (era emission only).
    #[serde(default)]
    pub rewards_total: i64,
    /// Staker-share rewards this day (era emission minus masternode payment,
    /// per the PIVX Core v5.6.1 schedule in src/emission.rs; sats).
    #[serde(default)]
    pub staker_rewards_total: i64,
    /// Coin days destroyed this day (PIV * days, per spent input).
    #[serde(default)]
    pub coin_days_destroyed: f64,
    /// Value newly delegated into P2CS (cold staking) outputs this day (sats).
    #[serde(default)]
    pub p2cs_created: i64,
    /// P2CS value spent (undelegated or restaked) this day (sats).
    #[serde(default)]
    pub p2cs_spent: i64,
    /// Coinstakes that staked a P2CS (cold-staking) delegation this day —
    /// subset of `coinstake`, identified by P2CS re-mint outputs.
    #[serde(default)]
    pub coldstake_txs: u64,
}

/// Per-day aggregates that require prevout joins; accumulated during Pass 2b
/// (which already loads the previous transaction for every input) and merged
/// into the persisted TxDayAgg series.
#[derive(Default, Clone)]
pub struct DayJoinAgg {
    pub fees_total: i64,
    pub normal_tx_bytes: u64,
    /// Minted coinstake value excluding budget payouts (era emission only).
    pub rewards_total: i64,
    /// Staker share of the day's coinstake rewards (excl. masternode share).
    pub staker_rewards_total: i64,
    pub coin_days_destroyed: f64,
    pub p2cs_created: i64,
    pub p2cs_spent: i64,
}

/// HODL / dormancy snapshot: unspent value bucketed by coin age bands.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct HodlSnapshot {
    /// (band label, unspent value in sats) — oldest band last.
    pub bands: Vec<(String, i64)>,
    pub total: i64,
}

/// One budget/treasury payout: value minted in excess of the era's scheduled
/// block reward (PIVX Core v5.6.1 GetBlockValue; see src/emission.rs).
/// PoW era: extra coinbase outputs. PoS era: minted inside the coinstake at
/// heights at/after each 43200-block budget cycle, one proposal per block.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct TreasuryPayout {
    pub height: i32,
    pub date: String,
    /// The minted excess over era_block_reward(height), sats.
    pub total_paid_sats: i64,
    pub n_outputs: u32,
}

/// Convert compact nBits to difficulty (diff1 target 0x1d00ffff convention).
pub(crate) fn nbits_to_difficulty(nbits: u32) -> f64 {
    let exp = (nbits >> 24) as i32;
    let mant = (nbits & 0x00ff_ffff) as f64;
    if mant == 0.0 {
        return 0.0;
    }
    // difficulty = (0xffff * 256^(0x1d-3)) / (mant * 256^(exp-3))
    (65535.0 * 256f64.powi(0x1d - 3)) / (mant * 256f64.powi(exp - 3))
}

/// Convert a unix timestamp to a UTC YYYY-MM-DD date string (civil-from-days).
pub fn unix_to_date(ts: u64) -> String {
    let days = (ts / 86_400) as i64;
    // Howard Hinnant's civil_from_days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Build a height -> block nTime index by reading every canonical block header.
fn build_block_times(
    db: &Arc<DB>,
    tip: i32,
) -> Result<(Vec<u32>, Vec<u32>), Box<dyn std::error::Error>> {
    let cf_metadata = db
        .cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    let cf_blocks = db.cf_handle("blocks").ok_or("blocks CF not found")?;
    let mut times = vec![0u32; (tip as usize) + 1];
    let mut bits = vec![0u32; (tip as usize) + 1];
    for height in 0..=tip {
        let height_key = height.to_le_bytes();
        // chain_metadata: height -> display_hash (reversed)
        let display_hash = match db.get_cf(&cf_metadata, height_key)? {
            Some(h) => h,
            None => continue,
        };
        let internal_hash: Vec<u8> = display_hash.iter().rev().cloned().collect();
        // blocks: internal_hash -> header bytes; nTime is at offset 68 (4+32+32)
        if let Some(header) = db.get_cf(&cf_blocks, &internal_hash)? {
            if header.len() >= 76 {
                times[height as usize] =
                    u32::from_le_bytes(header[68..72].try_into().unwrap_or([0; 4]));
                bits[height as usize] =
                    u32::from_le_bytes(header[72..76].try_into().unwrap_or([0; 4]));
            }
        }
    }
    Ok((times, bits))
}

/// Canonical iff the reverse 'h' hint (EITHER byte order — parse writes
/// 'h'+internal, legacy live-monitor wrote 'h'+display) resolves to a height whose
/// FORWARD map entry round-trips back to this (internal) `hash`. Confirming against
/// the forward map (correctly maintained on reorg) means a stale/leaked 'h' entry
/// cannot make a genuine orphan look canonical.
fn is_canonical_hash(
    db: &Arc<DB>,
    cf_metadata: &impl rocksdb::AsColumnFamilyRef,
    hash: &[u8],
) -> bool {
    let mut h_internal = vec![b'h'];
    h_internal.extend_from_slice(hash);
    let mut h_display = vec![b'h'];
    h_display.extend(hash.iter().rev());
    for hk in [&h_internal, &h_display] {
        let Ok(Some(hb)) = db.get_cf(cf_metadata, hk) else {
            continue;
        };
        if hb.len() != 4 {
            continue;
        }
        let height = i32::from_le_bytes([hb[0], hb[1], hb[2], hb[3]]);
        if let Ok(Some(fwd)) = db.get_cf(cf_metadata, height.to_le_bytes()) {
            if fwd.len() == 32 && fwd.iter().rev().eq(hash.iter()) {
                return true;
            }
        }
    }
    false
}

/// Header nTime of the canonical block at `height` (forward map → blocks CF).
fn header_ntime_at(
    db: &Arc<DB>,
    cf_metadata: &impl rocksdb::AsColumnFamilyRef,
    cf_blocks: &impl rocksdb::AsColumnFamilyRef,
    height: i32,
) -> Option<u32> {
    let display = db.get_cf(cf_metadata, height.to_le_bytes()).ok()??;
    let internal: Vec<u8> = display.iter().rev().cloned().collect();
    let header = db.get_cf(cf_blocks, &internal).ok()??;
    if header.len() >= 72 {
        Some(u32::from_le_bytes(header[68..72].try_into().ok()?))
    } else {
        None
    }
}

/// Persistent orphan keyspace (in `chain_state`). `tail_blocks` is EPHEMERAL —
/// blk-tail prunes settled records ~K_RETENTION blocks past the tip — so orphans
/// must be PERSISTED as discovered, never re-derived from a scan (else a pruned
/// tail orphan would silently drop out of the count). `orphanseen:<hash>` → date is
/// the append-only dedup marker; `orphancount:<date>` → u64 LE is the count, bumped
/// atomically with each new marker. A settled orphan never un-orphans (that needs a
/// deep reorg, ~impossible on PIVX), so the markers are never removed.
pub const ORPHAN_SEEN_PFX: &[u8] = b"orphanseen:";
pub const ORPHAN_COUNT_PFX: &[u8] = b"orphancount:";

/// The persisted orphan count for a date (= `orphan_blocks`), 0 if unset.
pub fn orphan_count(db: &Arc<DB>, date: &str) -> u64 {
    db.cf_handle("chain_state")
        .and_then(|cf| {
            let mut k = ORPHAN_COUNT_PFX.to_vec();
            k.extend_from_slice(date.as_bytes());
            db.get_cf(&cf, &k).ok().flatten()
        })
        .filter(|v| v.len() == 8)
        .map(|v| u64::from_le_bytes(v[..8].try_into().unwrap()))
        .unwrap_or(0)
}

/// Mark one non-canonical block (internal `hash`, header time `t`) as a NEW orphan
/// unless already seen this run or persisted in a prior run.
fn mark_one(
    db: &Arc<DB>,
    cf_state: &impl rocksdb::AsColumnFamilyRef,
    batch: &mut rocksdb::WriteBatch,
    seen: &mut HashSet<[u8; 32]>,
    new_by_date: &mut HashMap<String, u64>,
    hash: &[u8],
    t: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut h = [0u8; 32];
    h.copy_from_slice(hash);
    if !seen.insert(h) {
        return Ok(()); // already processed this run
    }
    let mut mk = ORPHAN_SEEN_PFX.to_vec();
    mk.extend_from_slice(hash);
    if db.get_cf(cf_state, &mk)?.is_some() {
        return Ok(()); // already persisted in a prior run
    }
    let date = unix_to_date(t as u64);
    batch.put_cf(cf_state, &mk, date.as_bytes());
    *new_by_date.entry(date).or_insert(0) += 1;
    Ok(())
}

/// Discover stale (non-canonical) blocks past the 2h tip-window exemption and
/// PERSIST each NEW one into the orphan index (marker + per-date count), so it
/// survives blk-tail's pruning of the ephemeral `tail_blocks` CF and a reorg
/// rebuild. Idempotent: a block already marked (this run or prior) is never
/// recounted.
/// - `tail_only=false` (full enrich): scans the canonical `blocks` CF AND
///   `tail_blocks` to build the baseline.
/// - `tail_only=true` (Lane R, ~hourly): scans ONLY `tail_blocks` (~hundreds of
///   records) — no blocks-CF iterate, so it is cheap enough to run every tick.
/// A main-CF orphan is dated by its own header nTime; a tail orphan (no stored
/// nTime) by the canonical block's time at its claimed height. Dedup by internal hash.
pub fn mark_orphans(
    db: &Arc<DB>,
    tip: i32,
    tip_time: u32,
    tail_only: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    let cf_blocks = db.cf_handle("blocks").ok_or("blocks CF not found")?;
    let cf_metadata = db
        .cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    let mut batch = rocksdb::WriteBatch::default();
    let mut new_by_date: HashMap<String, u64> = HashMap::new();
    let mut seen: HashSet<[u8; 32]> = HashSet::new();

    // 1. Canonical blocks CF — stale headers dated by their own nTime (full only).
    if !tail_only {
        for item in db.iterator_cf(&cf_blocks, rocksdb::IteratorMode::Start) {
            let (key, header) = item?;
            let hash: &[u8] = if key.len() == 33 && key[0] == b'b' {
                &key[1..]
            } else {
                &key[..]
            };
            if hash.len() != 32 || header.len() < 72 {
                continue;
            }
            let t = u32::from_le_bytes(header[68..72].try_into().unwrap_or([0; 4]));
            if t == 0 || (tip_time > 0 && t + 7_200 > tip_time) {
                continue;
            }
            if is_canonical_hash(db, &cf_metadata, hash) {
                continue;
            }
            mark_one(
                db,
                &cf_state,
                &mut batch,
                &mut seen,
                &mut new_by_date,
                hash,
                t,
            )?;
        }
    }

    // 2. blk-tail private capture — dated by the canonical nTime at claimed_height
    // (records are `b'b' || hash(32)`; claimed_height is a LE i32 at value offset 33;
    // the `b'i'` index keys are 38 bytes and skipped).
    if let Some(cf_tail) = db.cf_handle("tail_blocks") {
        for item in db.iterator_cf(&cf_tail, rocksdb::IteratorMode::Start) {
            let (key, value) = item?;
            if key.len() != 33 || key[0] != b'b' || value.len() < 37 {
                continue;
            }
            let claimed_height = i32::from_le_bytes(value[33..37].try_into().unwrap_or([0; 4]));
            // Settle a tail orphan by DEPTH (blk-tail's own K_CONFIRM), not by a
            // wall-clock 2h exemption: depth is reached well before the K_RETENTION
            // prune AND is independent of block spacing, so a fast-block burst can't
            // prune the record before the exemption lifts. (The main-CF path keeps
            // the time exemption — it has each header's own nTime.)
            if claimed_height > tip - crate::blk_tail::K_CONFIRM {
                continue;
            }
            let Some(t) = header_ntime_at(db, &cf_metadata, &cf_blocks, claimed_height) else {
                continue;
            };
            if t == 0 {
                continue;
            }
            let hash = &key[1..];
            if is_canonical_hash(db, &cf_metadata, hash) {
                continue;
            }
            mark_one(
                db,
                &cf_state,
                &mut batch,
                &mut seen,
                &mut new_by_date,
                hash,
                t,
            )?;
        }
    }

    // Bump the per-date counts for the newly-marked orphans, atomic with the markers.
    for (date, n) in new_by_date {
        let mut ck = ORPHAN_COUNT_PFX.to_vec();
        ck.extend_from_slice(date.as_bytes());
        let cur = db
            .get_cf(&cf_state, &ck)?
            .filter(|v| v.len() == 8)
            .map(|v| u64::from_le_bytes(v[..8].try_into().unwrap()))
            .unwrap_or(0);
        batch.put_cf(&cf_state, &ck, (cur + n).to_le_bytes());
    }
    db.write(batch)?;
    Ok(())
}

/// Record an address occurrence for the active/new-address daily metrics.
/// The transaction scan is NOT chronological, so first-seen is tracked as a
/// per-address MINIMUM date and bucketed afterwards.
fn note_address(
    addr: String,
    date: &str,
    day_active: &mut HashMap<String, HashSet<String>>,
    first_seen: &mut HashMap<String, String>,
) {
    match first_seen.get_mut(&addr) {
        Some(existing) => {
            if date < existing.as_str() {
                *existing = date.to_string();
            }
        }
        None => {
            first_seen.insert(addr.clone(), date.to_string());
        }
    }
    day_active.entry(date.to_string()).or_default().insert(addr);
}

/// Tier-A analytics recovery (Lever 2 follow-up): rebuild the daily transaction
/// series when the deferred background pass never finished (e.g. a crash). This is
/// the DEGRADED path -- it rebuilds block_times from chain headers and recomputes
/// every NON-join daily metric (tx counts, volumes, difficulty, stakers, active
/// addresses, PoW-era treasury) with EMPTY join aggregates, so only the
/// prevout-join-derived fields (PoS-era treasury, fees/rewards, coin-days) are
/// omitted until a full re-enrich. Balance-neutral: writes only analytics_* keys,
/// never 'a'/'r'/'s'/'t'. Sets `analytics_complete` on success.
pub async fn rebuild_daily_series_degraded(db: Arc<DB>) -> Result<(), Box<dyn std::error::Error>> {
    let tip = crate::chain_state::get_sync_height(&db).unwrap_or(0);
    if tip <= 0 {
        return Ok(());
    }
    let (block_times, block_bits) = build_block_times(&db, tip)?;
    let empty_joins: HashMap<String, DayJoinAgg> = HashMap::new();
    let empty_treasury: Vec<TreasuryPayout> = Vec::new();
    persist_tx_daily_series(
        &db,
        tip,
        &block_times,
        &block_bits,
        &empty_joins,
        &empty_treasury,
    )
    .await?;
    if let Some(cf_state) = db.cf_handle("chain_state") {
        db.put_cf(&cf_state, b"analytics_complete", [1u8])?;
        // The degraded base has ZEROED join fields, so it is NOT a valid live
        // baseline: clear the live gate so Lane I/R stay dark until a full
        // re-enrich re-greens it (DESIGN-live-analytics-update.md §0).
        let _ = db.put_cf(&cf_state, crate::analytics_live::K_READY, [0u8]);
    }
    info!("Degraded daily-series rebuild complete (join-derived fields omitted until a full re-enrich)");
    Ok(())
}

async fn persist_tx_daily_series(
    db: &Arc<DB>,
    tip: i32,
    block_times: &[u32],
    block_bits: &[u32],
    day_joins: &HashMap<String, DayJoinAgg>,
    coinstake_treasury: &[TreasuryPayout],
) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    let cf_transactions = db
        .cf_handle("transactions")
        .ok_or("transactions CF not found")?;

    if tip <= 0 || block_times.len() <= tip as usize {
        return Ok(());
    }

    let mut days: HashMap<String, TxDayAgg> = HashMap::new();
    let mut day_stakers: HashMap<String, HashMap<String, u64>> = HashMap::new();
    let mut day_active: HashMap<String, HashSet<String>> = HashMap::new();
    let mut first_seen: HashMap<String, String> = HashMap::new();
    let mut treasury: Vec<TreasuryPayout> = Vec::new();
    let iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    for item in iter {
        let (key, value) = item?;
        if key.first() == Some(&b'B') || value.len() < 8 {
            continue;
        }
        let height = i32::from_le_bytes(value[4..8].try_into().unwrap_or([0, 0, 0, 0]));
        if !should_index_transaction(height) || height < 0 || height > tip {
            continue;
        }
        let t = block_times[height as usize];
        if t == 0 {
            continue;
        }
        let raw_tx = &value[8..];
        let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
        tx_with_header.extend_from_slice(&[0u8; 4]);
        tx_with_header.extend_from_slice(raw_tx);
        let tx = match deserialize_transaction(&tx_with_header).await {
            Ok(tx) => tx,
            Err(_) => continue,
        };

        let agg_date = unix_to_date(t as u64);
        let tx_type = crate::tx_type::detect_transaction_type(&tx);
        // Inline counts/volumes/type + the PoW-coinbase budget, via the SHARED
        // accumulator (Lane I calls the exact same fn — no drift on coldstake_txs
        // or the >10 PIV budget threshold).
        let pow_treasury = {
            let agg = days.entry(agg_date.clone()).or_default();
            accumulate_tx_inline(agg, &tx, tx_type, raw_tx.len(), height, &agg_date)
        };
        if let Some(t) = pow_treasury {
            treasury.push(t);
        }
        // The day's stakers (for unique_stakers / top10 concentration) — a Lane-R
        // set field; the staker is the first address of the first paying output.
        if tx_type == crate::tx_type::TransactionType::Coinstake {
            if let Some(addr) = tx
                .outputs
                .iter()
                .find(|o| !o.address.is_empty())
                .and_then(|o| o.address.first())
            {
                *day_stakers
                    .entry(agg_date.clone())
                    .or_default()
                    .entry(addr.clone())
                    .or_insert(0) += 1;
            }
        }

        // Active / first-seen address tracking from output attributions (same
        // classification rules as the address index, incl. both P2CS sides).
        for (vout_index, output) in tx.outputs.iter().enumerate() {
            if tx_type == crate::tx_type::TransactionType::Coinstake && vout_index == 0 {
                continue; // coinstake marker output
            }
            match classify_output(output) {
                ScriptClassification::P2PKH(addr)
                | ScriptClassification::P2SH(addr)
                | ScriptClassification::P2PK(addr) => {
                    note_address(addr, &agg_date, &mut day_active, &mut first_seen);
                }
                ScriptClassification::ColdStake { staker, owner } => {
                    note_address(staker, &agg_date, &mut day_active, &mut first_seen);
                    note_address(owner, &agg_date, &mut day_active, &mut first_seen);
                }
                _ => {}
            }
        }
    }

    // Per-day average difficulty and block count from the canonical headers.
    let mut day_diff: HashMap<String, (f64, u64)> = HashMap::new();
    for h in 0..=(tip as usize) {
        if block_times[h] == 0 {
            continue;
        }
        let d = unix_to_date(block_times[h] as u64);
        let e = day_diff.entry(d).or_insert((0.0, 0));
        e.0 += nbits_to_difficulty(block_bits[h]);
        e.1 += 1;
    }
    // Discover + PERSIST all orphans (blocks CF + blk-tail) into the orphan index,
    // then set each day's orphan_blocks from the persistent count (incl. orphan-only
    // days). The count survives blk-tail pruning + reorg, so Lane R just reads it.
    {
        let tip_time = block_times[tip as usize];
        mark_orphans(db, tip, tip_time, false)?;
        for item in db.prefix_iterator_cf(&cf_state, ORPHAN_COUNT_PFX) {
            let (k, v) = item?;
            if !k.starts_with(ORPHAN_COUNT_PFX) {
                break;
            }
            if v.len() == 8 {
                let date = String::from_utf8_lossy(&k[ORPHAN_COUNT_PFX.len()..]).to_string();
                days.entry(date).or_default().orphan_blocks =
                    u64::from_le_bytes(v[..8].try_into().unwrap());
            }
        }
    }

    for (date, (diff_sum, blocks)) in &day_diff {
        if let Some(agg) = days.get_mut(date) {
            agg.avg_difficulty = if *blocks > 0 {
                diff_sum / *blocks as f64
            } else {
                0.0
            };
            agg.blocks = *blocks;
        }
    }

    for (date, stakers) in &day_stakers {
        if let Some(agg) = days.get_mut(date) {
            agg.unique_stakers = stakers.len() as u64;
            // Staker concentration: blocks won by the day's top-10 stakers.
            let mut counts: Vec<u64> = stakers.values().copied().collect();
            counts.sort_unstable_by(|a, b| b.cmp(a));
            agg.top10_blocks = counts.iter().take(10).sum();
        }
    }

    // Per-day block interval distribution (p95 / max) from consecutive header
    // times; an interval is attributed to the day of the LATER block.
    let mut day_intervals: HashMap<String, Vec<u32>> = HashMap::new();
    for h in 1..=(tip as usize) {
        if block_times[h] == 0 || block_times[h - 1] == 0 {
            continue;
        }
        let dt = block_times[h].saturating_sub(block_times[h - 1]);
        day_intervals
            .entry(unix_to_date(block_times[h] as u64))
            .or_default()
            .push(dt);
    }
    for (date, mut intervals) in day_intervals {
        if let Some(agg) = days.get_mut(&date) {
            intervals.sort_unstable();
            let n = intervals.len();
            agg.interval_p95_secs = intervals[(n * 95 / 100).min(n - 1)] as u64;
            agg.interval_max_secs = intervals[n - 1] as u64;
        }
    }

    // Active / new address counts.
    for (date, addrs) in &day_active {
        if let Some(agg) = days.get_mut(date) {
            agg.active_addresses = addrs.len() as u64;
        }
    }
    let mut new_per_day: HashMap<&str, u64> = HashMap::new();
    for date in first_seen.values() {
        *new_per_day.entry(date.as_str()).or_insert(0) += 1;
    }
    for (date, n) in &new_per_day {
        if let Some(agg) = days.get_mut(*date) {
            agg.new_addresses = *n;
        }
    }

    // Merge the Tier-2 prevout-join aggregates accumulated during Pass 2b.
    for (date, j) in day_joins {
        let agg = days.entry(date.clone()).or_default();
        agg.fees_total = j.fees_total;
        agg.normal_tx_bytes = j.normal_tx_bytes;
        agg.rewards_total = j.rewards_total;
        agg.staker_rewards_total = j.staker_rewards_total;
        agg.coin_days_destroyed = j.coin_days_destroyed;
        agg.p2cs_created = j.p2cs_created;
        agg.p2cs_spent = j.p2cs_spent;
    }

    let mut dates: Vec<String> = days.keys().cloned().collect();
    dates.sort();
    let mut batch = rocksdb::WriteBatch::default();
    for (date, agg) in &days {
        let mut k = b"analytics_tx_day:".to_vec();
        k.extend_from_slice(date.as_bytes());
        batch.put_cf(&cf_state, &k, bincode::serialize(agg)?);
    }
    batch.put_cf(&cf_state, b"analytics_tx_days", bincode::serialize(&dates)?);
    // Tier 4: treasury payouts — PoW-era coinbase payouts (collected above)
    // merged with PoS-era coinstake payouts (collected in Pass 2b), sorted
    // by height.
    treasury.extend_from_slice(coinstake_treasury);
    treasury.sort_by_key(|t| t.height);
    batch.put_cf(
        &cf_state,
        b"analytics_treasury",
        bincode::serialize(&treasury)?,
    );
    db.write(batch)?;
    info!(
        days = dates.len(),
        treasury_payouts = treasury.len(),
        "Transaction daily series precomputed and stored"
    );
    Ok(())
}

/// Serializable rich-list entry stored in chain_state for O(1) API reads.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct RichListSnapshotEntry {
    pub address: String,
    pub balance: i64,
    pub tx_count: u64,
}

/// Snapshot of wealth distribution stored in chain_state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct WealthSnapshot {
    pub total_balance: i64,
    pub address_count: u64,
    /// Cumulative balance of the top 10/50/100/1000 holders.
    pub top_10: i64,
    pub top_50: i64,
    pub top_100: i64,
    pub top_1000: i64,
    /// Histogram bucket counts, same 7 ranges the API exposes.
    pub histogram: Vec<(String, u64)>,
    /// Gini coefficient over all positive balances (0 = equal, 1 = one holder).
    #[serde(default)]
    pub gini: f64,
    /// Minimum number of holders whose balances sum to >50% of the total.
    #[serde(default)]
    pub nakamoto_coefficient: u32,
}

/// Number of rich-list entries to retain (API serves a clamped slice of this).
pub(crate) const RICHLIST_KEEP: usize = 1000;

/// Pure, deterministic computation of the rich list + wealth snapshot from a set
/// of per-address balances (satoshis). The caller supplies the FINAL balance per
/// address (owner-attributed / deduped upstream — this function performs no
/// cross-address dedup); it then:
///   * keeps only strictly-positive balances (zero/negative is never a holder; a
///     negative would signal an upstream accounting bug, never a top-N entry);
///   * sorts by (balance DESC, address ASC) so ties — and richlist membership at
///     the `keep` cutoff — are reproducible regardless of input iteration order
///     (the previous HashMap-ordered, balance-only sort was nondeterministic);
///   * accumulates totals / top-N sums / the Gini weighted sum in i128, removing
///     the i64 intermediate-overflow footgun;
///   * derives the Nakamoto coefficient from an exact integer threshold
///     (`2*acc > total`) rather than an f64 compare that rounds above 2^53.
/// `tx_count_of` is invoked only for the kept top-N entries (cheap).
pub fn compute_wealth_richlist(
    mut balances: Vec<(String, i64)>,
    keep: usize,
    tx_count_of: impl Fn(&str) -> u64,
) -> (Vec<RichListSnapshotEntry>, WealthSnapshot) {
    balances.retain(|(_, b)| *b > 0);

    // Deterministic TOTAL order: balance descending, address ascending on ties.
    balances.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let richlist: Vec<RichListSnapshotEntry> = balances
        .iter()
        .take(keep)
        .map(|(address, balance)| RichListSnapshotEntry {
            address: address.clone(),
            balance: *balance,
            tx_count: tx_count_of(address),
        })
        .collect();

    // Totals + top-N sums in i128, then saturate back to i64. PIVX supply fits i64
    // comfortably; the wider accumulator only removes the overflow footgun.
    let total_i128: i128 = balances.iter().map(|(_, b)| *b as i128).sum();
    let total_balance = total_i128.min(i64::MAX as i128) as i64;
    let sum_take = |n: usize| -> i64 {
        let s: i128 = balances.iter().take(n).map(|(_, b)| *b as i128).sum();
        s.min(i64::MAX as i128) as i64
    };

    let histogram_ranges: [(i64, i64, &str); 7] = [
        (0, 1_00000000, "0-1 PIV"),
        (1_00000000, 10_00000000, "1-10 PIV"),
        (10_00000000, 100_00000000, "10-100 PIV"),
        (100_00000000, 1000_00000000, "100-1K PIV"),
        (1000_00000000, 10000_00000000, "1K-10K PIV"),
        (10000_00000000, 100000_00000000, "10K-100K PIV"),
        (100000_00000000, i64::MAX, "100K+ PIV"),
    ];
    let histogram: Vec<(String, u64)> = histogram_ranges
        .iter()
        .map(|(min, max, label)| {
            let count = balances
                .iter()
                .filter(|(_, b)| *b >= *min && *b < *max)
                .count() as u64;
            (label.to_string(), count)
        })
        .collect();

    // Gini (standard formula over ASCENDING balances):
    //   G = 2*Σ(i*x_i) / (n*Σx) - (n+1)/n   (i = 1-based ascending rank)
    // balances is sorted DESCENDING, so iterate in reverse for ascending order;
    // the weighted sum is accumulated exactly in i128 and cast once at the end.
    let n = balances.len();
    let gini = if n > 0 && total_i128 > 0 {
        let mut weighted: i128 = 0;
        for (i, (_, b)) in balances.iter().rev().enumerate() {
            weighted += (i as i128 + 1) * (*b as i128);
        }
        (2.0 * weighted as f64) / (n as f64 * total_i128 as f64) - (n as f64 + 1.0) / n as f64
    } else {
        0.0
    };

    // Nakamoto: minimum holders summing to >50% of total, exact integer test.
    let mut nakamoto_coefficient: u32 = 0;
    let mut acc: i128 = 0;
    for (_, b) in &balances {
        acc += *b as i128;
        nakamoto_coefficient += 1;
        if 2 * acc > total_i128 {
            break;
        }
    }

    let wealth = WealthSnapshot {
        total_balance,
        address_count: balances.len() as u64,
        top_10: sum_take(10),
        top_50: sum_take(50),
        top_100: sum_take(100),
        top_1000: sum_take(1000),
        histogram,
        gini,
        nakamoto_coefficient,
    };

    (richlist, wealth)
}

fn persist_wealth_analytics(
    db: &Arc<DB>,
    totals_received: &HashMap<u32, i64>,
    totals_sent: &HashMap<u32, i64>,
    addr_rev: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    let cf_addr = db
        .cf_handle("addr_index")
        .ok_or("addr_index CF not found")?;

    // Per-address balance = received - sent (the same value `/address` serves and
    // the periodic recompute uses). `compute_wealth_richlist` filters >0, applies
    // the deterministic (balance DESC, address ASC) order, and computes the
    // aggregates in i128 with an exact-integer Nakamoto threshold.
    let balances: Vec<(String, i64)> = totals_received
        .iter()
        .filter_map(|(&id, recv)| {
            let bal = recv - *totals_sent.get(&id).unwrap_or(&0);
            // Resolve (clone) the address string only for actual holders, so the
            // transient Vec stays bounded to positive balances during enrich.
            (bal > 0).then(|| (addr_rev[id as usize].clone(), bal))
        })
        .collect();

    // tx_count for the kept top-N from the deduped on-disk 't' list (already
    // written by the address-index batch above) — the SAME source the periodic
    // recompute reads, so the enrich and recompute snapshots agree at the same tip.
    let tx_count_of = |address: &str| -> u64 {
        let mut t_key = Vec::with_capacity(address.len() + 1);
        t_key.push(b't');
        t_key.extend_from_slice(address.as_bytes());
        db.get_cf(&cf_addr, &t_key)
            .ok()
            .flatten()
            .map(|b| (b.len() / crate::parser::ADDR_TX_STRIDE) as u64)
            .unwrap_or(0)
    };

    let (richlist, wealth) = compute_wealth_richlist(balances, RICHLIST_KEEP, tx_count_of);

    db.put_cf(
        &cf_state,
        b"analytics_richlist",
        bincode::serialize(&richlist)?,
    )?;
    db.put_cf(&cf_state, b"analytics_wealth", bincode::serialize(&wealth)?)?;
    info!(
        richlist_entries = richlist.len(),
        holders = wealth.address_count,
        gini = wealth.gini,
        nakamoto = wealth.nakamoto_coefficient,
        "Wealth analytics precomputed and stored"
    );
    Ok(())
}

/// HODL age bands in days (1440 blocks ~= 1 day): label, lower (incl), upper (excl).
const HODL_BANDS: [(&str, i64, i64); 6] = [
    ("<1m", 0, 30),
    ("1-3m", 30, 90),
    ("3-6m", 90, 180),
    ("6-12m", 180, 365),
    ("1-2y", 365, 730),
    (">2y", 730, i64::MAX),
];

/// Band index for a UTXO created at `create_height` with the chain at `tip`.
fn hodl_band_index(tip: i32, create_height: i32) -> usize {
    let age_days = ((tip - create_height) as i64) / 1440;
    HODL_BANDS
        .iter()
        .position(|(_, lo, hi)| age_days >= *lo && age_days < *hi)
        .unwrap_or(HODL_BANDS.len() - 1)
}

/// Persist the HODL / dormancy snapshot.
///
/// The band sums are accumulated in the 'a'-entry write loop from the deduped
/// per-address unspent UTXO sets — the exact data the address API serves, and
/// the path whose balances are verified against the reference explorer. Only
/// address-attributed outputs are counted: outputs with no address (chiefly
/// OP_ZEROCOINMINT) are excluded because zerocoin spends never consume the
/// mint outpoint, which made a spent-set-based walk over the tx cache count
/// every zPIV mint ever created as eternally unspent (~744M phantom PIV,
/// an 8.2x overcount of the transparent supply).
fn persist_hodl_snapshot(
    db: &Arc<DB>,
    sums: &[i64; HODL_BANDS.len()],
    total: i64,
    tip: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    if tip <= 0 {
        return Ok(());
    }
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;

    let snapshot = HodlSnapshot {
        bands: HODL_BANDS
            .iter()
            .zip(sums.iter())
            .map(|((label, _, _), v)| (label.to_string(), *v))
            .collect(),
        total,
    };
    db.put_cf(&cf_state, b"analytics_hodl", bincode::serialize(&snapshot)?)?;
    info!(
        total_sats = total,
        "HODL age-band snapshot precomputed and stored"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CScript;

    // ---- Step 1: pure compute_wealth_richlist (deterministic snapshot math) ----

    fn bal(addr: &str, sats: i64) -> (String, i64) {
        (addr.to_string(), sats)
    }

    #[test]
    fn wealth_richlist_ties_broken_by_address_ascending_order_independent() {
        // Four holders with the SAME balance, fed in two different orders. The
        // richlist must come out identical and address-ascending, so the persisted
        // bytes are reproducible regardless of (HashMap) input iteration order.
        let order_a = vec![
            bal("zeta", 100),
            bal("alpha", 100),
            bal("mike", 100),
            bal("bravo", 100),
        ];
        let order_b = vec![
            bal("bravo", 100),
            bal("zeta", 100),
            bal("alpha", 100),
            bal("mike", 100),
        ];
        let (rl_a, _) = compute_wealth_richlist(order_a, 1000, |_| 0);
        let (rl_b, _) = compute_wealth_richlist(order_b, 1000, |_| 0);
        let addrs_a: Vec<String> = rl_a.iter().map(|e| e.address.clone()).collect();
        assert_eq!(addrs_a, vec!["alpha", "bravo", "mike", "zeta"]);
        let addrs_b: Vec<String> = rl_b.iter().map(|e| e.address.clone()).collect();
        assert_eq!(addrs_a, addrs_b);
    }

    #[test]
    fn wealth_richlist_tie_at_cutoff_is_deterministic() {
        // keep=2, three equal balances: address-ascending decides which two are
        // kept, independent of input order.
        let in1 = vec![bal("c", 50), bal("a", 50), bal("b", 50)];
        let in2 = vec![bal("b", 50), bal("c", 50), bal("a", 50)];
        let (rl1, _) = compute_wealth_richlist(in1, 2, |_| 0);
        let (rl2, _) = compute_wealth_richlist(in2, 2, |_| 0);
        let kept1: Vec<String> = rl1.iter().map(|e| e.address.clone()).collect();
        assert_eq!(kept1, vec!["a", "b"]);
        let kept2: Vec<String> = rl2.iter().map(|e| e.address.clone()).collect();
        assert_eq!(kept1, kept2);
    }

    #[test]
    fn wealth_richlist_keeps_top_n_by_balance_with_tx_counts() {
        let balances = vec![bal("a", 10), bal("b", 500), bal("c", 250), bal("d", 1)];
        let tx = |addr: &str| match addr {
            "b" => 7u64,
            "c" => 3,
            _ => 0,
        };
        let (rl, w) = compute_wealth_richlist(balances, 2, tx);
        assert_eq!(rl.len(), 2);
        assert_eq!(rl[0].address, "b");
        assert_eq!(rl[0].balance, 500);
        assert_eq!(rl[0].tx_count, 7);
        assert_eq!(rl[1].address, "c");
        assert_eq!(rl[1].tx_count, 3);
        assert_eq!(w.address_count, 4);
        assert_eq!(w.total_balance, 761);
    }

    #[test]
    fn wealth_total_and_topn_use_i128_without_overflow() {
        // Two holders each at i64::MAX/2: the i128 accumulation must yield the
        // exact total and a correct Nakamoto coefficient via the exact integer
        // threshold (an i64 sum here is on the edge; an f64 compare would round).
        let half = i64::MAX / 2;
        let balances = vec![bal("a", half), bal("b", half)];
        let (_, w) = compute_wealth_richlist(balances, 1000, |_| 0);
        assert_eq!(w.total_balance, half * 2);
        assert_eq!(w.top_10, half * 2);
        // 2*acc after the first holder == total (not strictly >), so it takes both.
        assert_eq!(w.nakamoto_coefficient, 2);
    }

    #[test]
    fn wealth_nakamoto_single_dominant_holder() {
        let balances = vec![
            bal("whale", 1_000_000),
            bal("a", 1),
            bal("b", 1),
            bal("c", 1),
        ];
        let (_, w) = compute_wealth_richlist(balances, 1000, |_| 0);
        assert_eq!(w.nakamoto_coefficient, 1);
    }

    #[test]
    fn wealth_histogram_buckets_and_positive_filter() {
        // 0.5 / 5 / 50 PIV, plus a zero and a negative that MUST be dropped.
        let balances = vec![
            bal("half", 50_000_000),
            bal("five", 5_00000000),
            bal("fifty", 50_00000000),
            bal("zero", 0),
            bal("neg", -10),
        ];
        let (_, w) = compute_wealth_richlist(balances, 1000, |_| 0);
        assert_eq!(w.address_count, 3);
        assert_eq!(w.total_balance, 50_000_000 + 5_00000000 + 50_00000000);
        let count = |label: &str| {
            w.histogram
                .iter()
                .find(|(l, _)| l == label)
                .map(|(_, c)| *c)
                .unwrap()
        };
        assert_eq!(count("0-1 PIV"), 1);
        assert_eq!(count("1-10 PIV"), 1);
        assert_eq!(count("10-100 PIV"), 1);
        assert_eq!(count("100-1K PIV"), 0);
    }

    #[test]
    fn wealth_empty_input_is_all_zero() {
        let (rl, w) = compute_wealth_richlist(vec![], 1000, |_| 0);
        assert!(rl.is_empty());
        assert_eq!(w.total_balance, 0);
        assert_eq!(w.address_count, 0);
        assert_eq!(w.nakamoto_coefficient, 0);
        assert_eq!(w.gini, 0.0);
    }

    /// The shard bounds must form an EXACT partition of the 't' keyspace: floor
    /// at the prefix, ceiling past it, contiguous (no gap), disjoint (no overlap),
    /// every shard non-empty. This is the keystone that lets concatenated shard
    /// output equal serial slot order byte-for-byte.
    #[test]
    fn tx_shard_bounds_partition_is_exact() {
        for n in 1..=4 {
            let b = tx_shard_bounds(n);
            assert_eq!(b.len(), n, "n={n}");
            assert_eq!(b[0].0, vec![b't', 0], "floor at the 't' prefix");
            assert_eq!(b[n - 1].1, vec![b't' + 1], "ceiling past the 't' prefix");
            for (lo, hi) in &b {
                assert!(lo < hi, "empty shard range {lo:?}..{hi:?} at n={n}");
            }
            for k in 0..n - 1 {
                assert_eq!(b[k].1, b[k + 1].0, "gap/overlap at shard {k} (n={n})");
            }
        }
        // Clamp: > 4 collapses to 4 (RAM cap), 0 collapses to 1 (serial).
        assert_eq!(tx_shard_bounds(9).len(), 4);
        assert_eq!(tx_shard_bounds(0).len(), 1);
    }

    /// Every possible first txid byte must fall in EXACTLY one shard's [lo, hi),
    /// so the union of shards == the serial iterator's full visit set (each tx
    /// processed once, in the same order — no tx dropped, none double-counted).
    #[test]
    fn tx_shard_bounds_assign_every_txid_once() {
        for n in 1..=4 {
            let b = tx_shard_bounds(n);
            for first in 0u8..=255 {
                let key = [b't', first, 0x00, 0x00]; // a representative 't'+txid key
                let k = key.as_slice();
                let hits = b
                    .iter()
                    .filter(|(lo, hi)| k >= lo.as_slice() && k < hi.as_slice())
                    .count();
                assert_eq!(hits, 1, "first byte {first} matched {hits} shards at n={n}");
            }
        }
    }

    /// Build a CTxOut with the given value/index/script/addresses (the fields
    /// classify_output() reads). script_length is informational and unused here.
    fn mk_out(value: i64, index: u64, script: Vec<u8>, address: Vec<&str>) -> CTxOut {
        CTxOut {
            value,
            script_length: script.len() as i32,
            script_pubkey: CScript { script },
            index,
            address: address.into_iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Canonical 25-byte P2PKH script (0x76a914 <20> 88ac) — get_script_type=="pubkeyhash".
    fn p2pkh_script() -> Vec<u8> {
        let mut s = vec![0x76, 0xa9, 0x14];
        s.extend_from_slice(&[0xAB; 20]);
        s.extend_from_slice(&[0x88, 0xac]);
        s
    }

    /// Canonical 51-byte P2CS cold-stake script — get_script_type=="coldstake".
    fn coldstake_script() -> Vec<u8> {
        let mut s = vec![0u8; 51];
        s[0] = 0x76;
        s[1] = 0xa9;
        s[2] = 0x7b;
        s[3] = 0x63;
        s[4] = 0xd2;
        s[5] = 0x14;
        s[26] = 0x67;
        s[27] = 0x14;
        s[48] = 0x68;
        s[49] = 0x88;
        s[50] = 0xac;
        s
    }

    /// Packing a synthetic tx: out_sum is preserved across EVERY output (incl. the
    /// kind=0 coinstake marker and an OP_RETURN), and each output maps to the
    /// expected kind / interned ids. Mirrors the Pass 1 packing path exactly.
    #[test]
    fn pack_output_kind_mapping_and_out_sum() {
        let outputs = vec![
            // vout 0: empty coinstake marker — value 0, empty script => kind 0.
            mk_out(0, 0, vec![], vec![]),
            // vout 1: P2PKH single address => kind 1.
            mk_out(
                5_0000_0000,
                1,
                p2pkh_script(),
                vec!["DStakerOrPayee1111111111111111111"],
            ),
            // vout 2: P2CS cold stake (staker, owner) => kind 2, two DISTINCT ids.
            mk_out(
                12_0000_0000,
                2,
                coldstake_script(),
                vec![
                    "SStakerColdAddr1111111111111111111",
                    "DOwnerColdAddr1111111111111111111",
                ],
            ),
            // vout 3: OP_RETURN (value 0, script starts with 0x6a) => kind 0.
            mk_out(
                0,
                3,
                vec![0x6a, 0x04, 0xde, 0xad, 0xbe, 0xef],
                vec!["ignored"],
            ),
            // vout 4: another P2PKH for the SAME address as vout 1 => same interned id.
            mk_out(
                3_0000_0000,
                4,
                p2pkh_script(),
                vec!["DStakerOrPayee1111111111111111111"],
            ),
        ];

        let mut addr_intern: HashMap<String, u32> = HashMap::new();
        let mut addr_rev: Vec<String> = Vec::new();
        let packed: Vec<PackedOut> = outputs
            .iter()
            .map(|o| pack_output(o, &mut addr_intern, &mut addr_rev))
            .collect();

        // INVARIANT #1 / #8: every output is packed (incl. markers) and out_sum
        // equals the sum of the raw output values exactly.
        assert_eq!(packed.len(), outputs.len());
        let out_sum: i64 = packed.iter().map(|p| p.value).sum();
        let raw_sum: i64 = outputs.iter().map(|o| o.value).sum();
        assert_eq!(out_sum, raw_sum);
        assert_eq!(out_sum, 5_0000_0000 + 12_0000_0000 + 3_0000_0000);

        // vout positions are copied verbatim from output.index.
        for (i, p) in packed.iter().enumerate() {
            assert_eq!(p.vout as usize, i);
        }

        // Kind mapping.
        assert_eq!(packed[0].kind, 0, "empty coinstake marker => kind 0");
        assert_eq!(packed[0].addr_a, NO_ADDR);
        assert_eq!(packed[0].addr_b, NO_ADDR);

        assert_eq!(packed[1].kind, 1, "P2PKH => kind 1 (single)");
        assert_ne!(packed[1].addr_a, NO_ADDR);
        assert_eq!(packed[1].addr_b, NO_ADDR);

        assert_eq!(packed[2].kind, 2, "P2CS => kind 2 (coldstake dual id)");
        assert_ne!(packed[2].addr_a, NO_ADDR);
        assert_ne!(packed[2].addr_b, NO_ADDR);
        assert_ne!(packed[2].addr_a, packed[2].addr_b, "staker != owner id");

        assert_eq!(packed[3].kind, 0, "OP_RETURN => kind 0");
        assert_eq!(packed[3].addr_a, NO_ADDR);

        assert_eq!(packed[4].kind, 1, "second P2PKH => kind 1");
        // Interning round-trip: identical address strings share one dense id.
        assert_eq!(
            packed[1].addr_a, packed[4].addr_a,
            "same address => same id"
        );
        assert_eq!(
            addr_rev[packed[1].addr_a as usize],
            "DStakerOrPayee1111111111111111111"
        );
    }

    // ---- compute_tx_join: the shared join arithmetic (live == enrich) ----
    use crate::emission::{era_block_reward, era_staker_reward, COIN};
    use crate::tx_type::TransactionType;

    fn base_inputs() -> TxJoinInputs {
        TxJoinInputs {
            height: 100,
            ty: TransactionType::Normal,
            out_sum: 0,
            p2cs_created: 0,
            value_balance: 0,
            value_len: 250,
            n_outputs: 2,
            input_sum: 0,
            inputs_with_prevout: 0,
            inputs_resolved: 0,
            coin_days: 0.0,
            p2cs_spent: 0,
        }
    }

    /// Normal tx, all inputs resolved -> fee credited; normal_tx_bytes = len-8.
    #[test]
    fn join_normal_fee_clamped_and_bytes() {
        let inp = TxJoinInputs {
            input_sum: 10 * COIN,
            out_sum: 9 * COIN, // fee = 1 PIV (+ value_balance 0)
            inputs_with_prevout: 2,
            inputs_resolved: 2,
            value_len: 300,
            ..base_inputs()
        };
        let c = compute_tx_join(&inp, "2024-01-01");
        assert_eq!(c.fees_total, COIN);
        assert_eq!(c.normal_tx_bytes, 300 - 8);
        assert_eq!(c.rewards_total, 0);
        assert!(c.treasury.is_none());
    }

    /// Mixed resolved+unresolved inputs -> fee SUPPRESSED, but coin_days / p2cs /
    /// normal_tx_bytes are UNCONDITIONAL (the v4-review regression guard).
    #[test]
    fn join_normal_unresolved_suppresses_fee_only() {
        let inp = TxJoinInputs {
            input_sum: 10 * COIN,
            out_sum: 9 * COIN,
            inputs_with_prevout: 2,
            inputs_resolved: 1, // not all resolved
            coin_days: 42.5,
            p2cs_spent: 3 * COIN,
            p2cs_created: 7 * COIN,
            value_len: 300,
            ..base_inputs()
        };
        let c = compute_tx_join(&inp, "2024-01-01");
        assert_eq!(
            c.fees_total, 0,
            "fee suppressed when not all inputs resolved"
        );
        assert_eq!(c.coin_days, 42.5);
        assert_eq!(c.p2cs_spent, 3 * COIN);
        assert_eq!(c.p2cs_created, 7 * COIN);
        assert_eq!(c.normal_tx_bytes, 300 - 8);
    }

    /// Out-of-range / non-positive fee is dropped (clamp ceiling + positivity).
    #[test]
    fn join_normal_fee_out_of_range_dropped() {
        // Negative fee (out > in) -> dropped.
        let neg = TxJoinInputs {
            input_sum: 9 * COIN,
            out_sum: 10 * COIN,
            inputs_with_prevout: 1,
            inputs_resolved: 1,
            ..base_inputs()
        };
        assert_eq!(compute_tx_join(&neg, "d").fees_total, 0);
        // Absurd fee (>= 1000 PIV) -> dropped.
        let huge = TxJoinInputs {
            input_sum: 5000 * COIN,
            out_sum: 0,
            inputs_with_prevout: 1,
            inputs_resolved: 1,
            ..base_inputs()
        };
        assert_eq!(compute_tx_join(&huge, "d").fees_total, 0);
    }

    /// Coinstake with small minted (<= expected + 100 PIV) -> rewards = minted,
    /// no treasury payout; normal_tx_bytes stays 0 for coinstake.
    #[test]
    fn join_coinstake_small_minted_no_treasury() {
        let h = 200_000;
        let expected = era_block_reward(h);
        let minted = expected + 50 * COIN; // excess 50 PIV (< 100 threshold)
        let inp = TxJoinInputs {
            height: h,
            ty: TransactionType::Coinstake,
            input_sum: 1000 * COIN,
            out_sum: 1000 * COIN + minted,
            inputs_with_prevout: 1,
            inputs_resolved: 1,
            ..base_inputs()
        };
        let c = compute_tx_join(&inp, "d");
        assert_eq!(c.rewards_total, minted);
        assert!(c.treasury.is_none());
        assert_eq!(
            c.normal_tx_bytes, 0,
            "coinstake never counts normal_tx_bytes"
        );
        assert_eq!(c.staker_rewards_total, era_staker_reward(h).min(minted));
    }

    /// Coinstake with budget payout (excess > 100 PIV) -> treasury = excess,
    /// rewards = expected era reward (not minted).
    #[test]
    fn join_coinstake_budget_payout_split() {
        let h = 200_000;
        let expected = era_block_reward(h);
        let excess = 5000 * COIN; // well over 100 PIV
        let minted = expected + excess;
        let inp = TxJoinInputs {
            height: h,
            ty: TransactionType::Coinstake,
            input_sum: 1000 * COIN,
            out_sum: 1000 * COIN + minted,
            n_outputs: 4,
            inputs_with_prevout: 1,
            inputs_resolved: 1,
            ..base_inputs()
        };
        let c = compute_tx_join(&inp, "2024-02-02");
        assert_eq!(c.rewards_total, expected);
        let t = c.treasury.expect("budget payout present");
        assert_eq!(t.total_paid_sats, excess);
        assert_eq!(t.n_outputs, 4);
        assert_eq!(t.date, "2024-02-02");
        assert_eq!(t.height, h);
    }

    /// zPoS-style coinstake: a null prevout never resolves (inputs_with_prevout=1,
    /// inputs_resolved=0) -> clamp fails -> rewards/staker suppressed, but the
    /// output-only p2cs_created still passes through.
    #[test]
    fn join_coinstake_zpos_suppressed() {
        let inp = TxJoinInputs {
            height: 200_000,
            ty: TransactionType::Coinstake,
            input_sum: 0,
            out_sum: 250 * COIN,
            p2cs_created: 11 * COIN,
            inputs_with_prevout: 1,
            inputs_resolved: 0,
            ..base_inputs()
        };
        let c = compute_tx_join(&inp, "d");
        assert_eq!(c.rewards_total, 0);
        assert_eq!(c.staker_rewards_total, 0);
        assert_eq!(
            c.p2cs_created,
            11 * COIN,
            "output-only field is unconditional"
        );
    }

    /// Coinbase contributes only the unconditional output-derived fields.
    #[test]
    fn join_coinbase_only_unconditional() {
        let inp = TxJoinInputs {
            ty: TransactionType::Coinbase,
            p2cs_created: 2 * COIN,
            value_len: 500,
            ..base_inputs()
        };
        let c = compute_tx_join(&inp, "d");
        assert_eq!(c.fees_total, 0);
        assert_eq!(c.rewards_total, 0);
        assert_eq!(c.normal_tx_bytes, 0);
        assert_eq!(c.p2cs_created, 2 * COIN);
    }
}
