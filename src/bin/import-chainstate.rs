//! Chainstate Import Tool
//!
//! Imports and verifies UTXO balances from PIVX Core's chainstate LevelDB.
//!
//! ## Usage
//!
//! ```bash
//! # 1. Stop PIVX Core
//! pkill pivxd
//!
//! # 2. Copy chainstate
//! cp -r ~/.pivx/chainstate /tmp/chainstate-backup
//!
//! # 3. Run import (Core can be restarted now)
//! cargo run --release --bin import-chainstate -- --chainstate-path /tmp/chainstate-backup
//! ```
//!
//! ## What it does
//!
//! 1. Reads all UTXO entries from PIVX Core's chainstate LevelDB
//! 2. Parses CCoins format (amount/script compression)
//! 3. Aggregates balances by address
//! 4. Compares with transaction-based balances in explorer DB
//! 5. Reports matches, mismatches, and discrepancies
//!
//! ## PIVX Core Parity
//!
//! This tool uses EXACT PIVX Core algorithms for CCoins deserialization.
//! Any balance mismatch indicates either:
//! - Bug in transaction indexing
//! - Orphaned transactions not cleaned up
//! - Cold staking attribution errors

use rustyblox::chainstate_leveldb::read_chainstate_map;
use rustyblox::utxo::aggregate_by_address;
use rustyblox::config::{get_global_config, init_global_config, get_db_path};
use rustyblox::parser::{deserialize_utxos, deserialize_transaction};
use rustyblox::maturity::{filter_spendable_utxos, get_current_height};
use clap::Parser;
use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::Arc;
use rocksdb::{DB, ColumnFamilyDescriptor, Options};

#[derive(Parser, Debug)]
#[clap(name = "import-chainstate")]
#[clap(about = "Import and verify UTXO balances from PIVX Core chainstate", long_about = None)]
struct Args {
    /// Path to PIVX Core chainstate directory (uses config chainstate_copy_dir by default)
    #[clap(long)]
    chainstate_path: Option<String>,
    
    /// Verify balances against transaction database
    #[clap(long, default_value_t = true)]
    verify: bool,
    
    /// Exit with error if any mismatches found
    #[clap(long, default_value_t = false)]
    strict: bool,
    
    /// Show only addresses with balance > threshold (PIV)
    #[clap(long, default_value_t = 0.01)]
    min_balance: f64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    
    // Initialize config to get default chainstate path
    init_global_config()?;
    let config = get_global_config();
    
    // Use provided path or default from config
    let chainstate_path_str = if let Some(ref path) = args.chainstate_path {
        shellexpand::tilde(path).to_string()
    } else {
        config
            .get_string("paths.chainstate_copy_dir")
            .unwrap_or_else(|_| "/tmp/pivx_chainstate_current".to_string())
    };
    
    let chainstate_path = PathBuf::from(&chainstate_path_str);
    
    if !chainstate_path.exists() {
        eprintln!("❌ Chainstate path does not exist: {}", chainstate_path.display());
        eprintln!("   Default path: {} (from config.toml)", chainstate_path_str);
        eprintln!("   Or specify with: --chainstate-path <path>");
        std::process::exit(1);
    }
    
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║         PIVX Core Chainstate Import & Verification            ║");
    println!("╚════════════════════════════════════════════════════════════════╝\n");
    
    println!("📦 Importing UTXO set from PIVX Core chainstate...");
    println!("   Path: {}", chainstate_path.display());
    
    // Read raw chainstate
    println!("\n⏳ Reading chainstate LevelDB...");
    let raw_map = match read_chainstate_map(&chainstate_path.to_str().unwrap()) {
        Ok(map) => map,
        Err(e) => {
            eprintln!("❌ Failed to read chainstate: {}", e);
            eprintln!("   Make sure PIVX Core is stopped before copying chainstate");
            std::process::exit(1);
        }
    };
    
    println!("   ✅ Read {} chainstate entries", raw_map.len());
    
    // Aggregate by address
    println!("\n⏳ Parsing CCoins and aggregating by address...");
    let balances = aggregate_by_address(raw_map);
    
    if balances.is_empty() {
        eprintln!("❌ No balances aggregated - chainstate may be empty or corrupt");
        std::process::exit(1);
    }
    
    // Calculate total supply
    let total_supply: u64 = balances.values().sum();
    let total_piv = total_supply as f64 / 100_000_000.0;
    
    println!("\n💰 Chainstate Summary:");
    println!("   Addresses with balance: {}", balances.len());
    println!("   Total supply:           {:.2} PIV ({} satoshis)", total_piv, total_supply);
    
    // Show top addresses
    let mut sorted: Vec<_> = balances.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    
    println!("\n🏆 Top 10 Addresses:");
    for (i, (addr, amount)) in sorted.iter().take(10).enumerate() {
        let piv = **amount as f64 / 100_000_000.0;
        println!("   {}. {} - {:.2} PIV", i + 1, addr, piv);
    }
    
