// jemalloc: returns freed memory to the OS far more aggressively than the system
// allocator (the macOS default retained the per-tx deserialization churn, inflating
// enrichment RSS ~6GB-raw -> ~24GB-resident). The #[global_allocator] must live in
// each binary root, not lib.rs (it would conflict across the 14 bins).
#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use rustyblox::api::{
    // Addresses module
    addr_v2,
    // Root handlers
    api_handler,
    // Blocks module
    block_index_v2,
    block_stats_v2,
    block_v2,
    // Governance module
    budget_info_v2,
    budget_projection_v2,
    budget_votes_v2,
    cache_stats_v2,
    coldstaking_analytics,
    finalized_budgets_v2,
    health_check_v2,
    hodl_analytics,
    mempool_tx_v2,
    mempool_v2,
    // Masternodes module
    mn_count_v2,
    mn_list_v2,
    money_supply_v2,
    network_health_analytics,
    // Price module
    price_v2,
    relay_mnb_v2,
    rich_list,
    root_handler,
    // Search module
    search_v2,
    send_tx_post_v2,
    send_tx_v2,
    snapshots_analytics,
    staking_analytics,
    // Network module
    status_v2,
    // Analytics module
    supply_analytics,
    transaction_analytics,
    treasury_analytics,
    // Transactions module
    tx_v2,
    utxo_v2,
    wealth_distribution,
    xpub_v2,
};
use rustyblox::block_detail::block_detail_v2;
use rustyblox::cache::CacheManager;
use rustyblox::config::{get_global_config, init_global_config};
use rustyblox::mempool::{run_mempool_monitor, MempoolState};
use rustyblox::metrics;
use rustyblox::sync::run_sync_service;
use rustyblox::telemetry::{init_tracing, TelemetryConfig};
use rustyblox::types::MyError;
use rustyblox::websocket::{
    ws_blocks_handler, ws_mempool_handler, ws_transactions_handler, EventBroadcaster,
};
use tracing::{error, info, warn};

use axum::http::header::{HeaderName, HeaderValue};
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use lazy_static::lazy_static;
use rocksdb::{BlockBasedOptions, Cache, ColumnFamilyDescriptor, Options, DB};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex as TokioMutex, Semaphore};
use tower_http::cors::{Any, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use tracing::info_span;

// Single source of truth for the DB column-family set (incl. the private tail CFs).
use rustyblox::COLUMN_FAMILIES;

lazy_static! {
    static ref DB_MUTEX: TokioMutex<()> = TokioMutex::new(());
}

// ============================================================================
// Request admission control (P1-2)
// ----------------------------------------------------------------------------
// The backend listens on 0.0.0.0:3005 and previously had only a 30s timeout.
// There was no ceiling on the number of in-flight requests, so a burst of
// traffic (or an abusive client hammering the unauthenticated node-broadcast
// proxies /sendtx and /relaymnb) could exhaust the tokio runtime and the
// upstream PIVX node. We add two semaphore-backed concurrency caps applied as
// axum middleware:
//   * a GLOBAL cap across every route (defence-in-depth alongside the
//     reverse-proxy per-IP rate limiting in frontend-legacy/nginx.conf), and
//   * a tighter cap scoped to the broadcast routes so transaction/MN-broadcast
//     floods can't monopolise the node even within the global budget.
// (tower's GlobalConcurrencyLimitLayer would require enabling tower's `limit`
// feature as a new direct dependency; using axum middleware + a tokio
// Semaphore — both already direct deps — achieves the same back-pressure with
// no Cargo manifest change. Over-limit requests get 503 + Retry-After.)
const GLOBAL_MAX_INFLIGHT: usize = 256;
const BROADCAST_MAX_INFLIGHT: usize = 8;

/// Concurrency-limit middleware: refuses (503) once `permits` in-flight
/// requests are already being served by the guarded scope. The permit is held
/// for the lifetime of the request (including the downstream handler) and
/// released on drop when the response is produced.
async fn concurrency_limit(permits: Arc<Semaphore>, request: Request, next: Next) -> Response {
    match permits.try_acquire() {
        Ok(_permit) => next.run(request).await,
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            [("Retry-After", "1")],
            "server busy: concurrency limit reached",
        )
            .into_response(),
    }
}

