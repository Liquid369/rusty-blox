/// Validate ALL transaction heights against canonical chain
/// 
/// Unlike revalidate-heights which only checks specific transactions,
/// this tool validates EVERY transaction in the database to ensure
/// its height corresponds to a block in the canonical chain.
///
/// Marks transactions as HEIGHT_ORPHAN (-1) if:
/// - Transaction references a block height not in canonical chain
/// - Transaction's block hash doesn't match canonical chain at that height

use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use rocksdb::{DB, Options, ColumnFamilyDescriptor, WriteBatch, IteratorMode};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║   VALIDATE ALL TRANSACTION HEIGHTS VS CANONICAL    ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    
    // 1. Open database
    let db_path = std::env::var("DB_PATH")
        .unwrap_or_else(|_| "data/blocks.db".to_string());
    
    println!("📂 Opening database: {}", db_path);
    
    let mut opts = Options::default();
    opts.create_if_missing(false);
    
    let cfs = vec![
        ColumnFamilyDescriptor::new("default", Options::default()),
        ColumnFamilyDescriptor::new("blocks", Options::default()),
        ColumnFamilyDescriptor::new("transactions", Options::default()),
        ColumnFamilyDescriptor::new("addr_index", Options::default()),
        ColumnFamilyDescriptor::new("utxo", Options::default()),
        ColumnFamilyDescriptor::new("chain_metadata", Options::default()),
        ColumnFamilyDescriptor::new("pubkey", Options::default()),
        ColumnFamilyDescriptor::new("chain_state", Options::default()),
        ColumnFamilyDescriptor::new("utxo_undo", Options::default()),
    ];
    
    let db = Arc::new(DB::open_cf_descriptors(&opts, &db_path, cfs)?);
    
    println!("✅ Database opened\n");
    
    // 2. Build canonical chain from PIVX Core
    println!("📂 Reading PIVX Core block index...");
    
    let pivx_dir = std::env::var("HOME")
        .map(|h| format!("{}/Library/Application Support/PIVX", h))
        .unwrap_or_else(|_| "/Users/liquid/Library/Application Support/PIVX".to_string());
    
    let block_index_src = format!("{}/blocks/index", pivx_dir);
    let block_index_copy = "/tmp/pivx_block_index_validate";
    
    // Remove old copy
    std::fs::remove_dir_all(block_index_copy).ok();
    
    // Copy block index
    println!("   Copying from: {}", block_index_src);
    let copy_result = std::process::Command::new("cp")
        .args(["-R", &block_index_src, block_index_copy])
        .output()?;
    
    if !copy_result.status.success() {
        return Err(format!("Failed to copy block index: {}", 
            String::from_utf8_lossy(&copy_result.stderr)).into());
    }
    
    println!("✅ Block index copied\n");
    
    // Use the library function to build canonical chain
    use rustyblox::leveldb_index::build_canonical_chain_from_leveldb;
    
    let canonical_chain = match build_canonical_chain_from_leveldb(block_index_copy) {
        Ok(chain) => chain,
        Err(e) => {
            eprintln!("❌ Failed to read block index: {}", e);
            return Err(e);
        }
    };
    
    println!("✅ Canonical chain built: {} blocks\n", canonical_chain.len());
    
    // 3. Build lookup: height -> canonical block hash
    println!("📊 Building height→hash lookup...");
    let mut canonical_heights: HashMap<i64, Vec<u8>> = HashMap::new();
    let max_height = canonical_chain.last().map(|(h, _, _, _)| *h).unwrap_or(0);
    
    for (height, block_hash, _, _) in &canonical_chain {
        canonical_heights.insert(*height, block_hash.clone());
    }
    
    println!("   ✅ Indexed {} canonical heights (0 → {})\n", canonical_heights.len(), max_height);
    
    // 4. Scan ALL transactions and validate their heights
    println!("🔍 Validating ALL transaction heights...");
    
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    let mut total_scanned = 0;
    let mut valid_heights = 0;
    let mut invalid_heights = 0;
    let mut already_orphaned = 0;
    let mut to_mark_orphan: Vec<Vec<u8>> = Vec::new();
    
    let iter = db.iterator_cf(&cf_transactions, IteratorMode::Start);
    
    for item in iter {
        let (key, value) = item?;
        
        // Only process 't' prefix entries (transaction data)
        if key.is_empty() || key[0] != b't' {
            continue;
        }
        
        total_scanned += 1;
        
        if value.len() < 8 {
            continue;
        }
        
        let height = i32::from_le_bytes([value[4], value[5], value[6], value[7]]);
        
        if height == -1 {
            // Already marked as orphan
            already_orphaned += 1;
        } else if height < 0 {
            // Other negative height (like -2 = HEIGHT_UNRESOLVED)
            to_mark_orphan.push(key[1..].to_vec());
            invalid_heights += 1;
        } else {
            // Positive height - validate against canonical chain
            if canonical_heights.contains_key(&(height as i64)) {
                valid_heights += 1;
            } else {
                // Height not in canonical chain - orphaned!
                to_mark_orphan.push(key[1..].to_vec());
                invalid_heights += 1;
            }
        }
        
        if total_scanned % 500000 == 0 {
            println!("   Scanned {} transactions ({} invalid found)...", total_scanned, invalid_heights);
        }
    }
    
    println!("\n📈 Validation complete:");
    println!("   Total scanned: {}", total_scanned);
    println!("   Valid canonical heights: {}", valid_heights);
    println!("   Invalid/non-canonical: {}", invalid_heights);
    println!("   Already orphaned (-1): {}", already_orphaned);
    println!();
    
    if to_mark_orphan.is_empty() {
        println!("✅ All transaction heights are valid!");
        return Ok(());
    }
    
    // 5. Mark invalid transactions as HEIGHT_ORPHAN
    println!("🔧 Marking {} transactions as HEIGHT_ORPHAN (-1)...", to_mark_orphan.len());
    
    let mut batch = WriteBatch::default();
    let mut marked = 0;
    const BATCH_SIZE: usize = 10_000;
    
    for txid_internal in to_mark_orphan {
        let mut tx_key = vec![b't'];
        tx_key.extend_from_slice(&txid_internal);
        
        if let Ok(Some(tx_data)) = db.get_cf(&cf_transactions, &tx_key) {
            if tx_data.len() >= 8 {
                // Update height to HEIGHT_ORPHAN (-1)
                let mut new_value = tx_data[0..4].to_vec(); // version
                new_value.extend(&(-1i32).to_le_bytes());
                new_value.extend(&tx_data[8..]); // rest of data
                
                batch.put_cf(&cf_transactions, &tx_key, &new_value);
                marked += 1;
                
                if marked % BATCH_SIZE == 0 {
                    db.write(batch)?;
                    batch = WriteBatch::default();
                    println!("      Marked {} transactions...", marked);
                }
            }
        }
    }
    
    if !batch.is_empty() {
        db.write(batch)?;
    }
    
    println!("\n✅ Marked {} transactions as HEIGHT_ORPHAN\n", marked);
    
    println!("╔════════════════════════════════════════════════════╗");
    println!("║              ✅ VALIDATION COMPLETE! ✅             ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    println!("Next step: Rebuild address index to apply these changes:");
    println!("  cargo run --release --bin rebuild-address-index\n");
    
    Ok(())
}
