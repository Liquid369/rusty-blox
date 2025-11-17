/// Address Enrichment Module
/// 
/// Builds address index after fast_sync completes.
/// This reads all transactions and creates address -> [txids] mappings
/// without modifying the original transaction data.

use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use rocksdb::DB;
use crate::parser::{deserialize_transaction, serialize_utxos_with_spent};

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
        let raw_tx = &value[8..];
        
        // Deserialize to get inputs
        let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
        tx_with_header.extend_from_slice(&[0u8; 4]);
        tx_with_header.extend_from_slice(raw_tx);
        
        if let Ok(tx) = deserialize_transaction(&tx_with_header).await {
            for input in &tx.inputs {
                // DEBUG: Check our test transactions' inputs
                if let Some(prevout) = &input.prevout {
                    if let Ok(prev_txid_bytes) = hex::decode(&prevout.hash) {
                        let txid_display = hex::encode(&prev_txid_bytes);
                        if txid_display.starts_with("74f4914e") || txid_display.starts_with("9c4c3819") || txid_display.starts_with("f6592f84") {
                            println!("     🔎 INPUT FOUND: {} vout {}, coinbase={:?}", 
                                txid_display, prevout.n, input.coinbase.is_some());
                        }
                    }
                }
                
                // Skip coinbase inputs
                if input.coinbase.is_some() {
                    continue;
                }
                
                if let Some(prevout) = &input.prevout {
                    if let Ok(prev_txid_bytes) = hex::decode(&prevout.hash) {
                        let prev_txid_internal: Vec<u8> = prev_txid_bytes.iter().rev().cloned().collect();
                        
                        // DEBUG: Log what we're adding to spent_outputs for our test case
                        let txid_display = hex::encode(&prev_txid_bytes);
                        if txid_display.starts_with("74f4914ed8970af9f3ee") || 
                           txid_display.starts_with("9c4c381950afe8a10b39") || 
                           txid_display.starts_with("f6592f84a1855bc4a558") {
                            println!("     ✅ DEBUG: About to add internal_hex={} vout={}", 
                                hex::encode(&prev_txid_internal), prevout.n);
                        }
                        
                        spent_outputs.insert((prev_txid_internal, prevout.n as u64));
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
    
    // PASS 2: Build address map with spent flags
    let mut address_map: HashMap<String, Vec<(Vec<u8>, u64)>> = HashMap::new();
    let iter2 = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    
    for item in iter2 {
        let (key, value) = item?;
        
        // Skip block transaction index keys (start with 'B')
        if key.first() == Some(&b'B') {
            continue;
        }
        
        // Transaction value format: version (i32) + height (i32) + raw_tx_bytes
        // Skip first 8 bytes to get to raw transaction data
        if value.len() < 8 {
            continue; // Invalid transaction data
        }
        let raw_tx = &value[8..]; // Skip version + height
        
        // deserialize_transaction expects block_version (4 bytes) + tx data
        // Add dummy block version before raw tx
        let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
        tx_with_header.extend_from_slice(&[0u8; 4]); // Dummy block version
        tx_with_header.extend_from_slice(raw_tx);
        
        // Deserialize transaction
        let tx = match deserialize_transaction(&tx_with_header).await {
            Ok(tx) => tx,
            Err(_) => {
                // Skip corrupted transactions
                continue;
            }
        };
        
        // Extract txid from key (skip 't' prefix)
        let txid = key[1..].to_vec();
        
        // NO LONGER TRACKING SPENT OUTPUTS HERE - we did that in Pass 1!
        // Just index the outputs
        
        // Index each output by address
        for output in &tx.outputs {
            // Skip zero-value outputs (like coinstake vout 0)
            if output.value == 0 {
                continue;
            }
            
            for address_str in &output.address {
                if address_str.is_empty() || 
                   address_str == "Nonstandard" || 
                   address_str == "CoinBaseTx" ||
                   address_str == "CoinStakeTx" {
                    continue;
                }
                
                // DEBUG: Show what we're indexing for our test address
                if address_str == "DR6swVK8eVaMCnh7ChYrpwsRjAB2SkT21G" {
                    let mut txid_display = txid.clone();
                    txid_display.reverse();
                    println!("      📌 INDEXING: txid={} vout={} value={}", 
                        hex::encode(&txid_display), output.index, output.value);
                }
                
                address_map
                    .entry(address_str.clone())
                    .or_insert_with(Vec::new)
                    .push((txid.clone(), output.index));
                
                indexed_outputs += 1;
            }
        }
        
        processed += 1;
        
        // Progress reporting
        if processed % 50000 == 0 {
            println!("  Processed {} transactions, {} outputs indexed", 
                     processed, indexed_outputs);
        }
    }
    
    println!("\n📝 Writing address index to database...");
    println!("   {} unique addresses found", address_map.len());
    println!("   spent_outputs HashSet size: {}", spent_outputs.len());
    
    // Write address mappings to database
    let mut batch = rocksdb::WriteBatch::default();
    let mut written = 0;
    
    for (address, utxos) in address_map {
        let mut key = vec![b'a'];
        key.extend_from_slice(address.as_bytes());
        
        // Add spent flags to each UTXO
        let utxos_with_spent: Vec<(Vec<u8>, u64, bool)> = utxos.iter()
            .map(|(txid, vout)| {
                let is_spent = spent_outputs.contains(&(txid.clone(), *vout));
                
                // DEBUG: Log for our test address
                if address == "DR6swVK8eVaMCnh7ChYrpwsRjAB2SkT21G" {
                    let tuple_to_find = (txid.clone(), *vout);
                    let direct_check = spent_outputs.contains(&tuple_to_find);
                    println!("      🔍 vout={} is_spent_from_map={} direct_contains={}", 
                        vout, is_spent, direct_check);
                }
                
                (txid.clone(), *vout, is_spent)
            })
            .collect();
        
        let serialized_utxos = serialize_utxos_with_spent(&utxos_with_spent).await;
        batch.put_cf(&cf_addr_index, &key, &serialized_utxos);
        
        written += 1;
        
        if batch.len() >= batch_size {
            db.write(batch)?;
            batch = rocksdb::WriteBatch::default();
            
            if written % 100000 == 0 {
                println!("  Written {} addresses...", written);
            }
        }
    }
    
    // Write final batch
    if batch.len() > 0 {
        db.write(batch)?;
    }
    
    println!();
    println!("✅ Address index building complete!");
    println!("   Total transactions scanned: {}", processed);
    println!("   Total outputs indexed: {}", indexed_outputs);
    println!("   Total spent outputs marked: {}", spent_outputs.len());
    println!("   Unique addresses: {}", written);
    println!();
    
    Ok(())
}

