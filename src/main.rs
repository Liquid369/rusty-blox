mod address;
mod api;
mod batch_writer;
mod blocks;
mod db_utils;
mod parser;
mod transactions;
mod config;
mod types;

use crate::config::{get_global_config, init_global_config};
use crate::db_utils::save_file_as_incomplete;
use crate::blocks::process_blk_file;
use crate::api::{
    api_handler, root_handler, block_index_v2, block_v2, tx_v2, addr_v2, xpub_v2, utxo_v2,
    send_tx_v2, mn_count_v2, mn_list_v2, money_supply_v2, budget_info_v2, relay_mnb_v2,
};
use crate::types::{MyError, AppState};

use std::sync::Arc;
use rocksdb::{DB, ColumnFamilyDescriptor, Options};
use axum::{Router, routing::get};
use tokio::sync::Mutex as TokioMutex;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::fs;
use std::path::PathBuf;
use lazy_static::lazy_static;

const PREFIX: [u8; 4] = [0x90, 0xc4, 0xfd, 0xe9];
const MAX_PAYLOAD_SIZE: usize = 10000;

const COLUMN_FAMILIES: [&str; 7] = [
    "blocks",
    "transactions",
    "addr_index",
    "utxo",
    "chain_metadata",
    "pubkey",
    "chain_state",
];

const BATCH_SIZE: usize = 3;

lazy_static! {
    static ref DB_MUTEX: TokioMutex<()> = TokioMutex::new(());
}

async fn process_file_chunks(
    entries: &[PathBuf],
    db_arc: Arc<DB>,
    state: AppState,
) -> Result<(), Box<dyn std::error::Error>> {
    // Process files sequentially (RocksDB column family handles aren't Send)
    // Performance optimization: Within-file transaction processing is already async
    for file_path in entries {
        if let Some(file_name) = file_path.file_name().and_then(|n| n.to_str()) {
            if file_name.starts_with("blk") && file_name.ends_with(".dat") {
                if let Err(e) = process_blk_file(state.clone(), file_path.clone(), db_arc.clone()).await {
                    eprintln!("Failed to process blk file: {}", e);
                    save_file_as_incomplete(&db_arc, &file_path).await?;
                }
            }
        }
    }

    Ok(())
}

async fn start_web_server(db_arc: Arc<DB>) {
    let app = Router::new()
        .route("/", get(root_handler))
        .route("/api", get(api_handler))
        .route("/api/endpoint", get(api_handler))
        .route("/api/v2/block-index/:block_height", get(block_index_v2))
        .route("/api/v2/tx/:txid", get(tx_v2))
        .route("/api/v2/address/:address", get(addr_v2))
        .route("/api/v2/xpub/:xpub", get(xpub_v2))
        .route("/api/v2/utxo/:address", get(utxo_v2))
        .route("/api/v2/block/:block_height", get(block_v2))
        .route("/api/v2/sendtx/:hex_tx", get(send_tx_v2))
        .route("/api/v2/mncount", get(mn_count_v2))
        .route("/api/v2/mnlist", get(mn_list_v2))
        .route("/api/v2/moneysupply", get(money_supply_v2))
        .route("/api/v2/budgetinfo", get(budget_info_v2))
        .route("/api/v2/relaymnb/:hex_mnb", get(relay_mnb_v2))
        .route("/api/v2/budgetvotes/:proposal_name", get(api_handler))
        .route("/api/v2/budgetprojection", get(api_handler))
        .route("/api/v2/mnrawbudgetvote/:raw_vote_params", get(api_handler))
        .layer(axum::extract::Extension(db_arc));

    let addr = SocketAddr::from(([0, 0, 0, 0], 3005));
    println!("Listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("server failed");
}

async fn process_files_loop(blk_dir: PathBuf, db_arc: Arc<DB>, state: AppState) {
    loop {
        let read_dir_result = fs::read_dir(&blk_dir).await;
        match read_dir_result {
            Ok(mut dir_entries) => {
                let mut entries = Vec::new();
                
                // Read all entries from directory
                while let Ok(Some(entry)) = dir_entries.next_entry().await {
                    entries.push(entry.path());
                }

                if let Err(e) = process_file_chunks(&entries, Arc::clone(&db_arc), state.clone()).await {
                    eprintln!("Error processing file chunks: {}", e);
                }
            },
            Err(e) => {
                eprintln!("Error reading directory: {}", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        // Sleep for a while before the next iteration...
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_global_config()?;
    let config = get_global_config();

    let worker_threads_str: String = config
        .get("server.worker_threads")
        .map_err(|e| format!("Error getting server.worker_threads: {}", e))?;
    let _worker_threads: usize = worker_threads_str
        .parse()
        .map_err(|_| "Invalid number for worker_threads")?;

    let db_path_str = config
        .get_string("paths.db_path")
        .map_err(|_| MyError::new("Missing db_path in config"))?;

    let blk_dir = config
        .get_string("paths.blk_dir")
        .map_err(|_| MyError::new("Missing blk_dir in config"))?;

    let mut cf_descriptors = vec![ColumnFamilyDescriptor::new("default", Options::default())];
    for cf in COLUMN_FAMILIES.iter() {
        cf_descriptors.push(ColumnFamilyDescriptor::new(
            cf.to_string(),
            Options::default(),
        ));
    }

    let mut db_options = Options::default();
    db_options.create_if_missing(true);
    db_options.create_missing_column_families(true);
    let db = DB::open_cf_descriptors(
        &db_options,
        db_path_str,
        cf_descriptors,
    )?;
    let db_arc = Arc::new(db);

    let blk_dir_path = PathBuf::from(blk_dir);

    let state = AppState {
        db: Arc::clone(&db_arc),
    };

    // Run file processing directly (not spawned) for testing transaction integration
    process_files_loop(blk_dir_path, Arc::clone(&db_arc), state).await;

    // Start web server
    start_web_server(Arc::clone(&db_arc)).await;

    Ok(())
}