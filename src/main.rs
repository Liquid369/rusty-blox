use rustyblox::config::{get_global_config, init_global_config};
use rustyblox::sync::run_sync_service;
use rustyblox::mempool::{MempoolState, run_mempool_monitor};
use rustyblox::websocket::{EventBroadcaster, ws_blocks_handler, ws_transactions_handler, ws_mempool_handler};
use rustyblox::block_detail::block_detail_v2;
use rustyblox::cache::CacheManager;
use rustyblox::telemetry::{TelemetryConfig, init_tracing};
use rustyblox::metrics;
use tracing::{error, warn, info};
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
    // Analytics module
    supply_analytics, transaction_analytics, staking_analytics,
    network_health_analytics, rich_list, wealth_distribution,
};
use rustyblox::types::MyError;

use std::sync::Arc;
use rocksdb::{DB, ColumnFamilyDescriptor, Options, Cache, BlockBasedOptions};
use axum::{Router, routing::{get, post}, response::IntoResponse};
use tower_http::cors::{CorsLayer, Any};
use tokio::sync::Mutex as TokioMutex;
use std::net::SocketAddr;
use std::time::Duration;
use std::path::PathBuf;
use lazy_static::lazy_static;
use tracing::info_span;

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

/// Prometheus metrics endpoint handler
async fn metrics_handler() -> impl IntoResponse {
    let metrics_output = metrics::gather_metrics();
    (
        axum::http::StatusCode::OK,
        [("Content-Type", "text/plain; version=0.0.4")],
        metrics_output,
    )
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
        .route("/api/v2/analytics/supply", get(supply_analytics))
        .route("/api/v2/analytics/transactions", get(transaction_analytics))
        .route("/api/v2/analytics/staking", get(staking_analytics))
        .route("/api/v2/analytics/network", get(network_health_analytics))
        .route("/api/v2/analytics/richlist", get(rich_list))
        .route("/api/v2/analytics/wealth-distribution", get(wealth_distribution))
        .route("/ws/blocks", get(ws_blocks_handler))
        .route("/ws/transactions", get(ws_transactions_handler))
        .route("/ws/mempool", get(ws_mempool_handler))
        .route("/metrics", get(metrics_handler))  // Prometheus metrics endpoint
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
            error!(addr = %addr, port = server_port, error = ?e, "FATAL: Failed to bind to address - check if port is already in use");
            std::process::exit(1);
        }
    };
    println!("Listening on {}", addr);
    
    if let Err(e) = axum::serve(listener, app).await {
        error!(error = ?e, "Web server error");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ========================================
    // STEP 1: Initialize Telemetry (FIRST!)
    // ========================================
    let telemetry_config = TelemetryConfig::default();
    init_tracing(telemetry_config)?;
    
    let _span = info_span!("service_start").entered();
    info!("Service starting");
    
    // ========================================
    // STEP 2: Initialize Metrics
    // ========================================
    metrics::init_metrics()?;
    info!("Metrics registry initialized");
    
    // ========================================
    // STEP 3: Load Configuration
    // ========================================
    init_global_config()?;
    let config = get_global_config();
    info!("Configuration loaded");

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

    // ========================================
    // RocksDB Configuration
    // ========================================
    
    info!("Configuring RocksDB with optimized settings");
    
    // Load tuning parameters from config (with defaults)
    let write_buffer_size_mb = config.get_int("rocksdb.write_buffer_size").unwrap_or(256);
    let max_write_buffer_number = config.get_int("rocksdb.max_write_buffer_number").unwrap_or(4) as i32;
    let min_write_buffer_number_to_merge = config.get_int("rocksdb.min_write_buffer_number_to_merge").unwrap_or(2) as i32;
    let block_cache_size_mb = config.get_int("rocksdb.block_cache_size").unwrap_or(512);
    let target_file_size_mb = config.get_int("rocksdb.target_file_size_base").unwrap_or(256);
    let max_open_files = config.get_int("rocksdb.max_open_files").unwrap_or(5000) as i32;
    let level0_file_num_compaction_trigger = config.get_int("rocksdb.level0_file_num_compaction_trigger").unwrap_or(8) as i32;
    let level0_slowdown_writes_trigger = config.get_int("rocksdb.level0_slowdown_writes_trigger").unwrap_or(20) as i32;
    let level0_stop_writes_trigger = config.get_int("rocksdb.level0_stop_writes_trigger").unwrap_or(36) as i32;
    let max_background_jobs = config.get_int("rocksdb.max_background_jobs").unwrap_or(8) as i32;
    let max_subcompactions = config.get_int("rocksdb.max_subcompactions").unwrap_or(4);
    
    let compression_type_str = config.get_string("rocksdb.compression_type").unwrap_or_else(|_| "lz4".to_string());
    let compression_type = match compression_type_str.to_lowercase().as_str() {
        "none" => rocksdb::DBCompressionType::None,
        "snappy" => rocksdb::DBCompressionType::Snappy,
        "zstd" => rocksdb::DBCompressionType::Zstd,
        "lz4" => rocksdb::DBCompressionType::Lz4,
        "zlib" => rocksdb::DBCompressionType::Zlib,
        _ => {
            warn!(compression_type = %compression_type_str, "Unknown compression type, using lz4");
            rocksdb::DBCompressionType::Lz4
        }
    };
    
    let enable_pipelined_write = config.get_bool("rocksdb.enable_pipelined_write").unwrap_or(true);
    let allow_concurrent_memtable_write = config.get_bool("rocksdb.allow_concurrent_memtable_write").unwrap_or(true);
    let enable_write_thread_adaptive_yield = config.get_bool("rocksdb.enable_write_thread_adaptive_yield").unwrap_or(true);
    
    // Create shared block cache for all column families
    let block_cache = Cache::new_lru_cache(block_cache_size_mb as usize * 1024 * 1024);
    
    // Create optimized options for each column family based on access patterns
    let mut cf_descriptors = vec![ColumnFamilyDescriptor::new("default", Options::default())];
    
    for cf_name in COLUMN_FAMILIES.iter() {
        let mut cf_opts = Options::default();
        
        // Create block-based table options for this CF
        let mut block_opts = BlockBasedOptions::default();
        block_opts.set_block_cache(&block_cache);
        
        // Common settings for all CFs
        cf_opts.set_write_buffer_size(write_buffer_size_mb as usize * 1024 * 1024);
        cf_opts.set_max_write_buffer_number(max_write_buffer_number);
        cf_opts.set_min_write_buffer_number_to_merge(min_write_buffer_number_to_merge);
        cf_opts.set_compression_type(compression_type);
        cf_opts.set_target_file_size_base(target_file_size_mb as u64 * 1024 * 1024);
        cf_opts.set_level_zero_file_num_compaction_trigger(level0_file_num_compaction_trigger);
        cf_opts.set_level_zero_slowdown_writes_trigger(level0_slowdown_writes_trigger);
        cf_opts.set_level_zero_stop_writes_trigger(level0_stop_writes_trigger);
        
        // Per-CF optimizations based on access patterns
        match *cf_name {
            "blocks" => {
                // Blocks: Append-only, large values, sequential reads
                // Universal compaction is better for append-only workloads
                cf_opts.set_compaction_style(rocksdb::DBCompactionStyle::Universal);
                cf_opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(1)); // 'b' prefix
            }
            "transactions" => {
                // Transactions: High write volume, prefix-based scans (by height)
                cf_opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(1)); // 't' prefix
                cf_opts.optimize_for_point_lookup(256); // Bloom filter for point lookups (256MB)
            }
            "addr_index" => {
                // Address index: High read volume, prefix scans (by address)
                cf_opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(1)); // 'a' prefix
                cf_opts.optimize_for_point_lookup(512); // Larger bloom filter for addresses (512MB)
                // More aggressive caching for frequently accessed addresses
                block_opts.set_block_size(32 * 1024); // Larger blocks (32KB) for better compression
            }
            "utxo" => {
                // UTXO: High read/write volume, point lookups
                cf_opts.optimize_for_point_lookup(256);
                cf_opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(1)); // 'u' prefix
            }
            "chain_metadata" | "chain_state" => {
                // Metadata: Low volume, critical for correctness
                // Use Level compaction (default) for better consistency
                cf_opts.set_write_buffer_size(64 * 1024 * 1024); // Smaller buffer (less critical path)
            }
            _ => {
                // Default settings for other CFs
            }
        }
        
        // Apply block-based table options to this CF
        cf_opts.set_block_based_table_factory(&block_opts);
        
        cf_descriptors.push(ColumnFamilyDescriptor::new(cf_name.to_string(), cf_opts));
    }

    // Database-level options (apply to all CFs)
    let mut db_options = Options::default();
    db_options.create_if_missing(true);
    db_options.create_missing_column_families(true);
    db_options.set_max_open_files(max_open_files);
    db_options.set_max_background_jobs(max_background_jobs);
    db_options.set_max_subcompactions(max_subcompactions as u32);
    
    // Advanced concurrency settings
    if enable_pipelined_write {
        db_options.set_enable_pipelined_write(true);
    }
    if allow_concurrent_memtable_write {
        db_options.set_allow_concurrent_memtable_write(true);
    }
    if enable_write_thread_adaptive_yield {
        db_options.set_enable_write_thread_adaptive_yield(true);
    }
    
    // Logging and stats
    db_options.set_stats_dump_period_sec(300); // Dump stats every 5 minutes
    db_options.set_keep_log_file_num(3); // Keep last 3 log files
    
    info!(
        write_buffer_mb = write_buffer_size_mb,
        block_cache_mb = block_cache_size_mb,
        max_open_files = max_open_files,
        background_jobs = max_background_jobs,
        compression = ?compression_type,
        pipelined_writes = enable_pipelined_write,
        "RocksDB configuration"
    );
    
    let db = DB::open_cf_descriptors(
        &db_options,
        db_path_str,
        cf_descriptors,
    )?;
    let db_arc = Arc::new(db);

    // Initialize cached column family handles
    // This validates all required CFs exist at startup
    info!("Initializing database column family handles");
    let _db_handles = rustyblox::db_handles::DbHandles::new(Arc::clone(&db_arc))
        .map_err(|e| format!("Failed to initialize DB handles: {}", e))?;
    info!("Database handles validated");

    let blk_dir_path = PathBuf::from(blk_dir);
    
    // Create shared state
    let mempool_state = Arc::new(MempoolState::new());
    let broadcaster = Arc::new(EventBroadcaster::new());
    
    // Spawn mempool monitor service (can start early)
    let mempool_clone = Arc::clone(&mempool_state);
    tokio::spawn(async move {
        if let Err(e) = run_mempool_monitor(mempool_clone, 10).await {
            error!(error = ?e, "Mempool monitor error");
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
        info!("Starting blockchain sync");
        let sync_db = Arc::clone(&db_arc);
        let sync_broadcaster = Arc::clone(&broadcaster);
        let blk_dir_clone = blk_dir_path.clone();

        tokio::task::spawn_blocking(move || {
            // Use tokio runtime for async operations
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async {
                if let Err(e) = run_sync_service(blk_dir_clone, sync_db, Some(sync_broadcaster)).await {
                    error!(error = ?e, "Sync service error");
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
                        info!(height = height, "Minimum viable data indexed - starting web server");
                        break;
                    }
                    info!(height = height, "Indexing in progress - waiting for minimum 1000 blocks");
                }
                _ => {
                    info!("Indexing in progress - waiting for initial data");
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    } else {
        warn!("sync.auto_start=false - skipping sync service and starting web server in read-only mode");
    }
    
    // NOW start web server (data is ready)
    let api_mempool = Arc::clone(&mempool_state);
    let api_broadcaster = Arc::clone(&broadcaster);
    start_web_server(api_db, api_mempool, api_broadcaster).await;

    Ok(())
}