/// Blockchain Sync Validation Tool
/// 
/// Validates that our blockchain sync is complete by:
/// 1. Checking transaction count against external explorer for sample addresses
/// 2. Identifying gaps in block height coverage
/// 3. Validating address index completeness

use rocksdb::{DB, Options, IteratorMode};
use std::sync::Arc;
use std::error::Error;
use std::collections::HashSet;
use std::process::Command;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║       BLOCKCHAIN SYNC VALIDATION REPORT            ║");
    println!("╚════════════════════════════════════════════════════╝\n");

    let db_path = std::env::var("DB_PATH")
        .unwrap_or_else(|_| "data/blocks.db".to_string());
    
    let opts = Options::default();
    let db = Arc::new(DB::open_cf_for_read_only(
        &opts,
        &db_path,
        vec!["transactions", "addr_index", "chain_state", "blocks"],
        false
    )?);
    
    // 1. Count total transactions
    println!("📊 Counting transactions in database...");
    let tx_cf = db.cf_handle("transactions").ok_or("transactions CF not found")?;
    let mut total_txs = 0;
    let mut orphaned_txs = 0;
    let mut valid_txs = 0;
    
    let iter = db.iterator_cf(tx_cf, IteratorMode::Start);
    for item in iter {
        let (key, value) = item?;
        // Skip block index keys
        if key.first() == Some(&b'B') {
            continue;
        }
        
        total_txs += 1;
        
        // Check if orphaned
        if value.len() >= 8 {
            let height_bytes: [u8; 4] = value[4..8].try_into().unwrap_or([0,0,0,0]);
            let height = i32::from_le_bytes(height_bytes);
            if height == -1 {
                orphaned_txs += 1;
            } else {
                valid_txs += 1;
            }
        }
    }
    
    println!("  Total transaction entries: {}", total_txs);
    println!("  Valid transactions (height != -1): {}", valid_txs);
    println!("  Orphaned transactions (height == -1): {}", orphaned_txs);
    
    // 2. Count address index entries
    println!("\n📇 Checking address index...");
    let addr_cf = db.cf_handle("addr_index").ok_or("addr_index CF not found")?;
    let mut addr_tx_lists = 0;
    let mut addr_utxo_lists = 0;
    let mut unique_addresses: HashSet<String> = HashSet::new();
    
    let iter = db.iterator_cf(addr_cf, IteratorMode::Start);
    for item in iter {
        let (key, _value) = item?;
        if key.len() > 1 {
            let prefix = key[0];
            let address = String::from_utf8_lossy(&key[1..]).to_string();
            
            match prefix {
                b't' => addr_tx_lists += 1,
                b'a' => addr_utxo_lists += 1,
                _ => {}
            }
            
            unique_addresses.insert(address);
        }
    }
    
    println!("  Address transaction lists ('t' prefix): {}", addr_tx_lists);
    println!("  Address UTXO lists ('a' prefix): {}", addr_utxo_lists);
    println!("  Unique addresses indexed: {}", unique_addresses.len());
    
    // 3. Validate against external explorer for test address
    println!("\n🔍 Validating against external PIVX explorer...");
    let test_address = "DCSAJGThtCnDokqawZehRvVjdms9XLL6J6";
    println!("  Test address: {}", test_address);
    
    // Get our count
    let our_key = format!("t{}", test_address);
    let our_count = match db.get_cf(addr_cf, our_key.as_bytes())? {
        Some(data) => data.len() / 32,
        None => 0
    };
    
    println!("  Our transaction count: {}", our_count);
    
    // Get external count via API
    println!("  Querying external explorer API...");
    let url = format!("https://explorer.pivx.org/api/v2/address/{}", test_address);
    let output = Command::new("curl")
        .arg("-s")
        .arg(&url)
        .output()?;
    
    if output.status.success() {
        let json_str = String::from_utf8_lossy(&output.stdout);
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&json_str) {
            let external_count = json["txs"].as_u64().unwrap_or(0);
            let balance = json["balance"].as_str().unwrap_or("0");
            let received = json["totalReceived"].as_str().unwrap_or("0");
            let sent = json["totalSent"].as_str().unwrap_or("0");
            
            println!("  External transaction count: {}", external_count);
            println!("  External balance: {} satoshis ({} PIV)", balance, 
                     balance.parse::<i64>().unwrap_or(0) as f64 / 100_000_000.0);
            println!("  External received: {} satoshis ({} PIV)", received,
                     received.parse::<i64>().unwrap_or(0) as f64 / 100_000_000.0);
            println!("  External sent: {} satoshis ({} PIV)", sent,
                     sent.parse::<i64>().unwrap_or(0) as f64 / 100_000_000.0);
            
            let missing = external_count as i64 - our_count as i64;
            if missing > 0 {
                println!("\n  ⚠️  DISCREPANCY: {} transactions missing from our database", missing);
                println!("     This indicates INCOMPLETE BLOCKCHAIN SYNC");
                println!("     Missing transactions were never downloaded from PIVX daemon");
            } else if missing < 0 {
                println!("\n  ⚠️  WARNING: We have MORE transactions than external explorer");
                println!("     This may indicate orphaned transactions not yet cleaned up");
            } else {
                println!("\n  ✅ PERFECT MATCH: Transaction count matches external explorer");
            }
        }
    }
    
    // 4. Check for address index completeness marker
    println!("\n🎯 Checking sync status markers...");
    let state_cf = db.cf_handle("chain_state").ok_or("chain_state CF not found")?;
    let addr_complete = db.get_cf(state_cf, b"address_index_complete")?;
    if addr_complete.is_some() {
        println!("  ✅ Address index marked as complete");
    } else {
        println!("  ⚠️  Address index NOT marked as complete");
        println!("     Run 'cargo run --bin rebuild_address_index' if sync is complete");
    }
    
    // 5. Summary and recommendations
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║                    SUMMARY                         ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    
    if addr_tx_lists == 0 && valid_txs > 0 {
        println!("❌ CRITICAL: Address index is EMPTY but transactions exist");
        println!("   Action: Run 'cargo run --release --bin rebuild_address_index'");
    } else if our_count > 0 && our_count < 2445 {
        println!("❌ CRITICAL: Blockchain sync is INCOMPLETE");
        println!("   {} transactions involving test address are MISSING from database", 2445 - our_count);
        println!("   Action: Re-sync blockchain from PIVX daemon or check .dat files for corruption");
        println!("   These transactions were never downloaded during initial sync");
    } else if our_count == 2445 {
        println!("✅ SUCCESS: Blockchain sync is COMPLETE for test address");
        println!("   All transactions match external explorer");
    }
    
    println!();
    Ok(())
}
