use rustyblox::config::{get_global_config, init_global_config};
use rustyblox::sync::run_sync_service;
use rustyblox::mempool::{MempoolState, run_mempool_monitor};
use rustyblox::websocket::{EventBroadcaster, ws_blocks_handler, ws_transactions_handler, ws_mempool_handler};
use rustyblox::block_detail::block_detail_v2;
use rustyblox::cache::CacheManager;
use rustyblox::api::{
    // Root handlers
    api_handler, root_handler,
    // Network module
    status_v2, health_check_v2, money_supply_v2, cache_stats_v2,
    // Blocks module
    block_index_v2, block_v2, block_stats_v2,
    // Transactions module
    tx_v2, send_tx_v2, send_tx_post_v2,
    // Addresses module
    addr_v2, xpub_v2, utxo_v2,
    // Masternodes module
    mn_count_v2, mn_list_v2, relay_mnb_v2,
    // Governance module
    budget_info_v2, budget_votes_v2, budget_projection_v2,
    // Search module
    search_v2, mempool_v2, mempool_tx_v2,
};
use rustyblox::types::MyError;

use std::sync::Arc;
use rocksdb::{DB, ColumnFamilyDescriptor, Options};
use axum::{Router, routing::{get, post}};
use tower_http::cors::{CorsLayer, Any};
use tokio::sync::Mutex as TokioMutex;
use std::net::SocketAddr;
use std::time::Duration;
use std::path::PathBuf;
use lazy_static::lazy_static;

const COLUMN_FAMILIES: [&str; 8] = [
    "blocks",
    "transactions",
    "addr_index",
    "utxo",
    "chain_metadata",
    "pubkey",
    "chain_state",
    "utxo_undo",  // Spent UTXO tracking for reorg handling and input value calculation
];

lazy_static! {
    static ref DB_MUTEX: TokioMutex<()> = TokioMutex::new(());
}

