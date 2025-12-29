use std::sync::Arc;
use std::collections::HashMap;
use rocksdb::{DB, Options};
use rustyblox::config::load_config;
use rustyblox::parser::deserialize_transaction;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::args().nth(1)
        .unwrap_or_else(|| "DCSAJGThtCnDokqawZehRvVjdms9XLL6J6".to_string());
    
    println!("🔍 Analyzing address: {}\n", address);
    
    // Open RocksDB
    let config = load_config()?;
    let db_path = config.get_string("paths.db_path")
        .unwrap_or_else(|_| "./data/blocks.db".to_string());
    
    let cf_names = vec![
        "default", "blocks", "transactions", "addr_index", 
        "utxo", "chain_metadata", "pubkey", "chain_state"
    ];
    
    let db = Arc::new(DB::open_cf(&Options::default(), &db_path, &cf_names)?);
    
    // Get address data from our DB
    let cf_addr_index = db.cf_handle("addr_index").ok_or("addr_index CF not found")?;
    let cf_transactions = db.cf_handle("transactions").ok_or("transactions CF not found")?;
    
    let mut key_addr = vec![b'a'];
    key_addr.extend_from_slice(address.as_bytes());
    
    let mut key_txs = vec![b't'];
    key_txs.extend_from_slice(address.as_bytes());
    
    // Get UTXOs
    let utxos_data = db.get_cf(&cf_addr_index, &key_addr)?;
    let txs_data_ref = db.get_cf(&cf_addr_index, &key_txs)?;
    let txs_data = txs_data_ref.clone();
    
    println!("📊 Our Explorer Data:");
    println!("   UTXO key exists: {}", utxos_data.is_some());
    println!("   TX list key exists: {}", txs_data.is_some());
    
    if let Some(ref data) = utxos_data {
        println!("   UTXO data size: {} bytes", data.len());
    }
    
    if let Some(ref data) = txs_data {
        println!("   TX data size: {} bytes", data.len());
    }
    
    // Parse transaction list (stored as concatenated 32-byte txids, no count prefix)
    let mut our_txs: Vec<(Vec<u8>, i32)> = Vec::new();
    if let Some(txs_bytes) = txs_data_ref {
        let tx_count = txs_bytes.len() / 32;
        println!("   Transaction count: {}", tx_count);
        
        for i in 0..tx_count {
            let offset = i * 32;
            if offset + 32 <= txs_bytes.len() {
                let txid = txs_bytes[offset..offset+32].to_vec();
                
                // Transactions are stored with key: 't' + txid (in display/reversed format)
                let mut tx_key = vec![b't'];
                tx_key.extend_from_slice(&txid);
                
                // Get tx height
                let height = if let Some(tx_data) = db.get_cf(&cf_transactions, &tx_key)? {
                    if tx_data.len() >= 8 {
                        i32::from_le_bytes([tx_data[4], tx_data[5], tx_data[6], tx_data[7]])
                    } else {
                        -1
                    }
                } else {
                    -1
                };
                
                our_txs.push((txid.clone(), height));
            }
        }
    }
    
    println!("\n   Our transactions (first 10):");
    for (i, (txid, height)) in our_txs.iter().take(10).enumerate() {
        let txid_hex: String = txid.iter().rev().map(|b| format!("{:02x}", b)).collect();
        println!("     {}: {} (height: {})", i+1, txid_hex, height);
    }
    
    // Calculate our balances
    let mut total_received = 0i64;
    let mut total_sent = 0i64;
    let mut tx_count = 0;
    
    // Track which outputs we've seen
    let mut outputs_by_tx: HashMap<Vec<u8>, Vec<(u64, i64, bool)>> = HashMap::new();
    let mut inputs_by_tx: HashMap<Vec<u8>, Vec<(Vec<u8>, u64)>> = HashMap::new();
    
    let mut debug_input_count = 0;
    let mut debug_prevout_found = 0;
    let mut debug_prevout_to_address = 0;
    
    for (txid, height) in &our_txs {
        if *height == -1 {
            continue; // Skip orphaned
        }
        
        // Transactions are stored with key: 't' + txid
        let mut tx_key = vec![b't'];
        tx_key.extend_from_slice(txid);
        
        if let Some(tx_data) = db.get_cf(&cf_transactions, &tx_key)? {
            if tx_data.len() >= 8 {
                let raw_tx = &tx_data[8..];
                let mut tx_with_header = Vec::with_capacity(4 + raw_tx.len());
                tx_with_header.extend_from_slice(&[0u8; 4]);
                tx_with_header.extend_from_slice(raw_tx);
                
                if let Ok(tx) = deserialize_transaction(&tx_with_header).await {
                    tx_count += 1;
                    
                    // Check outputs to this address
                    for output in &tx.outputs {
                        if output.address.contains(&address.to_string()) {
                            outputs_by_tx.entry(txid.clone())
                                .or_default()
                                .push((output.index, output.value, false));
                            total_received += output.value;
                        }
                    }
                    
                    // Check inputs from this address
                    for input in &tx.inputs {
                        debug_input_count += 1;
                        if input.coinbase.is_some() {
                            continue;
                        }
                        if let Some(prevout) = &input.prevout {
                            if let Ok(prev_txid_bytes) = hex::decode(&prevout.hash) {
                                // prevout.hash is already in display format (reversed), so we use it directly
                                // Transactions are stored with key: 't' + txid (in display/reversed format)
                                let mut prev_tx_key = vec![b't'];
                                prev_tx_key.extend_from_slice(&prev_txid_bytes);
                                
                                // Check if prevout belongs to our address
                                if let Some(prev_tx_data) = db.get_cf(&cf_transactions, &prev_tx_key)? {
                                    debug_prevout_found += 1;
                                    if prev_tx_data.len() >= 8 {
                                        let prev_raw_tx = &prev_tx_data[8..];
                                        let mut prev_with_header = Vec::with_capacity(4 + prev_raw_tx.len());
                                        prev_with_header.extend_from_slice(&[0u8; 4]);
                                        prev_with_header.extend_from_slice(prev_raw_tx);
                                        
                                        if let Ok(prev_tx) = deserialize_transaction(&prev_with_header).await {
                                            if let Some(prev_out) = prev_tx.outputs.get(prevout.n as usize) {
                                                if debug_prevout_found <= 3 {
                                                    println!("   DEBUG prev_out addresses: {:?}", prev_out.address);
                                                    println!("   DEBUG looking for: {}", address);
                                                }
                                                if prev_out.address.contains(&address.to_string()) {
                                                    debug_prevout_to_address += 1;
                                                    inputs_by_tx.entry(txid.clone())
                                                        .or_default()
                                                        .push((prev_txid_bytes.clone(), prevout.n as u64));
                                                    total_sent += prev_out.value;
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
        }
    }
    
    let current_balance = total_received - total_sent;
    
    println!("\n� Debug Info:");
    println!("   Total inputs checked: {}", debug_input_count);
    println!("   Prevouts found in DB: {}", debug_prevout_found);
    println!("   Prevouts to this address: {}", debug_prevout_to_address);
    
    println!("\n�💰 Our Explorer Calculations:");
    println!("   Total Received: {:.8} PIV", total_received as f64 / 100_000_000.0);
    println!("   Total Sent: {:.8} PIV", total_sent as f64 / 100_000_000.0);
    println!("   Current Balance: {:.8} PIV", current_balance as f64 / 100_000_000.0);
    println!("   Transaction Count: {}", tx_count);
    
    println!("\n📌 Expected (from official explorer):");
    println!("   Total Received: 18,979 PIV");
    println!("   Total Sent: 8,949 PIV");
    println!("   Current Balance: 10,030 PIV");
    println!("   Transaction Count: 2,445");
    
    println!("\n❌ Discrepancies:");
    let expected_received = 18979.0 * 100_000_000.0;
    let expected_sent = 8949.0 * 100_000_000.0;
    let expected_balance = 10030.0 * 100_000_000.0;
    
    println!("   Received diff: {:.8} PIV", (total_received as f64 - expected_received) / 100_000_000.0);
    println!("   Sent diff: {:.8} PIV", (total_sent as f64 - expected_sent) / 100_000_000.0);
    println!("   Balance diff: {:.8} PIV", (current_balance as f64 - expected_balance) / 100_000_000.0);
    println!("   TX count diff: {}", tx_count - 2445);
    
    Ok(())
}
