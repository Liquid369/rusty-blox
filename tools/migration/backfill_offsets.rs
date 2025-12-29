/// Backfill offset mappings from PIVX LevelDB into existing RocksDB
/// 
/// This tool reads offset data from PIVX's block index and stores it in RocksDB
/// without rebuilding the entire database.

use std::path::{Path, PathBuf};
use std::fs;
use std::io;
use rocksdb::{DB, Options as RocksOptions};

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║        BACKFILL OFFSET MAPPINGS TO ROCKSDB        ║");
    println!("╚════════════════════════════════════════════════════╝\n");
    
    // Get PIVX block index path
    let home = std::env::var("HOME").expect("HOME not set");
    let pivx_blocks_index = PathBuf::from(format!("{}/Library/Application Support/PIVX/blocks/index", home));
    
    println!("📍 Source: PIVX block index");
    println!("   Path: {}", pivx_blocks_index.display());
    
    if !pivx_blocks_index.exists() {
        eprintln!("❌ PIVX block index not found!");
        std::process::exit(1);
    }
    
    // Copy to temp location to avoid locks
    let temp_dir = std::env::temp_dir().join("pivx_index_backfill");
    println!("\n📋 Creating temporary copy (to avoid locks)...");
    println!("   Temp path: {}", temp_dir.display());
    
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }
    
    copy_dir_all(&pivx_blocks_index, &temp_dir)?;
    println!("✅ Copy complete");
    
    // Read canonical chain from LevelDB
    println!("\n📖 Reading canonical chain from PIVX block index...");
    let canonical_chain = rustyblox::leveldb_index::build_canonical_chain_from_leveldb(
        temp_dir.to_str().unwrap()
    )?;
    
    println!("✅ Loaded {} blocks from LevelDB", canonical_chain.len());
    
    // Count blocks with offsets
    let mut blocks_with_offsets = 0;
    let mut blocks_without_offsets = 0;
    
    for (_height, _hash, file, pos) in &canonical_chain {
        if file.is_some() && pos.is_some() {
            blocks_with_offsets += 1;
        } else {
            blocks_without_offsets += 1;
        }
    }
    
    let percentage = (blocks_with_offsets as f64 / canonical_chain.len() as f64) * 100.0;
    
    println!("\n📊 Offset Data Availability:");
    println!("   Blocks WITH offsets:    {} ({:.2}%)", blocks_with_offsets, percentage);
    println!("   Blocks WITHOUT offsets: {} ({:.2}%)", blocks_without_offsets, 100.0 - percentage);
    
    if blocks_with_offsets == 0 {
        eprintln!("\n❌ ERROR: No offset data found in PIVX block index!");
        eprintln!("   Please run: pivxd -reindex");
        std::process::exit(1);
    }
    
    // Open RocksDB
    let db_path = "data";
    println!("\n📂 Opening RocksDB database...");
    println!("   Path: {}", db_path);
    
    let mut opts = RocksOptions::default();
    opts.create_if_missing(false);
    opts.create_missing_column_families(false);
    
    let cf_names = vec![
        "blocks",
        "transactions",
        "chain_metadata",
        "addr_index",
        "utxo",
        "pubkey",
        "chain_state",
    ];
    
    let db = DB::open_cf(&opts, db_path, &cf_names)?;
    println!("✅ Database opened");
    
    // Get chain_metadata CF
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    // Store offset mappings
    println!("\n📝 Writing offset mappings to RocksDB...");
    println!("   Format: 'o' + block_hash (33 bytes) → file(4) + pos(8) (12 bytes)");
    
    let mut stored = 0;
    let mut skipped = 0;
    
    for (height, hash, opt_file, opt_pos) in &canonical_chain {
        if let (Some(file_num), Some(data_pos)) = (opt_file, opt_pos) {
            // Key: 'o' + internal_hash (33 bytes total)
            let mut off_key = vec![b'o'];
            off_key.extend_from_slice(hash);  // hash is already in internal (little-endian) format
            
            // Value: file_num(u32) + data_pos(u64) = 12 bytes
            let mut buf = Vec::with_capacity(12);
            buf.extend_from_slice(&(*file_num as u32).to_le_bytes());
            buf.extend_from_slice(&{ *data_pos }.to_le_bytes());
            
            db.put_cf(&cf_metadata, &off_key, &buf)?;
            stored += 1;
            
            if stored % 100_000 == 0 {
                println!("   Progress: {} offset mappings written...", stored);
            }
        } else {
            skipped += 1;
        }
    }
    
    println!("\n✅ Backfill complete!");
    println!("   Offset mappings written: {}", stored);
    println!("   Blocks without offsets:  {}", skipped);
    println!("   Coverage: {:.2}%", (stored as f64 / canonical_chain.len() as f64) * 100.0);
    
    // Clean up temp directory
    fs::remove_dir_all(&temp_dir)?;
    println!("\n✅ Cleanup complete");
    
    // Verify by counting offset keys
    println!("\n🔍 Verifying offset mappings...");
    let iter = db.iterator_cf(&cf_metadata, rocksdb::IteratorMode::Start);
    
    let mut offset_count = 0;
    for item in iter {
        let (key, value) = item?;
        if key.len() == 33 && key[0] == b'o' && value.len() == 12 {
            offset_count += 1;
        }
    }
    
    println!("✅ Verification complete: {} offset mappings found in database", offset_count);
    
    if offset_count != stored {
        println!("⚠️  Warning: Stored {} but found {} in verification", stored, offset_count);
    }
    
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║                 SUCCESS!                           ║");
    println!("╚════════════════════════════════════════════════════╝");
    println!("\nYou can now use offset-based block reading with:");
    println!("  ./target/release/test_offset_indexer ~/Library/Application\\ Support/PIVX/blocks");
    
    Ok(())
}