async fn start_web_server(db_arc: Arc<DB>, mempool_state: Arc<MempoolState>, broadcaster: Arc<EventBroadcaster>) {
    let config = get_global_config();
    
    // Initialize cache manager
    let cache_manager = Arc::new(CacheManager::new());
    
    // Get server configuration
    let server_host = config
        .get_string("server.host")
        .unwrap_or_else(|_| "0.0.0.0".to_string());
    let server_port: u16 = config
        .get_int("server.port")
        .unwrap_or(3005) as u16;
    
    // Configure CORS to allow requests from the frontend
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // We just want to mimic blockbook API endpoints and structure for compatibility
    let app = Router::new()
        .route("/", get(root_handler))
        .route("/api", get(api_handler))
        .route("/api/", get(status_v2))  // Same as /api/v2/status
        .route("/api/endpoint", get(api_handler))
        .route("/api/v2/status", get(status_v2))
        .route("/api/v2/health", get(health_check_v2))
        .route("/api/v2/cache/stats", get(cache_stats_v2))  // Cache statistics endpoint
        .route("/api/v2/search/{query}", get(search_v2))
        .route("/api/v2/mempool", get(mempool_v2))
        .route("/api/v2/mempool/{txid}", get(mempool_tx_v2))
        .route("/api/v2/block-index/{block_height}", get(block_index_v2))
        .route("/api/v2/block-detail/{block_height}", get(block_detail_v2))
        .route("/api/v2/block-stats/{count}", get(block_stats_v2))
        .route("/api/v2/tx/{txid}", get(tx_v2))
        .route("/api/v2/address/{address}", get(addr_v2))
        .route("/api/v2/xpub/{xpub}", get(xpub_v2))
        .route("/api/v2/utxo/{address}", get(utxo_v2))
        .route("/api/v2/block/{block_height}", get(block_v2))
        .route("/api/v2/sendtx/{hex_tx}", get(send_tx_v2))
        .route("/api/v2/sendtx", post(send_tx_post_v2))  // Blockbook-compatible POST endpoint
        .route("/api/v2/mncount", get(mn_count_v2))
        .route("/api/v2/mnlist", get(mn_list_v2))
        .route("/api/v2/moneysupply", get(money_supply_v2))
        .route("/api/v2/budgetinfo", get(budget_info_v2))
        .route("/api/v2/relaymnb/{hex_mnb}", get(relay_mnb_v2))
        .route("/api/v2/budgetvotes/{proposal_name}", get(budget_votes_v2))
        .route("/api/v2/budgetprojection", get(budget_projection_v2))
        .route("/api/v2/mnrawbudgetvote/{raw_vote_params}", get(api_handler))
        .route("/ws/blocks", get(ws_blocks_handler))
        .route("/ws/transactions", get(ws_transactions_handler))
        .route("/ws/mempool", get(ws_mempool_handler))
        .layer(cors)
        .layer(axum::extract::Extension(cache_manager))
        .layer(axum::extract::Extension(db_arc))
        .layer(axum::extract::Extension(mempool_state))
        .layer(axum::extract::Extension(broadcaster));

    // Parse host IP
    let host_parts: Vec<u8> = server_host
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    
    let addr = if host_parts.len() == 4 {
        SocketAddr::from(([host_parts[0], host_parts[1], host_parts[2], host_parts[3]], server_port))
    } else {
        SocketAddr::from(([0, 0, 0, 0], server_port))
    };
    
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("FATAL: Failed to bind to {}: {}", addr, e);
            eprintln!("       Check if port {} is already in use", server_port);
            std::process::exit(1);
        }
    };
    println!("Listening on {}", addr);
    
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("Web server error: {}", e);
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

    // Initialize cached column family handles
    // This validates all required CFs exist at startup
    println!("Initializing database column family handles...");
    let _db_handles = rustyblox::db_handles::DbHandles::new(Arc::clone(&db_arc))
        .map_err(|e| format!("Failed to initialize DB handles: {}", e))?;
    println!("✅ Database handles validated");

    let blk_dir_path = PathBuf::from(blk_dir);
    
    // Create shared state
    let mempool_state = Arc::new(MempoolState::new());
    let broadcaster = Arc::new(EventBroadcaster::new());
    
    // Spawn mempool monitor service (can start early)
    let mempool_clone = Arc::clone(&mempool_state);
    tokio::spawn(async move {
        if let Err(e) = run_mempool_monitor(mempool_clone, 10).await {
            eprintln!("Mempool monitor error: {}", e);
        }
    });

    // Start sync service in background only if enabled in config.
    // If `sync.auto_start` is false, we skip the sync phase and start the web
    // server immediately in read-only mode. This prevents accidental long-running
    // block processing when the operator only wanted to run lightweight tasks.
    let auto_start_sync = config.get_bool("sync.auto_start").unwrap_or(true);
    // Prepare API clones now so they are available regardless of sync mode
    let api_db = Arc::clone(&db_arc);
    if auto_start_sync {
        println!("\n🔄 Starting blockchain sync...");
        let sync_db = Arc::clone(&db_arc);
        let sync_broadcaster = Arc::clone(&broadcaster);
        let blk_dir_clone = blk_dir_path.clone();

        tokio::task::spawn_blocking(move || {
            // Use tokio runtime for async operations
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                if let Err(e) = run_sync_service(blk_dir_clone, sync_db, Some(sync_broadcaster)).await {
                    eprintln!("Sync service error: {}", e);
                }
            });
        });

    // Wait for minimum viable sync before starting web server
    // Check every 2 seconds if we have at least 1000 blocks indexed
        loop {
            let cf_state = match api_db.cf_handle("chain_state") {
                Some(cf) => cf,
                None => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };

            match api_db.get_cf(&cf_state, b"sync_height") {
                Ok(Some(bytes)) if bytes.len() == 4 => {
                    let height = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    if height >= 1000 {
                        println!("\n✅ Minimum viable data indexed (height: {})", height);
                        println!("   Starting web server...\n");
                        break;
                    }
                    println!("📊 Indexing in progress (height: {})... waiting for minimum 1000 blocks", height);
                }
                _ => {
                    println!("📊 Indexing in progress... waiting for initial data");
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    } else {
        println!("⚠️  sync.auto_start=false - skipping sync service and starting web server in read-only mode");
    }
    
    // NOW start web server (data is ready)
    let api_mempool = Arc::clone(&mempool_state);
    let api_broadcaster = Arc::clone(&broadcaster);
    start_web_server(api_db, api_mempool, api_broadcaster).await;

    Ok(())
}