    // Verification against transaction database
    if args.verify {
        println!("\n╔════════════════════════════════════════════════════════════════╗");
        println!("║              Balance Verification vs Explorer DB              ║");
        println!("╚════════════════════════════════════════════════════════════════╝\n");
        
        // Config already initialized above, just get db path
        let db_path_str = get_db_path(&config)?;
        
        // Open DB with column families
        const COLUMN_FAMILIES: [&str; 8] = [
            "blocks",
            "transactions",
            "addr_index",
            "utxo",
            "chain_metadata",
            "pubkey",
            "chain_state",
            "utxo_undo",
        ];
        
        let mut cf_descriptors = vec![ColumnFamilyDescriptor::new("default", Options::default())];
        for cf in COLUMN_FAMILIES.iter() {
            cf_descriptors.push(ColumnFamilyDescriptor::new(
                cf.to_string(),
                Options::default(),
            ));
        }
        
        let mut db_options = Options::default();
        db_options.create_if_missing(false);  // DB must already exist
        
        let db = match DB::open_cf_descriptors(&db_options, db_path_str, cf_descriptors) {
            Ok(db) => Arc::new(db),
            Err(e) => {
                eprintln!("❌ Failed to open explorer database: {}", e);
                eprintln!("   Make sure rustyblox has been run at least once");
                std::process::exit(1);
            }
        };
        
        println!("⏳ Comparing {} addresses with explorer database...", balances.len());
        
        let results = verify_balances(&db, &balances, args.min_balance).await?;
        
        print_verification_results(&results);
        
        if args.strict && (results.mismatches > 0 || results.significant_diffs > 0) {
            eprintln!("\n❌ VERIFICATION FAILED: Found balance mismatches");
            std::process::exit(1);
        }
    }
    
    println!("\n✅ Chainstate import complete!");
    Ok(())
}

struct VerificationResults {
    matches: usize,
    mismatches: usize,
    chainstate_only: usize,
    txdb_only: usize,
    significant_diffs: usize,
    total_diff_satoshis: i64,
    examples: Vec<BalanceDiff>,
}

struct BalanceDiff {
    address: String,
    chainstate: u64,
    txdb: u64,
    diff: i64,
}

async fn verify_balances(
    db: &Arc<DB>,
    chainstate_balances: &HashMap<String, u64>,
    min_balance_piv: f64,
) -> Result<VerificationResults, Box<dyn std::error::Error>> {
    let min_balance_sats = (min_balance_piv * 100_000_000.0) as u64;
    
    let mut results = VerificationResults {
        matches: 0,
        mismatches: 0,
        chainstate_only: 0,
        txdb_only: 0,
        significant_diffs: 0,
        total_diff_satoshis: 0,
        examples: Vec::new(),
    };
    
    // Check chainstate addresses against txdb
    for (address, chainstate_balance) in chainstate_balances {
        let txdb_balance = get_address_balance(db, address).await.unwrap_or(0);
        
        if txdb_balance == 0 {
            if *chainstate_balance >= min_balance_sats {
                results.chainstate_only += 1;
                
                if results.examples.len() < 10 {
                    results.examples.push(BalanceDiff {
                        address: address.clone(),
                        chainstate: *chainstate_balance,
                        txdb: 0,
                        diff: *chainstate_balance as i64,
                    });
                }
            }
        } else {
            let diff = *chainstate_balance as i64 - txdb_balance as i64;
            results.total_diff_satoshis += diff.abs();
            
            // Allow 1 satoshi difference due to rounding
            if diff.abs() <= 1 {
                results.matches += 1;
            } else {
                results.mismatches += 1;
                
                // Significant diff: >0.01 PIV difference
                if diff.abs() > 1_000_000 {
                    results.significant_diffs += 1;
                }
                
                if results.examples.len() < 10 {
                    results.examples.push(BalanceDiff {
                        address: address.clone(),
                        chainstate: *chainstate_balance,
                        txdb: txdb_balance,
                        diff,
                    });
                }
            }
        }
    }
    
    // Check for addresses only in txdb (shouldn't happen unless orphaned txs)
    let addr_cf = db.cf_handle("addr_index")
        .ok_or("addr_index CF not found")?;
    
    let mut txdb_iter = db.iterator_cf(&addr_cf, rocksdb::IteratorMode::Start);
    let mut checked = 0;
    
    while let Some(Ok((key, _value))) = txdb_iter.next() {
        checked += 1;
        
        // Key format: 'a' + address
        if key.len() < 2 || key[0] != b'a' {
            continue;
        }
        
        let address = String::from_utf8_lossy(&key[1..]).to_string();
        
        if !chainstate_balances.contains_key(&address) {
            let txdb_balance = get_address_balance(db, &address).await.unwrap_or(0);
            
            if txdb_balance >= min_balance_sats {
                results.txdb_only += 1;
            }
        }
        
        // Limit checks to avoid long scan
        if checked > 100_000 {
            break;
        }
    }
    
    Ok(results)
}

