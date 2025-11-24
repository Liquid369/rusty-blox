/// Address Enrichment Module
/// 
/// Builds address index after fast_sync completes.
/// This reads all transactions and creates address -> [txids] mappings
/// without modifying the original transaction data.

use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use rocksdb::DB;
use crate::parser::{deserialize_transaction, serialize_utxos};
use crate::tx_keys::{tx_cf_key, txid_from_key, txid_from_hex};

/// Build address index from all transactions
/// This creates the addr_index CF entries for address lookups
pub async fn enrich_all_addresses(db: Arc<DB>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║          ADDRESS INDEX BUILDING STARTING           ║");
    println!("╚════════════════════════════════════════════════════╝");
    println!();
    println!("Building address index from transactions...");
    println!("This indexes addresses for API queries.\n");

    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    let cf_addr_index = db.cf_handle("addr_index")
        .ok_or("addr_index CF not found")?;

    let mut processed = 0;
    let mut indexed_outputs = 0;
    let batch_size = 10000;
    
    println!("📊 Two-pass address indexing:");
    println!("   Pass 1: Building complete spent outputs set...");
    
    // PASS 1: Build complete spent outputs set by scanning ALL transaction inputs
    let mut spent_outputs: HashSet<(Vec<u8>, u64)> = HashSet::new();
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
        // Check height: skip orphaned
        let height_bytes: [u8; 4] = value[4..8].try_into().unwrap_or([0,0,0,0]);
        let height = i32::from_le_bytes(height_bytes);
        if height == -1 {
            continue;
        }
        let raw_tx = &value[8..];
        let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
        tx_with_header.extend_from_slice(&[0u8; 4]);
        tx_with_header.extend_from_slice(raw_tx);
        if let Ok(tx) = deserialize_transaction(&tx_with_header).await {
            for input in &tx.inputs {
                if input.coinbase.is_some() {
                    continue;
                }
                if let Some(prevout) = &input.prevout {
                    // prevout.hash is hex-encoded display-order txid
                    // txid_from_hex decodes to natural byte order (no reversal needed)
                    if let Ok(prev_txid_bytes) = txid_from_hex(&prevout.hash) {
                        spent_outputs.insert((prev_txid_bytes.clone(), prevout.n as u64));
                    }
                }
            }
        }
        processed += 1;
        if processed % 100000 == 0 {
            println!("     Scanned {} transactions, {} spent outputs found", processed, spent_outputs.len());
        }
    }
    
    println!("   ✅ Pass 1 complete: {} transactions scanned, {} spent outputs found\n", processed, spent_outputs.len());
    
    // DEBUG: Immediately check if our test tuples are in the HashSet
    let test_txids = vec![
        ("d8138a0b1b171e861bf49d31c1ad4ff986ef8142bbe3eef3f90a97d84e91f474", 1u64),
        ("2fb7c2c919d1eb2138f996778ece156f8726ea42d263390ba1e8af5019384c9c", 1u64),
    ];
    for (txid_hex, vout) in &test_txids {
        if let Ok(txid_bytes) = hex::decode(txid_hex) {
            let found = spent_outputs.contains(&(txid_bytes.clone(), *vout));
            println!("   🔍 DEBUG: After Pass 1, checking {} vout {}: found={}", &txid_hex[..16], vout, found);
        }
    }
    
    println!("   Pass 2: Indexing outputs with spent flags...");
    
    // Reset counter for pass 2
    processed = 0;
    
    // PASS 2: Build address map with spent flags (outputs -> address_map)
    let mut address_map: HashMap<String, Vec<(Vec<u8>, u64)>> = HashMap::new();
    // Also maintain a txs_map to collect all txids involving an address (received OR sent)
    let mut txs_map: HashMap<String, Vec<Vec<u8>>> = HashMap::new();
    // NEW: Track total received and sent per address during Pass 2 (much faster!)
    let mut totals_received: HashMap<String, i64> = HashMap::new();
    let mut totals_sent: HashMap<String, i64> = HashMap::new();
    
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
        // Check height: skip orphaned
        let height_bytes: [u8; 4] = value[4..8].try_into().unwrap_or([0,0,0,0]);
        let height = i32::from_le_bytes(height_bytes);
        if height == -1 {
            continue;
        }
        let raw_tx = &value[8..]; // Skip version + height
        let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
        tx_with_header.extend_from_slice(&[0u8; 4]); // Dummy block version
        tx_with_header.extend_from_slice(raw_tx);
        let tx = match deserialize_transaction(&tx_with_header).await {
            Ok(tx) => tx,
            Err(_) => {
                continue;
            }
        };
        // Extract txid bytes from CF key (strip 't' prefix)
        let txid_bytes = txid_from_key(&key);
        if txid_bytes.is_empty() {
            continue; // Invalid key format
        }
        
        // Track which addresses are involved in this transaction (for txs_map)
        let mut tx_addresses: HashSet<String> = HashSet::new();
        
        for output in &tx.outputs {
            // Collect addresses from this output (regardless of value)
            for address_str in &output.address {
                if !address_str.is_empty() && 
                   address_str != "Nonstandard" && 
                   address_str != "CoinBaseTx" &&
                   address_str != "CoinStakeTx" {
                    tx_addresses.insert(address_str.clone());
                    // NEW: Add to total_received for this address
                    *totals_received.entry(address_str.clone()).or_insert(0) += output.value;
                }
            }
            
            // For UTXO indexing, skip zero-value outputs
            if output.value == 0 {
                continue;
            }
            
            // Index this output for each address
            for address_str in &output.address {
                if address_str.is_empty() || 
                   address_str == "Nonstandard" || 
                   address_str == "CoinBaseTx" ||
                   address_str == "CoinStakeTx" {
                    continue;
                }
                address_map
                    .entry(address_str.clone())
                    .or_default()
                    .push((txid_bytes.clone(), output.index));
                indexed_outputs += 1;
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
        if processed % 50000 == 0 {
            println!("  Processed {} transactions, {} outputs indexed", processed, indexed_outputs);
        }
    }
    
    println!("\n📝 Writing address index to database...");
    println!("   {} unique addresses found", address_map.len());
    println!("   spent_outputs HashSet size: {}", spent_outputs.len());

    // SECONDARY PASS: Scan all inputs to discover addresses used as inputs (sent transactions)
    // For each input, resolve the prevout's addresses (by reading the previous tx) and add
    // the current txid to those addresses' txs_map so 't' contains both sent and received txs.
    // ALSO calculate total_sent here!
    println!("   Pass 2b: Scanning inputs to include sent transactions and calculate totals...");
    let iter3 = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    let mut input_processed: usize = 0;
    for item in iter3 {
        let (key, value) = item?;
        if key.first() == Some(&b'B') { continue; }
        if value.len() < 8 { continue; }
        let height_bytes: [u8; 4] = value[4..8].try_into().unwrap_or([0,0,0,0]);
        let height = i32::from_le_bytes(height_bytes);
        if height == -1 { continue; }
        let raw_tx = &value[8..];
        let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
        tx_with_header.extend_from_slice(&[0u8; 4]);
        tx_with_header.extend_from_slice(raw_tx);
        let tx = match deserialize_transaction(&tx_with_header).await {
            Ok(tx) => tx,
            Err(_) => { continue; }
        };
        // Extract current txid from key
        let current_txid_bytes = txid_from_key(&key);
        if current_txid_bytes.is_empty() { continue; }
        
        // For every input, find the prevout's addresses and attribute this tx to them
        for input in &tx.inputs {
            if input.coinbase.is_some() { continue; }
            if let Some(prevout) = &input.prevout {
                // prevout.hash is hex-encoded display-order; decode to natural bytes
                if let Ok(prev_txid_bytes) = txid_from_hex(&prevout.hash) {
                    // Build correct CF key with 't' prefix
                    let prev_tx_key = tx_cf_key(&prev_txid_bytes);
                    if let Some(prev_tx_data) = db.get_cf(&cf_transactions, &prev_tx_key).ok().flatten() {
                        if prev_tx_data.len() >= 8 {
                            let prev_raw_tx = &prev_tx_data[8..];
                            let mut prev_with_header = Vec::with_capacity(4 + prev_raw_tx.len());
                            prev_with_header.extend_from_slice(&[0u8; 4]);
                            prev_with_header.extend_from_slice(prev_raw_tx);
                            if let Ok(prev_tx) = deserialize_transaction(&prev_with_header).await {
                                if let Some(prev_out) = prev_tx.outputs.get(prevout.n as usize) {
                                    for addr in &prev_out.address {
                                        if addr.is_empty() || addr == "Nonstandard" || addr == "CoinBaseTx" || addr == "CoinStakeTx" { continue; }
                                        txs_map.entry(addr.clone()).or_default().push(current_txid_bytes.clone());
                                        // NEW: Add to total_sent for this address
                                        *totals_sent.entry(addr.clone()).or_insert(0) += prev_out.value;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        input_processed += 1;
        if input_processed % 200000 == 0 {
            println!("     Scanned {} transactions for inputs", input_processed);
        }
    }
    println!("   Pass 2b complete: scanned {} transactions for inputs", input_processed);
    
    println!("\n📝 Writing address index to database...");
    println!("   {} unique addresses found", address_map.len());
    println!("   Calculating balances and totals for each address...");
    println!("   ⚠️  This may take a while for large address sets (DB lookups for each transaction)");
    
    // Write address mappings to database
    let mut batch = rocksdb::WriteBatch::default();
    let mut written = 0;
    let total_addresses = address_map.len();  // Cache length before consuming map
    
    for (address, utxos) in address_map {
        let mut key = vec![b'a'];
        key.extend_from_slice(address.as_bytes());
        
        // Build canonical UTXO list (only unspent entries) to match serialize_utxos format
        let mut utxos_unspent: Vec<(Vec<u8>, u64)> = Vec::new();

        for (txid_bytes, vout) in utxos.iter() {
            // Check spent status using natural byte order (matching Pass 1)
            let is_spent = spent_outputs.contains(&(txid_bytes.clone(), *vout));

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
        
        // More frequent progress updates for visibility
        if written % 10000 == 0 {
            println!("  Processed {} / {} addresses ({:.1}%)...", written, total_addresses, (written as f64 / total_addresses as f64) * 100.0);
        }
        
        if batch.len() >= batch_size {
            db.write(batch)?;
            batch = rocksdb::WriteBatch::default();
        }
    }
    
    // Write final batch
    if !batch.is_empty() {
        db.write(batch)?;
    }
    
    println!();
    println!("✅ Address index building complete!");
    println!("   Total transactions scanned: {}", processed);
    println!("   Total outputs indexed: {}", indexed_outputs);
    println!("   Total spent outputs marked: {}", spent_outputs.len());
    println!("   Unique addresses with balances: {}", written);
    println!("   ✅ Total received/sent calculated for all addresses");
    println!();
    
    Ok(())
}

