use rocksdb::{DB, Options};
use std::sync::Arc;
use rustyblox::config::{load_config, get_db_path};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config()?;
    let db_path = get_db_path(&config)?;
    let cf_names = vec!["default", "blocks", "transactions", "addr_index", "utxo", "chain_metadata", "pubkey", "chain_state"];
    let opts = Options::default();
    let db = Arc::new(DB::open_cf_for_read_only(&opts, db_path, &cf_names, false)?);
    
    let cf_transactions = db.cf_handle("transactions").unwrap();
    
    // The txid from block 1000 (display format)
    let txid_hex = "24d0f7c43987599a5a8d47a8b4dc8795ca3c499ffb3fbbe38713e06830113023";
    let txid_bytes = hex::decode(txid_hex)?;
    let reversed: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
    
    let mut key = vec![b't'];
    key.extend_from_slice(&reversed);
    
    let data = db.get_cf(&cf_transactions, &key)?.unwrap();
    
    println!("Transaction data length: {} bytes", data.len());
    println!("First 20 bytes: {}", hex::encode(&data[..20.min(data.len())]));
    
    let block_version = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let block_height = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    
    println!("Block version: {}", block_version);
    println!("Block height: {}", block_height);
    
    // Try parsing with dummy header
    let mut tx_data_with_header = vec![0u8; 4];
    tx_data_with_header.extend_from_slice(&data[8..]);
    
    println!("\nParsing transaction...");
    println!("Parser input length: {} bytes", tx_data_with_header.len());
    println!("First 20 bytes: {}", hex::encode(&tx_data_with_header[..20.min(tx_data_with_header.len())]));
    
    use rustyblox::parser::deserialize_transaction;
    match deserialize_transaction(&tx_data_with_header).await {
        Ok(tx) => {
            println!("\n✅ Transaction parsed successfully!");
            println!("TXID: {}", tx.txid);
            println!("Version: {}", tx.version);
            println!("Inputs: {}", tx.inputs.len());
            println!("Outputs: {}", tx.outputs.len());
            println!("Lock time: {}", tx.lock_time);
            
            for (i, input) in tx.inputs.iter().enumerate() {
                println!("\nInput {}:", i);
                if let Some(coinbase) = &input.coinbase {
                    println!("  Coinbase: {}", hex::encode(coinbase));
                } else if let Some(prevout) = &input.prevout {
                    println!("  Prev TX: {}", prevout.hash);
                    println!("  Prev Index: {}", prevout.n);
                }
                println!("  Sequence: {}", input.sequence);
            }
            
            for (i, output) in tx.outputs.iter().enumerate() {
                println!("\nOutput {}:", i);
                println!("  Value: {}", output.value);
                println!("  Address: {:?}", output.address);
                println!("  Script: {}", hex::encode(&output.script_pubkey.script));
            }
        }
        Err(e) => {
            println!("\n❌ Failed to parse: {:?}", e);
        }
    }
    
    Ok(())
}
