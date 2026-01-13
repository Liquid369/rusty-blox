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
use tracing::{info, warn, error, info_span};
use crate::parser::{deserialize_transaction, serialize_utxos};
use crate::tx_keys::{tx_cf_key, txid_from_key, txid_from_hex};
use crate::types::{CTransaction, CTxOut, ScriptClassification};

/// Detect coinstake transaction (PIVX Core parity)
/// Coinstake has: vin[0]=stake input, vout[0]=empty OP_RETURN marker, vout[1+]=rewards
fn is_coinstake(tx: &CTransaction) -> bool {
    !tx.inputs.is_empty() &&
    tx.outputs.len() >= 2 &&
    tx.outputs[0].value == 0 &&
    !tx.outputs[0].script_pubkey.script.is_empty() &&
    tx.outputs[0].script_pubkey.script[0] == 0x6a  // OP_RETURN
}

/// Classify output script for correct PIVX Core attribution
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
    
    // Cold staking: TWO addresses (staker + owner)
    if output.address.len() == 2 {
        // Check if this is from a Staking address type
        // Pattern: first is S-address (staker), second is D-address (owner)
        let staker = &output.address[0];
        let owner = &output.address[1];
        
        // S-addresses start with 'S', D-addresses start with 'D'
        if staker.starts_with('S') && owner.starts_with('D') {
            return ScriptClassification::ColdStake {
                staker: staker.clone(),
                owner: owner.clone(),
            };
        }
    }
    
    // Standard single-address outputs
    if output.address.len() == 1 {
        let addr = &output.address[0];
        // Determine type based on prefix (P2PKH='D', P2SH='s', etc.)
        if addr.starts_with('D') {
            return ScriptClassification::P2PKH(addr.clone());
        } else if addr.starts_with('s') {
            return ScriptClassification::P2SH(addr.clone());
        } else {
            return ScriptClassification::P2PK(addr.clone());
        }
    }
    
    ScriptClassification::Nonstandard
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

    let mut processed = 0;
    let mut indexed_outputs = 0;
    let batch_size = 10000;
    
    info!("Pass 1: Building complete spent outputs set");
    
    // PASS 1: Build complete spent outputs set by scanning ALL transaction inputs
    // O1 OPTIMIZATION: Build transaction cache to avoid repeated deserialization
    let mut spent_outputs: HashSet<(Vec<u8>, u64)> = HashSet::new();
    let mut tx_cache: HashMap<Vec<u8>, Arc<CTransaction>> = HashMap::new();
    
    // Phase 2 Instrumentation: Track deserialization metrics
    let mut pass1_tx_total = 0;
    let mut pass1_tx_deserialized = 0;
    let mut pass1_tx_failed = 0;
    let mut pass1_inputs_processed = 0;
    
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
                // O1: Extract txid and cache the transaction
                let txid_bytes = txid_from_key(&key);
                if !txid_bytes.is_empty() {
                    tx_cache.insert(txid_bytes, Arc::new(tx.clone()));
                }
                Arc::new(tx)
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
                        
                        spent_outputs.insert((prev_txid_display, prevout.n as u64));
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
        cache_entries = tx_cache.len(),
        cache_size_mb = (tx_cache.len() as f64 * 0.5) / 1000.0,
        "Pass 1 complete: Spent outputs set built"
    );
    
    // Store for comparison in Pass 2
    let debug_txid_1 = spent_outputs.iter().take(1).next()
        .map(|(txid, _)| hex::encode(txid))
        .unwrap_or_default();
    
    info!("Pass 2: Indexing outputs with spent flags");
    
    // Reset counter for pass 2
    processed = 0;
    
    // PASS 2: Build address map with spent flags (outputs -> address_map)
    let mut address_map: HashMap<String, Vec<(Vec<u8>, u64)>> = HashMap::new();
    // Also maintain a txs_map to collect all txids involving an address (received OR sent)
    let mut txs_map: HashMap<String, Vec<Vec<u8>>> = HashMap::new();
    // NEW: Track total received and sent per address during Pass 2 (much faster!)
    let mut totals_received: HashMap<String, i64> = HashMap::new();
    let mut totals_sent: HashMap<String, i64> = HashMap::new();
    
    // O1: Track cache hit rate
    let mut cache_hits = 0;
    let mut cache_misses = 0;
    
    // Phase 2 Instrumentation: Track Pass 2 metrics
    let mut pass2_tx_total = 0;
    let mut pass2_tx_deserialized = 0;
    let mut pass2_tx_failed = 0;
    let mut pass2_outputs_processed = 0;
    
    let iter2 = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    for item in iter2 {
        let (key, value) = item?;
        // Skip block transaction index keys (start with 'B')
        if key.first() == Some(&b'B') {
            continue;
        }
        // Transaction value format: version (i32) + height (i32) + raw_tx_bytes
        if value.len() < 8 {
            continue; // Invalid transaction data
        }
        // Check height: skip orphaned and unresolved transactions
        let height_bytes: [u8; 4] = value[4..8].try_into().unwrap_or([0,0,0,0]);
        let height = i32::from_le_bytes(height_bytes);
        if !should_index_transaction(height) {
            continue;
        }
        
        // Extract txid bytes from CF key (strip 't' prefix)
        let txid_bytes = txid_from_key(&key);
        if txid_bytes.is_empty() {
            continue; // Invalid key format
        }
        
        pass2_tx_total += 1;
        
        // O1: Try to get transaction from cache first
        let tx = if let Some(cached_tx) = tx_cache.get(&txid_bytes) {
            cache_hits += 1;
            pass2_tx_deserialized += 1;
            Arc::clone(cached_tx)
        } else {
            cache_misses += 1;
            let raw_tx = &value[8..]; // Skip version + height
            let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
            tx_with_header.extend_from_slice(&[0u8; 4]); // Dummy block version
            tx_with_header.extend_from_slice(raw_tx);
            match deserialize_transaction(&tx_with_header).await {
                Ok(tx) => {
                    pass2_tx_deserialized += 1;
                    Arc::new(tx)
                }
                Err(e) => {
                    pass2_tx_failed += 1;
                    let txid_hex = hex::encode(&txid_bytes);
                    warn!(txid = %txid_hex, height = height, error = ?e, "Pass 2: Failed to deserialize transaction");
                    continue;
                }
            }
        };
        
        // Track which addresses are involved in this transaction (for txs_map)
        let mut tx_addresses: HashSet<String> = HashSet::new();
        
        // Detect if this is a coinstake transaction
        let tx_is_coinstake = is_coinstake(&*tx);
        
        for (vout_index, output) in tx.outputs.iter().enumerate() {
            // PIVX Core Rule: Skip vout[0] in coinstake (OP_RETURN marker)
            if tx_is_coinstake && vout_index == 0 {
                continue;
            }
            
            // Classify the output script
            let script_class = classify_output(output);
            
            match script_class {
                ScriptClassification::P2PKH(addr) |
                ScriptClassification::P2SH(addr) |
                ScriptClassification::P2PK(addr) => {
                    // Standard single-address output
                    tx_addresses.insert(addr.clone());
                    *totals_received.entry(addr.clone()).or_insert(0) += output.value;
                    
                    // Index UTXO if non-zero value
                    if output.value > 0 {
                        address_map
                            .entry(addr.clone())
                            .or_default()
                            .push((txid_bytes.clone(), output.index));
                        indexed_outputs += 1;
                    }
                }
                
                ScriptClassification::ColdStake { staker, owner } => {
                    // CRITICAL FIX: PIVX Core attribution for cold staking
                    // - STAKER receives the value (delegation)
                    // - OWNER receives NO value (they already own the coins)
                    // - BOTH appear in transaction list
                    
                    *totals_received.entry(staker.clone()).or_insert(0) += output.value;
                    // Owner gets NO value added to total_received
                    
                    // Both addresses appear in transaction list
                    tx_addresses.insert(staker.clone());
                    tx_addresses.insert(owner.clone());
                    
                    // Both get UTXO entry for tracking
                    if output.value > 0 {
                        address_map
                            .entry(staker.clone())
                            .or_default()
                            .push((txid_bytes.clone(), output.index));
                        address_map
                            .entry(owner.clone())
                            .or_default()
                            .push((txid_bytes.clone(), output.index));
                        indexed_outputs += 2;  // Count both
                    }
                }
                
                ScriptClassification::OpReturn |
                ScriptClassification::Coinbase |
                ScriptClassification::Coinstake |
                ScriptClassification::Nonstandard => {
                    // No address attribution for these
                }
            }
        }
        
        // Add this transaction to txs_map for ALL addresses involved
        for address_str in tx_addresses {
            txs_map
                .entry(address_str)
                .or_default()
                .push(txid_bytes.clone());
        }
        
        processed += 1;
    }
    
    // O1: Report cache performance
    let cache_hit_rate = if cache_hits + cache_misses > 0 {
        (cache_hits as f64 / (cache_hits + cache_misses) as f64) * 100.0
    } else {
        0.0
    };
    info!(
        cache_hit_rate = cache_hit_rate,
        cache_hits = cache_hits,
        cache_misses = cache_misses,
        pass2_total = pass2_tx_total,
        pass2_deserialized = pass2_tx_deserialized,
        pass2_failed = pass2_tx_failed,
        pass2_outputs = pass2_outputs_processed,
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
    
    // O1: Track cache performance in Pass 2b
    let mut pass2b_cache_hits = 0;
    let mut _pass2b_cache_misses = 0;
    let mut pass2b_db_reads = 0;
    
    // Phase 2 Instrumentation: Track Pass 2b metrics
    let mut pass2b_tx_total = 0;
    let mut pass2b_tx_deserialized = 0;
    let mut pass2b_tx_failed = 0;
    let mut pass2b_coinstake_skipped = 0;
    let mut pass2b_inputs_processed = 0;
    
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
        
        pass2b_tx_total += 1;
        
        // O1: Try cache first for current transaction
        let tx = if let Some(cached_tx) = tx_cache.get(&current_txid_bytes) {
            pass2b_cache_hits += 1;
            pass2b_tx_deserialized += 1;
            Arc::clone(cached_tx)
        } else {
            _pass2b_cache_misses += 1;
            let raw_tx = &value[8..];
            let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
            tx_with_header.extend_from_slice(&[0u8; 4]);
            tx_with_header.extend_from_slice(raw_tx);
            match deserialize_transaction(&tx_with_header).await {
                Ok(tx) => {
                    pass2b_tx_deserialized += 1;
                    Arc::new(tx)
                }
                Err(e) => {
                    pass2b_tx_failed += 1;
                    let txid_hex = hex::encode(&current_txid_bytes);
                    warn!(txid = %txid_hex, height = height, error = ?e, "Pass 2b: Failed to deserialize transaction");
                    continue;
                }
            }
        };
        
        // PIVX Core Rule: Skip coinstake transactions in Pass 2b
        // Stake inputs are consumed for staking, NOT counted as "sent"
        let tx_is_coinstake = is_coinstake(&*tx);
        if tx_is_coinstake {
            pass2b_coinstake_skipped += 1;
            // Coinstake transactions don't count inputs as "sent"
            // The stake is consumed, rewards go to staker/owner
            input_processed += 1;
            continue;
        }
        
        // For every input, find the prevout's addresses and attribute this tx to them
        for input in &tx.inputs {
            if input.coinbase.is_some() { continue; }
            pass2b_inputs_processed += 1;
            if let Some(prevout) = &input.prevout {
                // prevout.hash from parser.rs is already in internal (reversed) format
                if let Ok(prev_txid_internal) = txid_from_hex(&prevout.hash) {
                    // O1: Try cache first - this is the CRITICAL optimization for Pass 2b!
                    let prev_tx = if let Some(cached_prev_tx) = tx_cache.get(&prev_txid_internal) {
                        pass2b_cache_hits += 1;
                        Some(Arc::clone(cached_prev_tx))
                    } else {
                        // Cache miss - need to read from DB
                        pass2b_db_reads += 1;
                        let prev_tx_key = tx_cf_key(&prev_txid_internal);
                        if let Some(prev_tx_data) = db.get_cf(&cf_transactions, &prev_tx_key).ok().flatten() {
                            if prev_tx_data.len() >= 8 {
                                let prev_raw_tx = &prev_tx_data[8..];
                                let mut prev_with_header = Vec::with_capacity(4 + prev_raw_tx.len());
                                prev_with_header.extend_from_slice(&[0u8; 4]);
                                prev_with_header.extend_from_slice(prev_raw_tx);
                                deserialize_transaction(&prev_with_header).await.ok().map(Arc::new)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };
                    
                    if let Some(prev_tx) = prev_tx {
                        if let Some(prev_out) = prev_tx.outputs.get(prevout.n as usize) {
                            // Classify the previous output
                            let prev_script_class = classify_output(prev_out);
                            
                            match prev_script_class {
                                ScriptClassification::P2PKH(addr) |
                                ScriptClassification::P2SH(addr) |
                                ScriptClassification::P2PK(addr) => {
                                    // Standard: address is spending
                                    *totals_sent.entry(addr.clone()).or_insert(0) += prev_out.value;
                                    txs_map.entry(addr.clone()).or_default().push(current_txid_bytes.clone());
                                }
                                
                                ScriptClassification::ColdStake { staker, owner } => {
                                    // CRITICAL FIX: PIVX Core cold stake spending
                                    // - Only OWNER can spend cold staked coins
                                    // - STAKER cannot spend (delegation only)
                                    // - BOTH appear in transaction list
                                    
                                    *totals_sent.entry(owner.clone()).or_insert(0) += prev_out.value;
                                    // Staker's total_sent is NOT increased
                                    
                                    // Both appear in transaction list
                                    txs_map.entry(staker.clone()).or_default().push(current_txid_bytes.clone());
                                    txs_map.entry(owner.clone()).or_default().push(current_txid_bytes.clone());
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
        input_processed += 1;
    }
    
    // O1: Final Pass 2b cache statistics
    let total_pass2b_lookups = pass2b_cache_hits + pass2b_db_reads;
    let pass2b_cache_hit_rate = if total_pass2b_lookups > 0 {
        (pass2b_cache_hits as f64 / total_pass2b_lookups as f64) * 100.0
    } else {
        0.0
    };
    info!(
        input_processed = input_processed,
        pass2b_total = pass2b_tx_total,
        pass2b_deserialized = pass2b_tx_deserialized,
        pass2b_failed = pass2b_tx_failed,
        pass2b_coinstake_skipped = pass2b_coinstake_skipped,
        pass2b_inputs = pass2b_inputs_processed,
        cache_hit_rate = pass2b_cache_hit_rate,
        cache_hits = pass2b_cache_hits,
        db_reads = pass2b_db_reads,
        db_reads_eliminated = pass2b_cache_hits,
        "Pass 2b complete"
    );
    
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
    
    for (address, utxos) in address_map {
        let mut key = vec![b'a'];
        key.extend_from_slice(address.as_bytes());
        
        // Build canonical UTXO list (only unspent entries) to match serialize_utxos format
        let mut utxos_unspent: Vec<(Vec<u8>, u64)> = Vec::new();

        for (txid_bytes, vout) in utxos.iter() {
            total_utxos_checked += 1;
            
            // Check spent status using natural byte order (matching Pass 1)
            let is_spent = spent_outputs.contains(&(txid_bytes.clone(), *vout));
            
            if is_spent {
                total_spent_found += 1;
            }
            
            if !is_spent {
                utxos_unspent.push((txid_bytes.clone(), *vout));
            }
        }

        // Serialize UTXOs in canonical format used by the API (txid(32) + vout(u64) per entry)
        let serialized_utxos = serialize_utxos(&utxos_unspent).await;
        batch.put_cf(&cf_addr_index, &key, &serialized_utxos);

        // Get pre-calculated totals from Pass 2 and 2b (MUCH faster than recalculating!)
        let total_received = *totals_received.get(&address).unwrap_or(&0);
        let total_sent = *totals_sent.get(&address).unwrap_or(&0);
        
        // Write transaction list ('t' + address)
        if let Some(txids) = txs_map.get(&address) {
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
    
    Ok(())
}

