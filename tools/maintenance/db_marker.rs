//! One-off admin tool: read or clear chain_state markers (e.g. address_index_complete)
//! Usage:
//!   db-marker get <marker>
//!   db-marker clear <marker>
//! Operates on the db_path from config.toml in the current directory.

use rustyblox::config::{get_db_path, load_config};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if !(args.len() == 3 && matches!(args[1].as_str(), "get" | "clear"))
        && !(args.len() == 4 && args[1] == "set-height")
    {
        eprintln!(
            "usage: db-marker <get|clear> <marker-name> | db-marker set-height <marker-name> <i32>"
        );
        std::process::exit(2);
    }
    let action = args[1].as_str();
    let marker = args[2].as_bytes();

    let config = load_config()?;
    let db_path = get_db_path(&config)?;

    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(false);
    let cfs = rocksdb::DB::list_cf(&opts, &db_path)?;
    let db = Arc::new(rocksdb::DB::open_cf(&opts, &db_path, &cfs)?);
    let cf = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;

    match action {
        "get" => match db.get_cf(&cf, marker)? {
            Some(v) => println!("{} = {:?}", args[2], v),
            None => println!("{} = <absent>", args[2]),
        },
        "clear" => {
            db.delete_cf(&cf, marker)?;
            db.flush()?;
            println!("cleared {}", args[2]);
        }
        "set-height" => {
            let v: i32 = args[3].parse()?;
            db.put_cf(&cf, marker, v.to_le_bytes())?;
            db.flush()?;
            println!("set {} = {}", args[2], v);
        }
        _ => unreachable!(),
    }
    Ok(())
}
