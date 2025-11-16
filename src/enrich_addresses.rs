/// Address Enrichment Module
/// 
/// Builds address index after fast_sync completes.
/// This reads all transactions and creates address -> [txids] mappings
/// without modifying the original transaction data.

use std::sync::Arc;
use std::collections::HashMap;
use rocksdb::DB;
use crate::types::AddressType;
use crate::parser::{deserialize_transaction, serialize_utxos};

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
    
    println!("📊 Scanning transactions...");

    // Build address map in memory first
    let mut address_map: HashMap<String, Vec<(Vec<u8>, u64)>> = HashMap::new();
    
    // Iterate through all transactions
    let iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);
    
    for item in iter {
        let (key, value) = item?;
        
        // Skip block transaction index keys (start with 'B')
        if key.first() == Some(&b'B') {
            continue;
        }
        
        // Deserialize transaction
        let tx = match deserialize_transaction(&value).await {
            Ok(tx) => tx,
            Err(_) => {
                // Skip corrupted transactions
                continue;
            }
        };
        
        // Extract txid from key (skip 't' prefix)
        let txid = key[1..].to_vec();
        
        // Index each output by address
        for output in &tx.outputs {
            for address_str in &output.address {
                if address_str.is_empty() || 
                   address_str == "Nonstandard" || 
                   address_str == "CoinBaseTx" ||
                   address_str == "CoinStakeTx" {
                    continue;
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
    
    // Write address mappings to database
    let mut batch = rocksdb::WriteBatch::default();
    let mut written = 0;
    
    for (address, utxos) in address_map {
        let mut key = vec![b'a'];
        key.extend_from_slice(address.as_bytes());
        
        let serialized_utxos = serialize_utxos(&utxos).await;
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
    println!("   Unique addresses: {}", written);
    println!();
    
    Ok(())
}

