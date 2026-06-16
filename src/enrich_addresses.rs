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

use crate::constants::{should_index_transaction};
use crate::metrics;
/// - Fast incremental updates
/// - Proper UTXO tracking (spent vs unspent)
/// 
/// **Alternative Approach:**
/// See `enrich_from_chainstate.rs` for verification using PIVX Core's chainstate
/// as the authoritative source of truth. That approach is best for one-time
/// verification or recovery but requires PIVX Core to be stopped.

use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use rocksdb::DB;
use tracing::{info, warn, info_span};
use crate::parser::{deserialize_transaction, serialize_utxos};
use crate::tx_keys::{txid_from_key, txid_from_hex};
use crate::types::{CTxOut, ScriptClassification};

/// Classify output script for correct PIVX Core attribution.
/// Classification is driven by the SCRIPT structure (like PIVX Core's Solver()),
/// not by guessing from base58 prefixes of the extracted address strings.
fn classify_output(output: &CTxOut) -> ScriptClassification {
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
    if output.value == 0 && (
        output.script_pubkey.script.is_empty() ||
        output.script_pubkey.script[0] == 0x6a
    ) {
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

/// Encode a detected transaction type as the PackedTx.ty u8.
/// 1 == Coinstake is load-bearing: Pass 2 / Pass 2b test `ty == 1` for coinstake.
fn ty_to_u8(t: crate::tx_type::TransactionType) -> u8 {
    match t {
        crate::tx_type::TransactionType::Normal => 0,
        crate::tx_type::TransactionType::Coinstake => 1,
        crate::tx_type::TransactionType::Coinbase => 2,
    }
}

/// Decode a PackedTx.ty u8 back to the transaction type (inverse of `ty_to_u8`).
fn u8_to_ty(b: u8) -> crate::tx_type::TransactionType {
    match b {
        1 => crate::tx_type::TransactionType::Coinstake,
        2 => crate::tx_type::TransactionType::Coinbase,
        _ => crate::tx_type::TransactionType::Normal,
    }
}

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
        value: output.value,       // INVARIANT #1: exact, verbatim i64
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

/// Build address index from all transactions
/// This creates the addr_index CF entries for address lookups
pub async fn enrich_all_addresses(db: Arc<DB>) -> Result<(), Box<dyn std::error::Error>> {
    let _span = info_span!("enrich_all_addresses").entered();
    info!("Building address index from transactions");

    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    let cf_addr_index = db.cf_handle("addr_index")
        .ok_or("addr_index CF not found")?;
    let cf_state = db.cf_handle("chain_state")
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

    let mut processed = 0;
    let mut indexed_outputs = 0;
    let batch_size = 10000;
    
    info!("Pass 1: Building complete spent outputs set");
    
    // PASS 1: Build complete spent outputs set by scanning ALL transaction inputs
    // O1 OPTIMIZATION: Build transaction cache to avoid repeated deserialization
    // Packed spent-outpoint set: ([u8;32] txid, u32 vout) inline keys — no
    // per-entry heap allocation, ~half the RAM and faster hashing than the old
    // (Vec<u8>, u64). Keys stay in display byte order (matching Pass 1).
    let mut spent_outputs: HashSet<([u8; 32], u32)> = HashSet::new();

    // ONE packed store replacing tx_cache / tx_heights / the per-tx deserialized
    // objects (~13-18GB saved). Pass 2 / Pass 2b / HODL primitive directly off
    // these and NEVER re-deserialize.
    //   tx_index : DISPLAY-order 32-byte txid -> dense slot
    //   packed   : slot -> PackedTx (height, ty, value_balance, packed outs)
    //   tx_rev   : slot -> DISPLAY-order 32-byte txid (for UTXO/txs_map serialize)
    let mut tx_index: HashMap<[u8; 32], u32> = HashMap::new();
    let mut packed: Vec<PackedTx> = Vec::new();
    let mut tx_rev: Vec<[u8; 32]> = Vec::new();

    // Address interning (Step 3) is hoisted ABOVE Pass 1 because Pass 1 now
    // classifies + interns every output while building the packed store. Intern
    // each distinct base58 string ONCE to a dense u32 id; resolve id->String only
    // when building on-disk key bytes, so the output stays byte-identical.
    // addr_rev[id] is the exact string; addr_intern[s] is its id.
    let mut addr_intern: HashMap<String, u32> = HashMap::new();
    let mut addr_rev: Vec<String> = Vec::new();
    // (intern() and pack_output() are free fns above — they borrow both maps
    // mutably at each call site.)

    // Phase 2 Instrumentation: Track deserialization metrics
    let mut pass1_tx_total = 0;
    let mut pass1_tx_deserialized = 0;
    let mut pass1_tx_failed = 0;
    let mut pass1_inputs_processed = 0;
    let mut pass1_sapling_count = 0;  // NEW: Track Sapling transactions
    
    let iter1 = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    
    for item in iter1 {
        let (key, value) = item?;
        // Skip block transaction index keys
        if key.first() == Some(&b'B') {
            continue;
        }
        // Skip invalid transactions
        if value.len() < 8 {
            continue;
        }
        // Check height: skip orphaned and unresolved transactions
        let height_bytes: [u8; 4] = value[4..8].try_into().unwrap_or([0,0,0,0]);
        let height = i32::from_le_bytes(height_bytes);
        if !should_index_transaction(height) {
            continue;
        }
        let raw_tx = &value[8..];
        let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
        tx_with_header.extend_from_slice(&[0u8; 4]);
        tx_with_header.extend_from_slice(raw_tx);
        
        pass1_tx_total += 1;
        
        let tx = match deserialize_transaction(&tx_with_header).await {
            Ok(tx) => {
                pass1_tx_deserialized += 1;
                // NEW: Count Sapling transactions (version >= 3 with Sapling data)
                if tx.sapling_data.is_some() {
                    pass1_sapling_count += 1;
                }
                tx
            }
            Err(e) => {
                pass1_tx_failed += 1;
                // CRITICAL: Log deserialization failures
                let txid_bytes = txid_from_key(&key);
                let txid_hex = hex::encode(&txid_bytes);
                warn!(txid = %txid_hex, height = height, error = ?e, "Pass 1: Failed to deserialize transaction");
                continue;
            }
        };

        // INVARIANT #2: detect the type NOW, while the inputs are still alive —
        // it depends on inputs[0]'s null/zerocoin prevout, which is dropped below.
        let ty = ty_to_u8(crate::tx_type::detect_transaction_type(&tx));
        // INVARIANT #4: carry the SIGNED Sapling net value forward for the fee calc.
        let value_balance = tx.sapling_data.as_ref().map(|s| s.value_balance).unwrap_or(0);

        // Build the packed outputs by running the SAME classify_output logic Pass 2
        // used to run, interning the resulting address string(s) to u32 ONCE here.
        // PUSH EVERY output (incl. kind=0 markers / zerocoin / nonstandard) so
        // outs.len() == tx.outputs.len() and out_sum is identical (INVARIANT #1, #8).
        let mut outs: Vec<PackedOut> = Vec::with_capacity(tx.outputs.len());
        for output in &tx.outputs {
            outs.push(pack_output(output, &mut addr_intern, &mut addr_rev));
        }

        // Insert the packed tx under its DISPLAY-order txid (same source the old
        // tx_cache key used: txid_from_key on the CF key). INVARIANT #7.
        let txid_bytes = txid_from_key(&key);
        if !txid_bytes.is_empty() {
            if let Ok(txid_arr) = <[u8; 32]>::try_from(txid_bytes.as_slice()) {
                let slot = packed.len() as u32;
                tx_index.insert(txid_arr, slot);
                tx_rev.push(txid_arr);
                packed.push(PackedTx {
                    height,
                    ty,
                    value_balance,
                    outs: outs.into_boxed_slice(),
                });
            }
        }

        // Scan inputs into the spent-outpoint set (UNCHANGED), then DROP the tx.
        for input in &tx.inputs {
                if input.coinbase.is_some() {
                    continue;
                }
                pass1_inputs_processed += 1;
                if let Some(prevout) = &input.prevout {
                    // FIXED: parser.rs now returns prevout.hash in DISPLAY format (reversed)
                    // This matches the format used in database keys ('t' + reversed_txid)
                    // and matches blocks.rs::read_outpoint() and transactions.rs::read_outpoint()
                    if let Ok(prev_txid_display) = txid_from_hex(&prevout.hash) {
                        // prev_txid_display is in display/reversed format
                        // This NOW matches the database key format

                        let t: [u8; 32] = prev_txid_display.as_slice().try_into()
                            .expect("spent-set prevout txid must be 32 bytes");
                        spent_outputs.insert((t, prevout.n as u32));
                    }
                }
        }
        processed += 1;
    }
    
    
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
                        indexed_outputs += 2;  // Count both
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
        warn!(pass1_total = pass1_tx_total, pass2_total = pass2_tx_total, 
              diff = (pass1_tx_total as i64 - pass2_tx_total as i64).abs(), 
              "Pass divergence: Transaction count mismatch");
    }
    if pass1_tx_failed != pass2_tx_failed {
        warn!(pass1_failed = pass1_tx_failed, pass2_failed = pass2_tx_failed,
              diff = (pass1_tx_failed as i64 - pass2_tx_failed as i64).abs(),
              "Asymmetric failures between passes - will cause balance errors");
    }
    
    info!(unique_addresses = address_map.len(), spent_outputs = spent_outputs.len(), 
          "Writing address index to database");
    info!("Pass 2b: Scanning inputs to include sent transactions and calculate totals");
    
    // Phase 2 Instrumentation: Track Pass 2b metrics. Prevout joins now resolve
    // through tx_index -> packed (no prevout re-deserialize, no tx_cache); the
    // current tx is still deserialized once for its INPUTS (not packed), while
    // its outs / ty / value_balance are read from packed[slot].
    let mut pass2b_tx_total = 0;
    let mut pass2b_tx_deserialized = 0;
    let mut pass2b_tx_failed = 0;
    let mut pass2b_coinstake_skipped = 0;
    let mut pass2b_inputs_processed = 0;
    let mut pass2b_prevout_resolved: u64 = 0;

    // Tier-2 per-day aggregates from the prevout joins below (fees, staking
    // rewards, coin days destroyed, cold-staking flows), bucketed by the
    // SPENDER's block date and merged into the daily series afterwards.
    let mut day_joins: HashMap<String, DayJoinAgg> = HashMap::new();
    // Tier-4: budget payouts minted INSIDE coinstakes (modern PIVX pays
    // proposals in the coinstake at heights at/after each 43200-block cycle
    // boundary). Detected here, where outputs - inputs is already joined,
    // as the minted excess over the era's scheduled block reward.
    let mut coinstake_treasury: Vec<TreasuryPayout> = Vec::new();

    let iter3 = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    let mut input_processed: usize = 0;
    for item in iter3 {
        let (key, value) = item?;
        if key.first() == Some(&b'B') { continue; }
        if value.len() < 8 { continue; }
        let height_bytes: [u8; 4] = value[4..8].try_into().unwrap_or([0,0,0,0]);
        let height = i32::from_le_bytes(height_bytes);
        if !should_index_transaction(height) { continue; }

        // Extract current txid from key
        let current_txid_bytes = txid_from_key(&key);
        if current_txid_bytes.is_empty() { continue; }

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

        pass2b_tx_total += 1;

        // Deserialize the current tx ONCE for its INPUTS (not held in packed).
        let raw_tx = &value[8..];
        let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
        tx_with_header.extend_from_slice(&[0u8; 4]);
        tx_with_header.extend_from_slice(raw_tx);
        let tx = match deserialize_transaction(&tx_with_header).await {
            Ok(tx) => {
                pass2b_tx_deserialized += 1;
                tx
            }
            Err(e) => {
                pass2b_tx_failed += 1;
                let txid_hex = hex::encode(&current_txid_bytes);
                warn!(txid = %txid_hex, height = height, error = ?e, "Pass 2b: Failed to deserialize transaction");
                continue;
            }
        };

        // Coinstake inputs ARE counted as "sent" (UTXO accounting, Blockbook parity):
        // the staked output is consumed and its principal re-minted in the coinstake
        // outputs (counted as received). This keeps balance == received - sent.
        // (ty was computed in Pass 1; ty==1 => Coinstake.)
        if packed[cur_slot as usize].ty == 1 {
            pass2b_coinstake_skipped += 1; // metric retained: counts coinstakes seen
        }

        // Tier-2 accumulators for this spending transaction (prevout joins).
        let mut input_sum: i64 = 0;
        let mut inputs_with_prevout: u64 = 0;
        let mut inputs_resolved: u64 = 0;
        let mut tx_coin_days: f64 = 0.0;
        let mut tx_p2cs_spent: i64 = 0;

        // For every input, find the prevout's addresses and attribute this tx to them
        for input in &tx.inputs {
            if input.coinbase.is_some() { continue; }
            pass2b_inputs_processed += 1;
            if let Some(prevout) = &input.prevout {
                inputs_with_prevout += 1;
                // prevout.hash from parser.rs is in DISPLAY (reversed) format — the
                // same byte order as tx_index keys / tx_rev (INVARIANT #7).
                if let Ok(prev_txid_display) = txid_from_hex(&prevout.hash) {
                    // INVARIANT #3 (zPoS null-prevout): resolve ONLY through tx_index.
                    // A zPoS coinstake's first input carries an all-zero/zerocoin
                    // prevout txid that was never inserted into tx_index, so the
                    // lookup MISSES, inputs_resolved is NOT incremented (while
                    // inputs_with_prevout IS), and the clamp
                    // `inputs_with_prevout>0 && inputs_resolved==inputs_with_prevout`
                    // correctly FAILS -> fee/minted suppressed for zPoS. The
                    // all-zero txid cannot map to a real slot: it is never inserted
                    // (only real CF-key txids are), so .get() returns None.
                    let prev = <[u8; 32]>::try_from(prev_txid_display.as_slice())
                        .ok()
                        .and_then(|a| tx_index.get(&a).copied())
                        .map(|s| {
                            let p = &packed[s as usize];
                            (p, p.height)
                        });

                    if let Some((prev_tx, prev_height)) = prev {
                        if let Some(prev_out) = prev_tx.outs.get(prevout.n as usize) {
                            // Tier 2: spend-side joins — input value sum (fees /
                            // rewards), coin age, and cold-staking principal spent.
                            inputs_resolved += 1;
                            pass2b_prevout_resolved += 1;
                            input_sum = input_sum.saturating_add(prev_out.value);
                            if prev_height >= 0 && height >= prev_height {
                                let age_days = (height - prev_height) as f64 / 1440.0;
                                tx_coin_days += (prev_out.value as f64 / 100_000_000.0) * age_days;
                            }
                            // coldstake-spent test <= (prev_out.kind == 2), which
                            // coincides exactly with the old get_script_type=="coldstake".
                            if prev_out.kind == 2 {
                                tx_p2cs_spent = tx_p2cs_spent.saturating_add(prev_out.value);
                            }

                            // Attribution from the packed kind / interned ids
                            // (already classified + interned in Pass 1).
                            match prev_out.kind {
                                1 => {
                                    // Standard: address is spending
                                    let id = prev_out.addr_a;
                                    *totals_sent.entry(id).or_insert(0) += prev_out.value;
                                    txs_map.entry(id).or_default().push(current_txid_bytes.clone());
                                }

                                2 => {
                                    // Cold stake spend: debit BOTH addresses, mirroring the
                                    // credit both received in Pass 2 (see Pass 2 comment).
                                    // This keeps balance == received - sent for the staker
                                    // and the owner independently. Staker and owner are
                                    // DISTINCT ids.
                                    let staker_id = prev_out.addr_a;
                                    let owner_id = prev_out.addr_b;
                                    *totals_sent.entry(staker_id).or_insert(0) += prev_out.value;
                                    *totals_sent.entry(owner_id).or_insert(0) += prev_out.value;

                                    // Both appear in transaction list
                                    txs_map.entry(staker_id).or_default().push(current_txid_bytes.clone());
                                    txs_map.entry(owner_id).or_default().push(current_txid_bytes.clone());
                                }

                                _ => {
                                    // No attribution for nonstandard/OP_RETURN/etc
                                }
                            }
                        }
                    }
                }
            }
        }

        // Tier 2: bucket this tx's join aggregates by the SPENDER's block date.
        if height >= 0 && (height as usize) < block_times.len() && block_times[height as usize] != 0 {
            let date = unix_to_date(block_times[height as usize] as u64);
            let agg = day_joins.entry(date.clone()).or_default();
            // out_sum / p2cs_created from the CURRENT tx's packed outs.
            let cur = &packed[cur_slot as usize];
            let out_sum: i64 = cur.outs.iter().map(|o| o.value).sum();
            let p2cs_created: i64 = cur
                .outs
                .iter()
                .filter(|o| o.kind == 2)
                .map(|o| o.value)
                .sum();
            agg.p2cs_created = agg.p2cs_created.saturating_add(p2cs_created);
            agg.p2cs_spent = agg.p2cs_spent.saturating_add(tx_p2cs_spent);
            agg.coin_days_destroyed += tx_coin_days;
            match u8_to_ty(cur.ty) {
                crate::tx_type::TransactionType::Normal => {
                    agg.normal_tx_bytes += (value.len() as u64).saturating_sub(8);
                    // Fee = transparent_in + valueBalance - transparent_out.
                    // Sapling txs move value via valueBalance (negative =
                    // transparent->shield, positive = shield->transparent);
                    // ignoring it booked the entire shielded amount as "fee"
                    // (a t->z tx reported ~1998 PIV vs a real ~0.5). Credit only
                    // when every transparent input resolved, and clamp to a sane
                    // ceiling to reject any residual mis-join. INVARIANT #4:
                    // value_balance is SIGNED and added with its sign.
                    if inputs_with_prevout > 0 && inputs_resolved == inputs_with_prevout {
                        let value_balance = cur.value_balance;
                        let fee = input_sum + value_balance - out_sum;
                        if fee > 0 && fee < 1_000 * crate::emission::COIN {
                            agg.fees_total = agg.fees_total.saturating_add(fee);
                        }
                    }
                }
                crate::tx_type::TransactionType::Coinstake => {
                    // Minted value: outputs - staked inputs. This is the era
                    // block reward PLUS any budget payout riding in the
                    // coinstake (PIVX pays treasury proposals inside the
                    // coinstake at heights at/after each budget cycle, e.g.
                    // 12,200 PIV at h=5,400,000 and 100,800 PIV at
                    // h=5,443,200 — node-verified, see src/emission.rs).
                    if inputs_resolved == inputs_with_prevout {
                        let minted = out_sum.saturating_sub(input_sum);
                        if minted > 0 {
                            let expected = crate::emission::era_block_reward(height);
                            let excess = minted.saturating_sub(expected);
                            // Budget payouts are large (hundreds–thousands of
                            // PIV). A coinstake whose excess is only a few PIV is
                            // collecting transaction fees, not paying a proposal —
                            // the old >1 PIV threshold swept ~19 fee-bearing
                            // coinstakes into the treasury history. Require
                            // >=100 PIV; fee excess then correctly flows into the
                            // staker's rewards (the else branch).
                            if excess > 100 * crate::emission::COIN {
                                // Budget payout: record it as treasury and
                                // keep ONLY the era emission in rewards_total
                                // so the staking APY series isn't polluted by
                                // superblocks. (>1 PIV tolerance for fees /
                                // rounding.)
                                coinstake_treasury.push(TreasuryPayout {
                                    height,
                                    date: date.clone(),
                                    total_paid_sats: excess,
                                    n_outputs: cur.outs.len() as u32,
                                });
                                agg.rewards_total =
                                    agg.rewards_total.saturating_add(expected);
                            } else {
                                agg.rewards_total =
                                    agg.rewards_total.saturating_add(minted);
                            }
                            // Staker share (excl. masternode payment) per the
                            // v5.6.1 schedule, capped by what was minted.
                            agg.staker_rewards_total = agg.staker_rewards_total.saturating_add(
                                crate::emission::era_staker_reward(height).min(minted),
                            );
                        }
                    }
                }
                crate::tx_type::TransactionType::Coinbase => {}
            }
        }

        input_processed += 1;
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
    debug_assert_eq!(addr_intern.len(), addr_rev.len(), "intern/rev size mismatch");
    if let Some((s, &id)) = addr_intern.iter().next() {
        debug_assert_eq!(&addr_rev[id as usize], s, "intern round-trip mismatch");
    }

    // CRITICAL: Final divergence check across all passes
    if pass1_tx_total != pass2_tx_total || pass2_tx_total != pass2b_tx_total {
        warn!(pass1_total = pass1_tx_total, pass2_total = pass2_tx_total, pass2b_total = pass2b_tx_total,
              "TX count mismatch across passes");
    }
    
    if pass1_tx_failed > 0 || pass2_tx_failed > 0 || pass2b_tx_failed > 0 {
        if pass1_tx_failed != pass2_tx_failed || pass2_tx_failed != pass2b_tx_failed {
            warn!(pass1_failed = pass1_tx_failed, pass2_failed = pass2_tx_failed, pass2b_failed = pass2b_tx_failed,
                  "Asymmetric deserialization failures - will cause balance errors");
        } else {
            info!(failed = pass1_tx_failed, "Deserialization failures (consistent across passes)");
        }
    }
    
    info!(unique_addresses = address_map.len(), "Writing address index to database");
    
    // Write address mappings to database
    let mut batch = rocksdb::WriteBatch::default();
    let mut written = 0;
    let total_addresses = address_map.len();  // Cache length before consuming map
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

        // Build canonical UTXO list (only unspent entries) to match serialize_utxos format
        let mut utxos_unspent: Vec<(Vec<u8>, u64)> = Vec::new();

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
                // serialize_utxos expects (txid Vec<u8>, vout u64); resolve here so
                // the on-disk format is byte-identical (txid(32) + vout u64 LE).
                utxos_unspent.push((txid32.to_vec(), *vout as u64));

                // HODL: bucket this UTXO's value by coin age, counting each
                // outpoint exactly once (pure in-memory lookups; no DB reads).
                if tip > 0 && hodl_seen.insert((txid32, *vout)) {
                    let ptx = &packed[*slot as usize];
                    let h = ptx.height;
                    if h >= 0 && h <= tip {
                        // Parser sets output.index == position, so direct indexing
                        // by vout is exact. Value comes from packed[slot].outs[vout]
                        // — identical to the original tx.outputs[vout].value.
                        if let Some(out) = ptx.outs.get(*vout as usize) {
                            let band_idx = hodl_band_index(tip, h);
                            hodl_sums[band_idx] =
                                hodl_sums[band_idx].saturating_add(out.value);
                            hodl_total = hodl_total.saturating_add(out.value);
                        }
                    }
                }
            }
        }

        // Serialize UTXOs in canonical format used by the API (txid(32) + vout(u64) per entry)
        let serialized_utxos = serialize_utxos(&utxos_unspent).await;
        batch.put_cf(&cf_addr_index, &key, &serialized_utxos);

        // Get pre-calculated totals from Pass 2 and 2b (MUCH faster than recalculating!)
        let total_received = *totals_received.get(&id).unwrap_or(&0);
        let total_sent = *totals_sent.get(&id).unwrap_or(&0);

        // Write transaction list ('t' + address)
        if let Some(txids) = txs_map.get(&id) {
            let mut unique_txids = txids.clone();
            unique_txids.sort();
            unique_txids.dedup();
            
            // Serialize transaction list
            let mut txs_serialized: Vec<u8> = Vec::with_capacity(unique_txids.len() * 32);
            for txid in unique_txids {
                txs_serialized.extend_from_slice(&txid);
            }
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
    metrics::set_sapling_transactions_count(pass1_sapling_count as u64);
    metrics::increment_sapling_transactions(pass1_sapling_count as u64);
    
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
    if let Err(e) = persist_wealth_analytics(&db, &totals_received, &totals_sent, &txs_map, &addr_rev) {
        warn!(error = %e, "Failed to persist wealth analytics");
    }

    // HODL / dormancy snapshot: value of the final unspent UTXO set bucketed
    // by coin age, accumulated above from the same deduped unspent sets that
    // back the 'a' entries (the balance path verified against the reference
    // explorer — see the comment at the accumulators).
    if let Err(e) = persist_hodl_snapshot(&db, &hodl_sums, hodl_total, tip) {
        warn!(error = %e, "Failed to persist HODL snapshot");
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
    let db_bg = Arc::clone(&db);
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
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
            if let Some(cf_state) = db_bg.cf_handle("chain_state") {
                let _ = db_bg.put_cf(&cf_state, b"analytics_complete", [1u8]);
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
fn nbits_to_difficulty(nbits: u32) -> f64 {
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
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Build a height -> block nTime index by reading every canonical block header.
fn build_block_times(db: &Arc<DB>, tip: i32) -> Result<(Vec<u32>, Vec<u32>), Box<dyn std::error::Error>> {
    let cf_metadata = db.cf_handle("chain_metadata").ok_or("chain_metadata CF not found")?;
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

async fn persist_tx_daily_series(
    db: &Arc<DB>,
    tip: i32,
    block_times: &[u32],
    block_bits: &[u32],
    day_joins: &HashMap<String, DayJoinAgg>,
    coinstake_treasury: &[TreasuryPayout],
) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db.cf_handle("chain_state").ok_or("chain_state CF not found")?;
    let cf_transactions = db.cf_handle("transactions").ok_or("transactions CF not found")?;

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
        let agg = days.entry(agg_date.clone()).or_default();
        agg.tx_count += 1;
        agg.tx_bytes += raw_tx.len() as u64;
        if tx.sapling_data.is_some() {
            agg.sapling_txs += 1;
        }
        let tx_type = crate::tx_type::detect_transaction_type(&tx);
        match tx_type {
            crate::tx_type::TransactionType::Coinbase => {
                agg.coinbase += 1;
                // Tier 4: PoW-era budget payouts ride in the coinbase as
                // value minted in excess of the era's scheduled block reward
                // (>1 PIV tolerance for tx fees). Node-verified: h=86,400
                // coinbase paid 1,000,250 = 250 era + 1,000,000 budget;
                // h=129,600 paid 325 = 225 era + 100 budget. PoS-era budget
                // payouts ride in COINSTAKES and are detected in Pass 2b.
                if height > 0 {
                    let total: i64 = tx.outputs.iter().map(|o| o.value).sum();
                    let excess =
                        total.saturating_sub(crate::emission::era_block_reward(height));
                    // Require >=10 PIV: excludes sub-PIV tx-fee noise (which the
                    // old >1 PIV bar let through) while keeping every legit
                    // PoW-era coinbase budget (smallest node-verified is 100 PIV
                    // at h=129,600). Budgets here are 100 PIV..1M PIV; fees are
                    // well under 10 PIV.
                    if excess > 10 * crate::emission::COIN {
                        treasury.push(TreasuryPayout {
                            height,
                            date: agg_date.clone(),
                            total_paid_sats: excess,
                            n_outputs: tx.outputs.len() as u32,
                        });
                    }
                }
            }
            crate::tx_type::TransactionType::Coinstake => {
                agg.coinstake += 1;
                // Cold-staked coinstake: a delegated (P2CS) stake re-mints its
                // principal back to the same P2CS script, so any coldstake
                // output marks the stake as cold staking (same script test as
                // the Pass 2b p2cs_created join).
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
                // First address of the first paying output identifies the staker
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
            crate::tx_type::TransactionType::Normal => {
                agg.payment += 1;
                agg.volume = agg
                    .volume
                    .saturating_add(tx.outputs.iter().map(|o| o.value).sum());
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
    // Stale blocks: every stored header whose hash is NOT in the canonical
    // 'h' index, bucketed by its own header time.
    {
        let cf_blocks = db.cf_handle("blocks").ok_or("blocks CF not found")?;
        let cf_metadata = db.cf_handle("chain_metadata").ok_or("chain_metadata CF not found")?;
        let iter = db.iterator_cf(&cf_blocks, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, header) = item?;
            let hash: &[u8] = if key.len() == 33 && key[0] == b'b' { &key[1..] } else { &key[..] };
            if hash.len() != 32 || header.len() < 72 {
                continue;
            }
            let mut h_key = vec![b'h'];
            h_key.extend_from_slice(hash);
            if db.get_cf(&cf_metadata, &h_key)?.is_some() {
                continue; // canonical
            }
            let t = u32::from_le_bytes(header[68..72].try_into().unwrap_or([0; 4]));
            if t == 0 { continue; }
            // Blocks within 2h of the canonical tip may simply postdate the
            // canonical index snapshot — not classifiable as stale yet.
            let tip_time = block_times[tip as usize];
            if tip_time > 0 && t + 7_200 > tip_time {
                continue;
            }
            days.entry(unix_to_date(t as u64)).or_default().orphan_blocks += 1;
        }
    }

    for (date, (diff_sum, blocks)) in &day_diff {
        if let Some(agg) = days.get_mut(date) {
            agg.avg_difficulty = if *blocks > 0 { diff_sum / *blocks as f64 } else { 0.0 };
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
    batch.put_cf(&cf_state, b"analytics_treasury", bincode::serialize(&treasury)?);
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
const RICHLIST_KEEP: usize = 1000;

fn persist_wealth_analytics(
    db: &Arc<DB>,
    totals_received: &HashMap<u32, i64>,
    totals_sent: &HashMap<u32, i64>,
    txs_map: &HashMap<u32, Vec<Vec<u8>>>,
    addr_rev: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db.cf_handle("chain_state").ok_or("chain_state CF not found")?;

    // Compute balance per address (by interned id); keep only positive balances.
    let mut balances: Vec<(u32, i64)> = Vec::with_capacity(totals_received.len());
    let mut total_balance: i64 = 0;
    for (&id, recv) in totals_received {
        let sent = *totals_sent.get(&id).unwrap_or(&0);
        let bal = recv - sent;
        if bal > 0 {
            balances.push((id, bal));
            total_balance += bal;
        }
    }

    // Sort descending by balance for both the rich list and the top-N sums.
    balances.sort_by(|a, b| b.1.cmp(&a.1));

    let richlist: Vec<RichListSnapshotEntry> = balances
        .iter()
        .take(RICHLIST_KEEP)
        .map(|(id, bal)| {
            // Unique tx count (txs_map may list a txid twice when an address is
            // both an input and an output of the same transaction). Deduping
            // only the kept top-N keeps this cheap.
            let tx_count = txs_map
                .get(id)
                .map(|v| {
                    let mut t = v.clone();
                    t.sort();
                    t.dedup();
                    t.len() as u64
                })
                .unwrap_or(0);
            RichListSnapshotEntry {
                // Resolve the interned id back to its exact base58 string only here.
                address: addr_rev[*id as usize].clone(),
                balance: *bal,
                tx_count,
            }
        })
        .collect();

    let sum_take = |n: usize| balances.iter().take(n).map(|(_, b)| *b).sum::<i64>();
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
            let count = balances.iter().filter(|(_, b)| *b >= *min && *b < *max).count() as u64;
            (label.to_string(), count)
        })
        .collect();

    // Gini coefficient (standard formula over ascending balances):
    //   G = 2 * Σ(i * x_i) / (n * Σx) - (n + 1) / n
    // balances is sorted DESCENDING, so iterate in reverse for ascending order.
    let n = balances.len();
    let gini = if n > 0 && total_balance > 0 {
        let mut weighted = 0.0f64;
        for (i, (_, b)) in balances.iter().rev().enumerate() {
            weighted += (i as f64 + 1.0) * (*b as f64);
        }
        (2.0 * weighted) / (n as f64 * total_balance as f64) - (n as f64 + 1.0) / n as f64
    } else {
        0.0
    };

    // Nakamoto coefficient: minimum holders summing to >50% of total balance.
    let mut nakamoto_coefficient: u32 = 0;
    let mut acc: i64 = 0;
    for (_, b) in &balances {
        acc = acc.saturating_add(*b);
        nakamoto_coefficient += 1;
        if (acc as f64) > total_balance as f64 / 2.0 {
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

    db.put_cf(&cf_state, b"analytics_richlist", bincode::serialize(&richlist)?)?;
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
    let cf_state = db.cf_handle("chain_state").ok_or("chain_state CF not found")?;

    let snapshot = HodlSnapshot {
        bands: HODL_BANDS
            .iter()
            .zip(sums.iter())
            .map(|((label, _, _), v)| (label.to_string(), *v))
            .collect(),
        total,
    };
    db.put_cf(&cf_state, b"analytics_hodl", bincode::serialize(&snapshot)?)?;
    info!(total_sats = total, "HODL age-band snapshot precomputed and stored");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CScript;

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
            mk_out(5_0000_0000, 1, p2pkh_script(), vec!["DStakerOrPayee1111111111111111111"]),
            // vout 2: P2CS cold stake (staker, owner) => kind 2, two DISTINCT ids.
            mk_out(
                12_0000_0000,
                2,
                coldstake_script(),
                vec!["SStakerColdAddr1111111111111111111", "DOwnerColdAddr1111111111111111111"],
            ),
            // vout 3: OP_RETURN (value 0, script starts with 0x6a) => kind 0.
            mk_out(0, 3, vec![0x6a, 0x04, 0xde, 0xad, 0xbe, 0xef], vec!["ignored"]),
            // vout 4: another P2PKH for the SAME address as vout 1 => same interned id.
            mk_out(3_0000_0000, 4, p2pkh_script(), vec!["DStakerOrPayee1111111111111111111"]),
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
        assert_eq!(packed[1].addr_a, packed[4].addr_a, "same address => same id");
        assert_eq!(addr_rev[packed[1].addr_a as usize], "DStakerOrPayee1111111111111111111");
    }

    /// ty_to_u8 / u8_to_ty round-trip, with 1 == Coinstake load-bearing for the
    /// Pass 2 / Pass 2b coinstake test.
    #[test]
    fn ty_u8_roundtrip() {
        use crate::tx_type::TransactionType::*;
        assert_eq!(ty_to_u8(Coinstake), 1);
        for t in [Normal, Coinstake, Coinbase] {
            assert_eq!(u8_to_ty(ty_to_u8(t)), t);
        }
    }
}

