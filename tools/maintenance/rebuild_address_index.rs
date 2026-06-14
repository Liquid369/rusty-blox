/// Utility to clear and rebuild the address index with proper UTXO tracking
/// This removes the old address index (that counted all outputs including spent)
/// and rebuilds it with only UNSPENT outputs for accurate balances

use std::sync::Arc;
use rocksdb::{DB, Options, ColumnFamilyDescriptor};
use rustyblox::enrich_addresses::enrich_all_addresses;
use rustyblox::telemetry::{TelemetryConfig, init_tracing};

// jemalloc (see src/main.rs) — the #[global_allocator] must be set in each binary
// root. This is the binary that runs enrichment in isolation for measurement.
#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The harness has no tracing subscriber otherwise, so enrich_all_addresses'
    // info! lines (ENRICH_TIMING + cache/db metrics) are dropped. Honors RUST_LOG.
    let _ = init_tracing(TelemetryConfig::default());
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║      REBUILD ADDRESS INDEX WITH UTXO TRACKING      ║");
    println!("╚════════════════════════════════════════════════════╝\n");

    let db_path = std::env::var("DB_PATH")
        .unwrap_or_else(|_| "data/blocks.db".to_string());
    
    println!("📂 Opening database: {}", db_path);
    
    // Open database with all column families
    let mut opts = Options::default();
    opts.create_if_missing(false);
    opts.create_missing_column_families(false);
    
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
    
    println!("✅ Database opened successfully\n");
    
    // Step 1: Clear existing address index
    println!("🗑️  Step 1: Clearing old address index...");
    let cf_addr_index = db.cf_handle("addr_index")
        .ok_or("addr_index CF not found")?;
    
    let mut delete_count = 0;
    let iter = db.iterator_cf(&cf_addr_index, rocksdb::IteratorMode::Start);
    
    for item in iter {
        let (key, _) = item?;
        db.delete_cf(&cf_addr_index, &key)?;
        delete_count += 1;
        
        if delete_count % 100000 == 0 {
            println!("   Deleted {} address entries...", delete_count);
        }
    }
    
    println!("   ✅ Deleted {} old address index entries\n", delete_count);
    
    // Step 2: Rebuild with proper UTXO tracking
    println!("🔨 Step 2: Rebuilding address index with UTXO tracking...");
    println!("   This will process all transactions twice:");
    println!("   - Pass 1: Identify spent outputs");
    println!("   - Pass 2: Index only UNSPENT outputs\n");
    
    enrich_all_addresses(db.clone()).await?;
    
    // Mark address index as complete
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    db.put_cf(&cf_state, b"address_index_complete", [1u8])?;
    
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║     ✅ ADDRESS INDEX REBUILD COMPLETE! ✅          ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    println!("The address index now contains only UNSPENT outputs.");
    println!("Balances should now be accurate!\n");
    
    Ok(())
}