/// Security-header middleware (P2-1): stamps conservative hardening headers on
/// every response. This is a read-only public block explorer so we keep CORS
/// origins permissive (see CorsLayer below) but lock down framing, MIME
/// sniffing, referrer leakage, and script/source origins via CSP.
async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    // Block MIME-type sniffing.
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    // Disallow embedding in frames (clickjacking).
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    // Never leak URLs/paths in the Referer header to third parties.
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    // Conservative CSP for the SPA: same-origin scripts/styles/connect, no
    // plugins/framing, inline styles allowed (Vue scoped styles), data: images.
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
             script-src 'self'; connect-src 'self'; font-src 'self' data:; \
             object-src 'none'; frame-ancestors 'none'; base-uri 'self'",
        ),
    );
    response
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

async fn start_web_server(
    db_arc: Arc<DB>,
    mempool_state: Arc<MempoolState>,
    broadcaster: Arc<EventBroadcaster>,
) {
    let config = get_global_config();

    // Initialize cache manager
    let cache_manager = Arc::new(CacheManager::new());

    // Get server configuration
    let server_host = config
        .get_string("server.host")
        .unwrap_or_else(|_| "0.0.0.0".to_string());
    let server_port: u16 = config.get_int("server.port").unwrap_or(3005) as u16;

    // Configure CORS (P2-1). This is a read-only PUBLIC block explorer, so we
    // intentionally keep `allow_origin(Any)` — anyone may read the chain data
    // from any site and there are no cookies/credentials to protect (CORS is
    // not a server-side authorization boundary here). We DO narrow methods to
    // the two we actually serve — GET for reads and POST for the
    // Blockbook-compatible /sendtx broadcast — instead of the previous `Any`,
    // and likewise scope allowed request headers to Content-Type.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    // Per-IP rate limiting lives at the reverse proxy (frontend-legacy/nginx.conf).
    // In-process we add semaphore-backed concurrency caps (see consts above): a
    // global ceiling and a tighter one around the node-broadcast routes.
    let global_limit = Arc::new(Semaphore::new(GLOBAL_MAX_INFLIGHT));
    let broadcast_limit = Arc::new(Semaphore::new(BROADCAST_MAX_INFLIGHT));

    // Node-broadcast proxies are unauthenticated and forward straight to the
    // upstream PIVX node, so they get their own tighter concurrency bound on
    // top of the global cap (P1-2). Split into a sub-router so the broadcast
    // limit layers ONLY these routes; merged into the main app below.
    let broadcast_routes = Router::new()
        .route("/api/v2/sendtx/{hex_tx}", get(send_tx_v2))
        .route("/api/v2/sendtx", post(send_tx_post_v2)) // Blockbook-compatible POST endpoint
        .route("/api/v2/relaymnb/{hex_mnb}", get(relay_mnb_v2))
        .layer(middleware::from_fn(move |req, next| {
            concurrency_limit(broadcast_limit.clone(), req, next)
        }));

    // We just want to mimic blockbook API endpoints and structure for compatibility
    let app = Router::new()
        .route("/api", get(api_handler))
        .route("/api/", get(status_v2)) // Same as /api/v2/status
        .route("/api/endpoint", get(api_handler))
        .route("/api/v2/status", get(status_v2))
        .route("/api/v2/health", get(health_check_v2))
        // NOTE (P3-3): /api/v2/cache/stats and /metrics (below) are operational
        // endpoints exposed on the same PUBLIC listener as the explorer API.
        // They leak no secrets but do expose internal cache/runtime detail. The
        // reverse proxy (frontend-legacy/nginx.conf) only forwards /api/ and
        // /ws/, so /metrics is NOT reachable through the public vhost; restrict
        // /api/v2/cache/stats at the proxy too if it must stay private. A future
        // hardening step is to bind these behind a separate admin listener /
        // config flag — left as a deliberate TODO to avoid breaking Prometheus
        // scraping which targets this port directly.
        .route("/api/v2/cache/stats", get(cache_stats_v2)) // Cache statistics endpoint
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
        .route("/api/v2/mncount", get(mn_count_v2))
        .route("/api/v2/mnlist", get(mn_list_v2))
        .route("/api/v2/moneysupply", get(money_supply_v2))
        .route("/api/v2/budgetinfo", get(budget_info_v2))
        .route("/api/v2/budgetvotes/{proposal_name}", get(budget_votes_v2))
        .route("/api/v2/budgetprojection", get(budget_projection_v2))
        .route("/api/v2/finalizedbudgets", get(finalized_budgets_v2))
        .route(
            "/api/v2/mnrawbudgetvote/{raw_vote_params}",
            get(api_handler),
        )
        .route("/api/v2/analytics/supply", get(supply_analytics))
        .route("/api/v2/analytics/transactions", get(transaction_analytics))
        .route("/api/v2/analytics/staking", get(staking_analytics))
        .route("/api/v2/analytics/network", get(network_health_analytics))
        .route("/api/v2/analytics/richlist", get(rich_list))
        .route(
            "/api/v2/analytics/wealth-distribution",
            get(wealth_distribution),
        )
        .route("/api/v2/analytics/hodl", get(hodl_analytics))
        .route("/api/v2/analytics/snapshots", get(snapshots_analytics))
        .route("/api/v2/analytics/treasury", get(treasury_analytics))
        .route("/api/v2/analytics/coldstaking", get(coldstaking_analytics))
        .route("/api/v2/price", get(price_v2)) // PIVX price data endpoint
        .route("/ws/blocks", get(ws_blocks_handler))
        .route("/ws/transactions", get(ws_transactions_handler))
        .route("/ws/mempool", get(ws_mempool_handler))
        .route("/metrics", get(metrics_handler)) // Prometheus metrics endpoint
        // Merge the rate-bounded broadcast proxies (their tighter concurrency
        // cap is already layered onto this sub-router above).
        .merge(broadcast_routes);

    // Serve the built frontend (SPA) for everything that isn't an API route.
    // ServeDir handles the hashed /assets/*.js|css files; unknown paths fall
    // back to index.html so client-side routing works. Previously only "/"
    // was routed (root_handler returned index.html with NO asset serving), so
    // every built bundle 404'd its own JS/CSS when hit directly on this port.
    let frontend_dist = config
        .get_string("paths.frontend_dist")
        .unwrap_or_else(|_| "frontend/dist".to_string());
    let app = if std::path::Path::new(&frontend_dist)
        .join("index.html")
        .exists()
    {
        info!(path = %frontend_dist, "Serving frontend");
        let index = std::path::Path::new(&frontend_dist).join("index.html");
        let assets_dir = std::path::Path::new(&frontend_dist).join("assets");
        // Hashed bundles live under /assets — serve them directly and return a
        // real 404 on a miss (NO index.html fallback here). Falling back to
        // index.html for a missing chunk returns 200 text/html, which browsers
        // reject with a strict-MIME error on every redeploy (stale clients still
        // request the previous build's chunk hashes). Non-asset paths fall
        // through to the SPA index below for client-side routing.
        let spa = tower_http::services::ServeDir::new(&frontend_dist)
            .fallback(tower_http::services::ServeFile::new(index));
        app.nest_service("/assets", tower_http::services::ServeDir::new(assets_dir))
            .fallback_service(spa)
    } else {
        warn!(path = %frontend_dist, "Frontend dist not found - build with: cd frontend-legacy && npm run build");
        app.route("/", get(root_handler))
    };

    let app = app
        // Stamp hardening headers on every response (P2-1).
        .layer(middleware::from_fn(security_headers))
        .layer(cors)
        // Hard ceiling on request duration: a wedged handler can no longer pin
        // a connection forever (returns 408 on expiry). tower-http 0.7 deprecated
        // TimeoutLayer::new; with_status_code keeps the same 408 behavior explicitly.
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(30),
        ))
        // GLOBAL in-flight cap (P1-2). Outermost limiter so we shed load before
        // doing any per-request work; the broadcast routes carry an additional
        // tighter cap layered on their sub-router above. 503 + Retry-After once
        // saturated. Layers are applied outermost-last, so this is the first
        // thing every request hits.
        .layer(middleware::from_fn(move |req, next| {
            concurrency_limit(global_limit.clone(), req, next)
        }))
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
        SocketAddr::from((
            [host_parts[0], host_parts[1], host_parts[2], host_parts[3]],
            server_port,
        ))
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
    info!(addr = %addr, "Listening");

    // Graceful shutdown on SIGTERM (Docker/systemd stop) and SIGINT (Ctrl-C):
    // stop accepting connections, drain in-flight requests, then let main exit
    // so RocksDB closes cleanly instead of being killed mid-write.
    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        error!(error = ?e, "Web server error");
    }
    info!("Web server shut down gracefully");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("Shutdown signal received - draining connections");
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
        .map_err(|e| format!("Error getting server.worker_threads: {e}"))?;
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
    let max_write_buffer_number = config
        .get_int("rocksdb.max_write_buffer_number")
        .unwrap_or(4) as i32;
    let min_write_buffer_number_to_merge = config
        .get_int("rocksdb.min_write_buffer_number_to_merge")
        .unwrap_or(2) as i32;
    let block_cache_size_mb = config.get_int("rocksdb.block_cache_size").unwrap_or(512);
    // Global memtable ceiling across all CFs (0 = unlimited). Bounds total write-
    // buffer RAM, which is otherwise capped only per-CF (write_buffer_size x
    // max_write_buffer_number x CFs ~= several GB) — a real peak contributor on
    // an 8 GB VPS shared with pivxd.
    let db_write_buffer_size_mb = config
        .get_int("rocksdb.db_write_buffer_size")
        .unwrap_or(1024);
    let target_file_size_mb = config
        .get_int("rocksdb.target_file_size_base")
        .unwrap_or(256);
    let max_open_files = config.get_int("rocksdb.max_open_files").unwrap_or(5000) as i32;
    let level0_file_num_compaction_trigger = config
        .get_int("rocksdb.level0_file_num_compaction_trigger")
        .unwrap_or(8) as i32;
    let level0_slowdown_writes_trigger = config
        .get_int("rocksdb.level0_slowdown_writes_trigger")
        .unwrap_or(20) as i32;
    let level0_stop_writes_trigger = config
        .get_int("rocksdb.level0_stop_writes_trigger")
        .unwrap_or(36) as i32;
    let max_background_jobs = config.get_int("rocksdb.max_background_jobs").unwrap_or(8) as i32;
    let max_subcompactions = config.get_int("rocksdb.max_subcompactions").unwrap_or(4);

    let compression_type_str = config
        .get_string("rocksdb.compression_type")
        .unwrap_or_else(|_| "lz4".to_string());
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

    let enable_pipelined_write = config
        .get_bool("rocksdb.enable_pipelined_write")
        .unwrap_or(true);
    let allow_concurrent_memtable_write = config
        .get_bool("rocksdb.allow_concurrent_memtable_write")
        .unwrap_or(true);
    let enable_write_thread_adaptive_yield = config
        .get_bool("rocksdb.enable_write_thread_adaptive_yield")
        .unwrap_or(true);

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
                cf_opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(1));
                // 'b' prefix
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
                cf_opts.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(1));
                // 'u' prefix
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
    // Cap total memtable memory across all column families. RocksDB flushes the
    // largest memtable when the aggregate is exceeded, bounding RAM WITHOUT
    // shrinking the per-CF buffers (which would multiply flush + compaction I/O
    // and slow sync). 0 leaves it at per-CF bounds only.
    if db_write_buffer_size_mb > 0 {
        db_options.set_db_write_buffer_size(db_write_buffer_size_mb as usize * 1024 * 1024);
    }

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
        db_write_buffer_mb = db_write_buffer_size_mb,
        block_cache_mb = block_cache_size_mb,
        max_open_files = max_open_files,
        background_jobs = max_background_jobs,
        compression = ?compression_type,
        pipelined_writes = enable_pipelined_write,
        "RocksDB configuration"
    );

    let db = DB::open_cf_descriptors(&db_options, db_path_str, cf_descriptors)?;
    let db_arc = Arc::new(db);

    // Initialize cached column family handles
    // This validates all required CFs exist at startup
    info!("Initializing database column family handles");
    let _db_handles = rustyblox::db_handles::DbHandles::new(Arc::clone(&db_arc))
        .map_err(|e| format!("Failed to initialize DB handles: {e}"))?;
    info!("Database handles validated");

    // Restore persisted metrics from database
    // This ensures metrics survive restarts and maintain continuity
    info!("Restoring metrics from database");
    if let Err(e) = rustyblox::metrics::load_metrics_from_db(&db_arc) {
        warn!(error = %e, "Failed to restore metrics from database - starting from defaults");
    } else {
        info!("Successfully restored metrics from database");
    }

    let blk_dir_path = PathBuf::from(blk_dir);

    // Create shared state
    let mempool_state = Arc::new(MempoolState::new());
    let broadcaster = Arc::new(EventBroadcaster::new());

    // Spawn database size sampler (background monitoring)
    let sampler_db = Arc::clone(&db_arc);
    tokio::spawn(async move {
        rustyblox::db_sampler::start_db_size_sampler(sampler_db, 60).await;
    });

    // Spawn mempool monitor service (can start early)
    let mempool_clone = Arc::clone(&mempool_state);
    tokio::spawn(async move {
        if let Err(e) = run_mempool_monitor(mempool_clone, 10).await {
            error!(error = ?e, "Mempool monitor error");
        }
    });

    // Spawn the live orphan-tail (opt-in; sync.live_tail_blkfiles, default off).
    // Pure observer: reads blk*.dat into the private tail_blocks/tail_meta CFs and
    // writes NO canonical CF. See DESIGN-live-orphan-capture.md.
    if rustyblox::blk_tail::is_enabled() {
        let tail_db = Arc::clone(&db_arc);
        let tail_blk_dir = blk_dir_path.clone();
        tokio::spawn(async move {
            rustyblox::blk_tail::run_tail(tail_db, tail_blk_dir, rustyblox::blk_tail::PIVX_MAGIC)
                .await;
        });
    }

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

        // Spawn sync in a completely separate OS thread with its own runtime
        // This isolates it from the main tokio runtime and avoids nested runtime issues
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to build runtime");

            // Retry the sync service on a fatal error with capped backoff instead of
            // letting the thread die. A prolonged RPC outage can abort a catch-up — the
            // heightless-block backfill rewinds sync_height for re-detection and returns
            // Err — and the web server keeps the process alive, so without this loop the
            // explorer would serve a frozen tip even after RPC recovers (no supervisor
            // restart fires). The rewound sync_height makes each retry re-run the whole
            // catch-up + backfill, so it self-heals once RPC is back.
            let mut backoff_secs = 5u64;
            loop {
                let result = rt.block_on(run_sync_service(
                    blk_dir_clone.clone(),
                    Arc::clone(&sync_db),
                    Some(Arc::clone(&sync_broadcaster)),
                ));
                match result {
                    Ok(()) => break, // clean completion (e.g. read-only / auto_start=false)
                    Err(e) => {
                        error!(error = ?e, retry_secs = backoff_secs, "Sync service error - retrying");
                        std::thread::sleep(std::time::Duration::from_secs(backoff_secs));
                        backoff_secs = (backoff_secs * 2).min(120);
                    }
                }
            }
        });

        // Wait for minimum viable sync before starting web server. Poll every 2s, but
        // throttle the "still waiting" heartbeat to ~once a minute so the multi-minute
        // leveldb import doesn't flood the log with hundreds of identical lines. The
        // gate still opens promptly (the height check runs every tick); only the
        // logging is throttled.
        let mut waited_ticks: u32 = 0;
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
                        info!(
                            height = height,
                            "Minimum viable data indexed - starting web server"
                        );
                        break;
                    }
                    if waited_ticks % 30 == 0 {
                        info!(
                            height = height,
                            "Indexing in progress - waiting for minimum 1000 blocks"
                        );
                    }
                }
                _ => {
                    if waited_ticks % 30 == 0 {
                        info!("Indexing in progress - waiting for initial data");
                    }
                }
            }
            waited_ticks += 1;
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
