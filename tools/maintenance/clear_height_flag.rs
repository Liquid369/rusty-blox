/// Clear height resolution flags to force re-validation
/// 
/// This will cause the next rustyblox startup to re-validate all transaction heights

use rocksdb::{DB, Options, ColumnFamilyDescriptor};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = std::env::var("DB_PATH")
        .unwrap_or_else(|_| "./data/blocks.db".to_string());
    
    println!("📂 Opening database: {}", db_path);
    
    let mut opts = Options::default();
    opts.create_if_missing(false);
    
    let cfs = vec![
        ColumnFamilyDescriptor::new("default", Options::default()),
        ColumnFamilyDescriptor::new("chain_metadata", Options::default()),
    ];
    
    let db = DB::open_cf_descriptors(&opts, &db_path, cfs)?;
    
    let cf = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    db.delete_cf(&cf, b"height_resolution_complete")?;
    db.delete_cf(&cf, b"repair_complete")?;
    
    println!("✅ Cleared height_resolution_complete and repair_complete flags");
    println!("   Next rustyblox startup will re-validate all transaction heights");
    println!("   This will fix the 39 orphaned UTXOs with height=0\n");
    
    Ok(())
}