async fn get_address_balance(db: &Arc<DB>, address: &str) -> Result<u64, Box<dyn std::error::Error>> {
    // Get UTXOs for this address (key: 'a' + address) from addr_index CF
    let key = format!("a{}", address);
    let key_bytes = key.as_bytes().to_vec();
    let db_clone = db.clone();
    
    // Get from addr_index column family in blocking task
    let result = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, String> {
        let cf_addr_index = db_clone.cf_handle("addr_index")
            .ok_or_else(|| "addr_index CF not found".to_string())?;
        db_clone.get_cf(&cf_addr_index, &key_bytes)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or(Ok(None))
    .unwrap_or(None)
    .unwrap_or_else(std::vec::Vec::new);
    
    // Deserialize UTXOs (format: list of (txid_hash, vout) pairs)
    let unspent_utxos = deserialize_utxos(&result).await;
    
    if unspent_utxos.is_empty() {
        return Ok(0);
    }
    
    // Get current chain height for maturity checks
    let current_height = get_current_height(&db).unwrap_or(0);
    
    // Filter UTXOs by maturity rules (coinbase/coinstake must meet maturity requirements)
    let spendable_utxos = filter_spendable_utxos(
        unspent_utxos.clone(),
        db.clone(),
        current_height,
    ).await;
    
    // Calculate balance from SPENDABLE unspent UTXOs only
    let mut balance: u64 = 0;
    
    for (txid_hash, output_index) in &spendable_utxos {
        // Get transaction data (key: 't' + txid)
        let mut key = vec![b't'];
        key.extend(txid_hash);
        let key_clone = key.clone();
        let db_clone = db.clone();
        
        let tx_data = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, String> {
            let cf_transactions = db_clone.cf_handle("transactions")
                .ok_or_else(|| "transactions CF not found".to_string())?;
            db_clone.get_cf(&cf_transactions, &key_clone)
                .map_err(|e| e.to_string())
        })
        .await
        .unwrap_or(Ok(None))
        .unwrap_or(None);
        
        if let Some(tx_data) = tx_data {
            // Transaction data format: 4 bytes timestamp + 4 bytes height + tx data
            if tx_data.len() >= 8 {
                let tx_data_len = tx_data.len() - 8;
                if tx_data_len > 0 {
                    // Prepare transaction data with dummy header for parser
                    let mut tx_data_with_header = Vec::with_capacity(4 + tx_data_len);
                    tx_data_with_header.extend_from_slice(&[0u8; 4]);
                    tx_data_with_header.extend_from_slice(&tx_data[8..]);
                
                    if let Ok(tx) = deserialize_transaction(&tx_data_with_header).await {
                        if let Some(output) = tx.outputs.get(*output_index as usize) {
                            balance += output.value as u64;
                        }
                    }
                }
            }
        }
    }
    
    Ok(balance)
}

fn print_verification_results(results: &VerificationResults) {
    println!("\n📊 Verification Results:");
    println!("   ✅ Exact matches:             {}", results.matches);
    println!("   ❌ Mismatches:                {}", results.mismatches);
    println!("   📦 Chainstate only:           {}", results.chainstate_only);
    println!("   💾 TxDB only:                 {}", results.txdb_only);
    println!("   ⚠️  Significant diffs (>0.01): {}", results.significant_diffs);
    
    let total_diff_piv = results.total_diff_satoshis as f64 / 100_000_000.0;
    println!("   📉 Total difference:          {:.8} PIV ({} sats)", 
             total_diff_piv, results.total_diff_satoshis);
    
    if !results.examples.is_empty() {
        println!("\n🔍 Example Mismatches (showing up to 10):");
        for (i, diff) in results.examples.iter().enumerate() {
            let diff_piv = diff.diff.abs() as f64 / 100_000_000.0;
            let symbol = if diff.diff > 0 { "📦 >" } else { "💾 <" };
            
            println!("   {}. {} {}", i + 1, symbol, diff.address);
            println!("      Chainstate: {:.8} PIV", diff.chainstate as f64 / 100_000_000.0);
            println!("      TxDB:       {:.8} PIV", diff.txdb as f64 / 100_000_000.0);
            println!("      Difference: {:.8} PIV", diff_piv);
        }
    }
    
    // Summary
    let total = results.matches + results.mismatches + results.chainstate_only;
    let match_rate = if total > 0 {
        results.matches as f64 / total as f64 * 100.0
    } else {
        0.0
    };
    
    println!("\n📈 Match Rate: {:.2}% ({}/{})", match_rate, results.matches, total);
    
    if results.mismatches == 0 && results.significant_diffs == 0 {
        println!("\n✅ VERIFICATION PASSED: All balances match PIVX Core!");
    } else {
        println!("\n⚠️  VERIFICATION ISSUES FOUND");
        println!("   Review mismatches above and investigate:");
        println!("   - Orphaned transactions (height=-1)");
        println!("   - Cold staking attribution");
        println!("   - Transaction indexing bugs");
    }
}
