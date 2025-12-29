use std::env;
use rustyblox::leveldb_index::build_canonical_chain_from_leveldb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Accept path via first CLI argument or LEVELDB_PATH env var.
    let args: Vec<String> = env::args().collect();
    let copy_leveldb_path = if args.len() > 1 {
        args[1].clone()
    } else if let Ok(v) = env::var("LEVELDB_PATH") {
        v
    } else {
        "/tmp/pivx_index_current".to_string()
    };

    println!("Inspecting LevelDB at: {}", copy_leveldb_path);

    let chain = build_canonical_chain_from_leveldb(&copy_leveldb_path)?;

    let len = chain.len();
    println!("Canonical chain entries found: {}", len);

    if len > 0 {
    let first = &chain[0];
    let last = &chain[len - 1];
    println!("First: height {}", first.0);
    println!("Last: height {}", last.0);

        // Print sample of first 5 and last 5 entries
        println!("\nFirst 5 entries:");
        for (h, hash, _file, _pos) in chain.iter().take(5) {
            println!("  {}: {}", h, hex::encode(hash));
        }

        println!("\nLast 5 entries:");
        for (h, hash, _file, _pos) in chain.iter().rev().take(5).collect::<Vec<_>>().iter() {
            println!("  {}: {}", h, hex::encode(hash));
        }
    }

    Ok(())
}
