use rocksdb::DB;
use std::sync::Arc;
use std::env;
use futures::executor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: diagnose-tx <txid_hex>");
        std::process::exit(1);
    }
    let txid_hex = &args[1];
    if txid_hex.len() != 64 {
        eprintln!("TXID must be 64 hex characters");
        std::process::exit(1);
    }

    let db_path = env::var("DB_PATH")
        .unwrap_or_else(|_| "/Users/liquid/Projects/rusty-blox/data/blocks.db".to_string());

    println!("Opening DB: {}", db_path);

    let mut cf_opts = rocksdb::Options::default();
    cf_opts.create_if_missing(false);

    let cf_names = vec!["default", "blocks", "transactions", "addr_index", "utxo", 
                        "chain_metadata", "pubkey", "chain_state", "utxo_undo"];

    let db = Arc::new(DB::open_cf_for_read_only(&cf_opts, &db_path, &cf_names, false)?);
    let cf_transactions = db.cf_handle("transactions").ok_or("transactions CF not found")?;

    let txid_bytes = hex::decode(txid_hex)?;
    let reversed: Vec<u8> = txid_bytes.iter().rev().cloned().collect();

    let mut key = vec![b't'];
    key.extend_from_slice(&reversed);

    let value = match db.get_cf(&cf_transactions, &key)? {
        Some(v) => v,
        None => {
            eprintln!("Transaction not found in DB for txid {}", txid_hex);
            std::process::exit(2);
        }
    };

    println!("Found transaction data: {} bytes", value.len());

    // Build parser input (prepend 4-byte dummy header as parser expects)
    let mut tx_data_with_header = vec![0u8; 4];
    if value.len() > 8 {
        tx_data_with_header.extend_from_slice(&value[8..]);
    } else {
        eprintln!("Transaction data too small to parse");
        std::process::exit(3);
    }

    // Use project parser to deserialize synchronously
    use rustyblox::parser::deserialize_transaction;

    let tx_res = executor::block_on(deserialize_transaction(&tx_data_with_header));
    match tx_res {
        Ok(tx) => {
            println!("Parsed TXID: {}", tx.txid);
            println!("Version: {}  Outputs: {}  Inputs: {}", tx.version, tx.outputs.len(), tx.inputs.len());
            for out in tx.outputs.iter() {
                println!("\nVOUT {}: value: {}", out.index, out.value);
                println!("  scriptPubKey: {}", hex::encode(&out.script_pubkey.script));
                println!("  detected addresses: {:?}", out.address);
            }
        }
        Err(e) => {
            eprintln!("Failed to parse transaction: {:?}", e);
            std::process::exit(4);
        }
    }

    Ok(())
}
