mod address;
mod api;
mod batch_writer;
mod block_detail;
mod blocks;
mod chain_state;
mod db_utils;
mod mempool;
mod monitor;
mod parallel;
mod parser;
mod search;
mod sync;
mod transactions;
mod config;
mod types;
mod websocket;

use crate::config::{get_global_config, init_global_config};
use crate::sync::run_sync_service;
use crate::mempool::{MempoolState, run_mempool_monitor};
use crate::websocket::{EventBroadcaster, ws_blocks_handler, ws_transactions_handler, ws_mempool_handler};
use crate::block_detail::block_detail_v2;
use crate::api::{
    api_handler, root_handler, block_index_v2, block_v2, tx_v2, addr_v2, xpub_v2, utxo_v2,
    send_tx_v2, mn_count_v2, mn_list_v2, money_supply_v2, budget_info_v2, relay_mnb_v2,
    status_v2, search_v2, mempool_v2, mempool_tx_v2,
};
use crate::types::MyError;

use std::sync::Arc;
use rocksdb::{DB, ColumnFamilyDescriptor, Options};
use axum::{Router, routing::get};
use tower_http::cors::{CorsLayer, Any};
use tokio::sync::Mutex as TokioMutex;
use std::net::SocketAddr;
use std::time::Duration;
use std::path::PathBuf;
use lazy_static::lazy_static;

const COLUMN_FAMILIES: [&str; 7] = [
    "blocks",
    "transactions",
    "addr_index",
    "utxo",
    "chain_metadata",
    "pubkey",
    "chain_state",
];

lazy_static! {
    static ref DB_MUTEX: TokioMutex<()> = TokioMutex::new(());
}

async fn start_web_server(db_arc: Arc<DB>, mempool_state: Arc<MempoolState>, broadcaster: Arc<EventBroadcaster>) {
    // Configure CORS to allow requests from the frontend
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/api", get(api_handler))
        .route("/api/endpoint", get(api_handler))
        .route("/api/v2/status", get(status_v2))
        .route("/api/v2/search/{query}", get(search_v2))
        .route("/api/v2/mempool", get(mempool_v2))
        .route("/api/v2/mempool/{txid}", get(mempool_tx_v2))
        .route("/api/v2/block-index/{block_height}", get(block_index_v2))
        .route("/api/v2/block-detail/{block_height}", get(block_detail_v2))
        .route("/api/v2/tx/{txid}", get(tx_v2))
        .route("/api/v2/address/{address}", get(addr_v2))
        .route("/api/v2/xpub/{xpub}", get(xpub_v2))
        .route("/api/v2/utxo/{address}", get(utxo_v2))
        .route("/api/v2/block/{block_height}", get(block_v2))
        .route("/api/v2/sendtx/{hex_tx}", get(send_tx_v2))
        .route("/api/v2/mncount", get(mn_count_v2))
        .route("/api/v2/mnlist", get(mn_list_v2))
        .route("/api/v2/moneysupply", get(money_supply_v2))
        .route("/api/v2/budgetinfo", get(budget_info_v2))
        .route("/api/v2/relaymnb/{hex_mnb}", get(relay_mnb_v2))
        .route("/api/v2/budgetvotes/{proposal_name}", get(api_handler))
        .route("/api/v2/budgetprojection", get(api_handler))
        .route("/api/v2/mnrawbudgetvote/{raw_vote_params}", get(api_handler))
        .route("/ws/blocks", get(ws_blocks_handler))
        .route("/ws/transactions", get(ws_transactions_handler))
        .route("/ws/mempool", get(ws_mempool_handler))
        .layer(cors)
        .layer(axum::extract::Extension(db_arc))
        .layer(axum::extract::Extension(mempool_state))
        .layer(axum::extract::Extension(broadcaster));

    let addr = SocketAddr::from(([0, 0, 0, 0], 3005));
    let listener = tokio::net::TcpListener::bind(addr).await.expect("Failed to bind");
    println!("Listening on {}", addr);
    axum::serve(listener, app)
        .await
        .expect("server failed");
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

    // RocksDB optimizations for high-throughput writes
    let mut db_options = Options::default();
    db_options.create_if_missing(true);
    db_options.create_missing_column_families(true);
    
    // Write buffer optimizations
    db_options.set_write_buffer_size(256 * 1024 * 1024); // 256MB write buffer
    db_options.set_max_write_buffer_number(4); // Allow up to 4 write buffers
    db_options.set_min_write_buffer_number_to_merge(2); // Merge after 2 buffers
    
    // File size and compaction
    db_options.set_target_file_size_base(256 * 1024 * 1024); // 256MB SST files
    db_options.set_level_zero_file_num_compaction_trigger(8); // Trigger compaction after 8 L0 files
    db_options.set_max_background_jobs(8); // Parallel background compaction/flush
    
    // Compression
    db_options.set_compression_type(rocksdb::DBCompressionType::Lz4); // Fast compression
    
    // Increase parallelism (8 cores)
    db_options.increase_parallelism(8);
    
    let db = DB::open_cf_descriptors(
        &db_options,
        db_path_str,
        cf_descriptors,
    )?;
    let db_arc = Arc::new(db);

    let blk_dir_path = PathBuf::from(blk_dir);
    
    // Create shared state
    let mempool_state = Arc::new(MempoolState::new());
    let broadcaster = Arc::new(EventBroadcaster::new());
    
    // Spawn web server in background task
    let api_db = Arc::clone(&db_arc);
    let api_mempool = Arc::clone(&mempool_state);
    let api_broadcaster = Arc::clone(&broadcaster);
    tokio::spawn(async move {
        start_web_server(api_db, api_mempool, api_broadcaster).await;
    });
    
    // Spawn mempool monitor service
    let mempool_clone = Arc::clone(&mempool_state);
    tokio::spawn(async move {
        if let Err(e) = run_mempool_monitor(mempool_clone, 10).await {
            eprintln!("Mempool monitor error: {}", e);
        }
    });

    // Give services time to start
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Run sync service (handles both initial and live sync)
    run_sync_service(blk_dir_path, Arc::clone(&db_arc), Some(broadcaster)).await?;

    Ok(())
}