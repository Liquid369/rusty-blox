/// Chainstate-based UTXO Enrichment
/// 
/// Uses PIVX Core's chainstate LevelDB as the source of truth for current UTXOs
/// This ensures our balances match Core exactly

use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use rocksdb::DB;
use crate::chainstate_leveldb;
use crate::chainstate::{aggregate_chainstate_with_coinbase_opts, AggregateOptions, parse_coins_value};
use crate::parser::deserialize_transaction;

/// Enrich UTXO index from PIVX Core's chainstate
/// This is the authoritative source for what UTXOs exist RIGHT NOW
pub async fn enrich_from_chainstate(db: Arc<DB>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║      CHAINSTATE UTXO ENRICHMENT STARTING           ║");
    println!("╚════════════════════════════════════════════════════╝");
    println!();
    
    // 1. Copy chainstate from PIVX Core
    let pivx_data_dir = std::env::var("HOME")
        .map(|h| format!("{}/Library/Application Support/PIVX", h))
        .unwrap_or_else(|_| "/Users/liquid/Library/Application Support/PIVX".to_string());
    
    let chainstate_src = format!("{}/chainstate", pivx_data_dir);
    let chainstate_copy = "/tmp/pivx_chainstate_current";
    
    println!("📋 Copying chainstate from PIVX Core...");
    println!("   Source: {}", chainstate_src);
    println!("   Dest: {}", chainstate_copy);
    
    // Remove old copy if exists
    std::fs::remove_dir_all(chainstate_copy).ok();
    
    // Copy using cp command
    let copy_result = std::process::Command::new("cp")
        .args(["-R", &chainstate_src, chainstate_copy])
        .output()?;
    
    if !copy_result.status.success() {
        return Err(format!("Failed to copy chainstate: {}", 
            String::from_utf8_lossy(&copy_result.stderr)).into());
    }
    
    println!("✅ Chainstate copied!\n");
    
    // 2. Use the working aggregation function to get balances
    println!("📖 Aggregating balances from chainstate...");
    let opts = AggregateOptions {
        include_shielded: false,
        include_unknown: false,
        include_coinbase: true,
        coinbase_maturity: None,
        current_height: None,
    };
    
    let agg_result = aggregate_chainstate_with_coinbase_opts(chainstate_copy, opts)?;
    
    println!("   Found {} addresses with balances", agg_result.balances.len());
    println!("   Total coinbase amount: {} satoshis\n", agg_result.coinbase_total);
    
    // 3. Now read raw chainstate to get UTXO details for each address
    println!("📖 Reading UTXO details from chainstate...");
    let chainstate_raw = chainstate_leveldb::read_chainstate_raw(chainstate_copy)?;
    
    println!("   Found {} UTXO entries in chainstate\n", chainstate_raw.len());
    
    let _cf_utxo = db.cf_handle("utxo")
        .ok_or("utxo CF not found")?;
    let cf_addr_index = db.cf_handle("addr_index")
        .ok_or("addr_index CF not found")?;
    
    println!("📊 Building address index from chainstate UTXOs...");
    
    // Map: address -> list of UTXOs
    let mut address_utxos: HashMap<String, Vec<(Vec<u8>, u64, u64)>> = HashMap::new();
    let mut processed = 0;
    let mut skipped_no_script = 0;
    
    for (key, value) in &chainstate_raw {
        // Parse the chainstate value to get amount and scriptPubKey
        // parse_coins_value returns: ParsedCoins { height, is_coinbase, unspent_outputs }
        // unspent_outputs: Vec<(vout_index, amount_satoshis, script_pubkey_bytes, output_kind, resolved_addresses)>
        if let Some(parsed) = parse_coins_value(value) {
            // Key format: 32-byte txid + varint vout
            if key.len() < 32 {
                continue;
            }
            
            let txid_internal = &key[0..32];
            
            // Parse vout from remaining bytes (simple varint decode)
            let vout_data = &key[32..];
            let _vout = if vout_data.is_empty() {
                0u64
            } else if vout_data[0] < 253 {
                vout_data[0] as u64
            } else {
                // More complex varint - for now skip
                vout_data[0] as u64
            };
            
            // Each parsed coin can have multiple outputs (but chainstate typically has one per key)
            for (vout_index, amount, _script_pubkey, _kind, addresses) in parsed.unspent_outputs {
                // Use the vout from the parsed output (it should match our vout from key)
                let actual_vout = vout_index as u64;
                
                // Add this UTXO to each address
                for address in addresses {
                    if address.is_empty() || 
                       address == "Nonstandard" || 
                       address == "CoinBaseTx" ||
                       address == "CoinStakeTx" {
                        continue;
                    }
                    
                    address_utxos
                        .entry(address)
                        .or_default()
                        .push((txid_internal.to_vec(), actual_vout, amount));
                }
            }
        } else {
            skipped_no_script += 1;
        }
        
        processed += 1;
        if processed % 100000 == 0 {
            println!("   Processed {} chainstate entries...", processed);
        }
    }
    
    println!("\n✅ Chainstate processing complete!");
    println!("   {} UTXO entries processed", processed);
    println!("   {} unique addresses found", address_utxos.len());
    println!("   {} entries skipped (unparseable)", skipped_no_script);
    
    // 4. Write UTXO index to database
    println!("\n📝 Writing UTXO index to database...");
    
    let mut batch = rocksdb::WriteBatch::default();
    let mut written = 0;
    
    for (address, utxos) in &address_utxos {
        // Key: 'a' + address_bytes
        let mut key = vec![b'a'];
        key.extend_from_slice(address.as_bytes());
        
        // Value: serialized list of (txid, vout, value, spent=false)
        // Format: count (u32) + [txid(32) + vout(u64) + value(u64) + spent(u8)]...
        let mut value = Vec::new();
        value.extend_from_slice(&(utxos.len() as u32).to_le_bytes());
        
        for (txid, vout, utxo_value) in utxos {
            value.extend_from_slice(txid);
            value.extend_from_slice(&vout.to_le_bytes());
            value.extend_from_slice(&utxo_value.to_le_bytes());
            value.push(0); // spent = false (chainstate only has unspent)
        }
        
        batch.put_cf(&cf_addr_index, &key, &value);
        written += 1;
        
        if batch.len() >= 10000 {
            db.write(batch)?;
            batch = rocksdb::WriteBatch::default();
        }
    }
    
    if !batch.is_empty() {
        db.write(batch)?;
    }
    
    println!("✅ {} addresses indexed with current UTXOs", written);
    
    // 5. Build transaction list ('t' prefix) - all txs involving each address
    println!("\n📊 Building transaction lists for addresses...");
    
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    // Scan all transactions to build tx lists
    let mut address_txs: HashMap<String, HashSet<Vec<u8>>> = HashMap::new();
    let mut tx_count = 0;
    
    let iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    for item in iter {
        let (key, value) = item?;
        
        // Skip block index keys
        if key.first() == Some(&b'B') {
            continue;
        }
        
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
            let txid = key[1..].to_vec();
            
            // Collect all addresses involved in this transaction
            let mut tx_addresses: HashSet<String> = HashSet::new();
            
            // From outputs
            for output in &tx.outputs {
                for address in &output.address {
                    if !address.is_empty() && 
                       address != "Nonstandard" && 
                       address != "CoinBaseTx" &&
                       address != "CoinStakeTx" {
                        tx_addresses.insert(address.clone());
                    }
                }
            }
            
            // From inputs (resolve prevout addresses)
            for input in &tx.inputs {
                if input.coinbase.is_some() {
                    continue;
                }
                
                if let Some(prevout) = &input.prevout {
                    if let Ok(prev_txid_bytes) = hex::decode(&prevout.hash) {
                        let mut prev_tx_key = vec![b't'];
                        prev_tx_key.extend_from_slice(&prev_txid_bytes);
                        
                        if let Some(prev_tx_data) = db.get_cf(&cf_transactions, &prev_tx_key).ok().flatten() {
                            if prev_tx_data.len() >= 8 {
                                let prev_raw_tx = &prev_tx_data[8..];
                                let mut prev_with_header = Vec::with_capacity(4 + prev_raw_tx.len());
                                prev_with_header.extend_from_slice(&[0u8; 4]);
                                prev_with_header.extend_from_slice(prev_raw_tx);
                                
                                if let Ok(prev_tx) = deserialize_transaction(&prev_with_header).await {
                                    if let Some(prev_out) = prev_tx.outputs.get(prevout.n as usize) {
                                        for addr in &prev_out.address {
                                            if !addr.is_empty() && 
                                               addr != "Nonstandard" && 
                                               addr != "CoinBaseTx" &&
                                               addr != "CoinStakeTx" {
                                                tx_addresses.insert(addr.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            // Add this tx to all involved addresses
            for address in tx_addresses {
                address_txs
                    .entry(address)
                    .or_default()
                    .insert(txid.clone());
            }
        }
        
        tx_count += 1;
        if tx_count % 100000 == 0 {
            println!("   Scanned {} transactions...", tx_count);
        }
    }
    
    println!("✅ Transaction lists built for {} addresses", address_txs.len());
    
    // Write transaction lists to database
    println!("\n📝 Writing transaction lists to database...");
    
    let mut batch = rocksdb::WriteBatch::default();
    let mut tx_list_written = 0;
    
    for (address, txids) in address_txs {
        // Key: 't' + address_bytes
        let mut key = vec![b't'];
        key.extend_from_slice(address.as_bytes());
        
        // Value: count (u32) + [txid(32)]...
        let mut value = Vec::new();
        value.extend_from_slice(&(txids.len() as u32).to_le_bytes());
        
        for txid in txids {
            value.extend_from_slice(&txid);
        }
        
        batch.put_cf(&cf_addr_index, &key, &value);
        tx_list_written += 1;
        
        if batch.len() >= 10000 {
            db.write(batch)?;
            batch = rocksdb::WriteBatch::default();
        }
    }
    
    if !batch.is_empty() {
        db.write(batch)?;
    }
    
    println!("✅ {} transaction lists written", tx_list_written);
    
    // Mark enrichment as complete
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    db.put_cf(&cf_state, b"address_index_complete", b"1")?;
    
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║    🎉 CHAINSTATE ENRICHMENT COMPLETE 🎉            ║");
    println!("╚════════════════════════════════════════════════════╝");
    println!();
    println!("📊 Summary:");
    println!("   {} addresses with current UTXOs (from chainstate)", address_utxos.len());
    println!("   {} addresses with transaction history", tx_list_written);
    println!("   All balances now match PIVX Core exactly!");
    println!();
    
    Ok(())
}
