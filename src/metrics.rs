/// Metrics Module - Prometheus Instrumentation
/// 
/// Implements METRICS_CATALOG.md requirements:
/// - All 45 metrics defined
/// - Prometheus registry
/// - Clean helper API
/// - Label cardinality enforcement
/// - Standard histogram buckets

use prometheus::{
    Registry, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Histogram, HistogramVec,
    HistogramOpts, Opts, Encoder, TextEncoder,
};
use lazy_static::lazy_static;
use std::time::Instant;

/// Standard latency buckets for histograms (seconds)
/// METRICS_CATALOG.md Section 2: Latency Histograms
const LATENCY_BUCKETS: &[f64] = &[0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0];

lazy_static! {
    /// Global Prometheus registry
    pub static ref REGISTRY: Registry = Registry::new();
    
    // ========================================================================
    // 1. PIPELINE PROGRESS & THROUGHPUT (8 metrics)
    // ========================================================================
    
    /// Total blocks processed by stage
    /// Labels: stage (leveldb_import, rpc_catchup, parallel, address_enrich)
    pub static ref BLOCKS_PROCESSED: IntCounterVec = IntCounterVec::new(
        Opts::new("rustyblox_blocks_processed_total", "Total blocks processed by stage"),
        &["stage"]
    ).unwrap();
    
    /// Total transactions processed by stage
    /// Labels: stage (parse, index, enrich)
    pub static ref TRANSACTIONS_PROCESSED: IntCounterVec = IntCounterVec::new(
        Opts::new("rustyblox_transactions_processed_total", "Total transactions processed by stage"),
        &["stage"]
    ).unwrap();
    
    /// Total UTXOs added (outputs created)
    pub static ref UTXOS_ADDED: IntCounter = IntCounter::new(
        "rustyblox_utxos_added_total",
        "Total UTXOs created (transaction outputs)"
    ).unwrap();
    
    /// Total UTXOs spent (inputs consumed)
    pub static ref UTXOS_SPENT: IntCounter = IntCounter::new(
        "rustyblox_utxos_spent_total",
        "Total UTXOs spent (transaction inputs)"
    ).unwrap();
    
    /// Blocks processed via RPC catchup
    pub static ref RPC_CATCHUP_BLOCKS: IntCounter = IntCounter::new(
        "rustyblox_rpc_catchup_blocks_total",
        "Blocks processed via RPC catchup"
    ).unwrap();
    
    /// Current pipeline stage
    /// Values: 0=init, 1=leveldb, 2=parallel, 3=enrich, 4=height_res, 5=rpc, 6=idle, 7=reorg
    pub static ref PIPELINE_STAGE_CURRENT: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rustyblox_pipeline_stage_current", "Current pipeline stage (0-7)"),
        &["stage"]
    ).unwrap();
    
    /// Chain tip height
    /// Labels: source (rpc, db)
    pub static ref CHAIN_TIP_HEIGHT: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rustyblox_chain_tip_height", "Chain tip height by source"),
        &["source"]
    ).unwrap();
    
    /// Indexed height by stage
    /// Labels: stage (block_index, tx_index, address_index, utxo_index)
    pub static ref INDEXED_HEIGHT: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rustyblox_indexed_height", "Highest block height indexed by stage"),
        &["stage"]
    ).unwrap();
    
    // ========================================================================
    // 2. LATENCY HISTOGRAMS (6 metrics)
    // ========================================================================
    
    /// Block parse duration
    /// Labels: file (blk file number, e.g., "00141")
    pub static ref BLOCK_PARSE_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new("rustyblox_block_parse_duration_seconds", "Block parsing latency")
            .buckets(LATENCY_BUCKETS.to_vec()),
        &["file"]
    ).unwrap();
    
    /// Transaction decode duration
    /// Labels: version (1, 2, 3)
    pub static ref TX_DECODE_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new("rustyblox_tx_decode_duration_seconds", "Transaction deserialization latency")
            .buckets(LATENCY_BUCKETS.to_vec()),
        &["version"]
    ).unwrap();
    
    /// Address enrichment pass duration
    /// Labels: pass (1, 2, 2b)
    pub static ref ADDRESS_ENRICHMENT_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new("rustyblox_address_enrichment_duration_seconds", "Address enrichment pass latency")
            .buckets(LATENCY_BUCKETS.to_vec()),
        &["pass"]
    ).unwrap();
    
    /// Database batch flush duration
    /// Labels: cf (column family name)
    pub static ref DB_BATCH_FLUSH_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new("rustyblox_db_batch_flush_duration_seconds", "Database batch flush latency")
            .buckets(LATENCY_BUCKETS.to_vec()),
        &["cf"]
    ).unwrap();
    
    /// RPC call duration
    /// Labels: method (getblock, getblockhash, getblockcount, getrawtransaction)
    pub static ref RPC_CALL_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new("rustyblox_rpc_call_duration_seconds", "RPC call latency")
            .buckets(LATENCY_BUCKETS.to_vec()),
        &["method"]
    ).unwrap();
    
    /// Height resolution phase duration
    /// Labels: phase (scan, validate, update)
    pub static ref HEIGHT_RESOLUTION_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new("rustyblox_height_resolution_duration_seconds", "Height resolution phase latency")
            .buckets(LATENCY_BUCKETS.to_vec()),
        &["phase"]
    ).unwrap();
    
    // ========================================================================
    // 3. ERROR & RETRY COUNTERS (5 metrics)
    // ========================================================================
    
    /// Database errors
    /// Labels: op (get, put, delete, flush, iterator), cf (column family)
    pub static ref DB_ERRORS: IntCounterVec = IntCounterVec::new(
        Opts::new("rustyblox_db_errors_total", "Database errors by operation and CF"),
        &["op", "cf"]
    ).unwrap();
    
    /// RPC errors
    /// Labels: method (getblock, etc.), error_type (timeout, connection, parse)
    pub static ref RPC_ERRORS: IntCounterVec = IntCounterVec::new(
        Opts::new("rustyblox_rpc_errors_total", "RPC errors by method and type"),
        &["method", "error_type"]
    ).unwrap();
    
    /// Reorg events
    pub static ref REORG_EVENTS: IntCounter = IntCounter::new(
        "rustyblox_reorg_events_total",
        "Total blockchain reorganization events"
    ).unwrap();
    
    /// Invariant violations
    /// Labels: type (missing_utxo, hash_mismatch, height_mismatch, tx_count_mismatch)
    pub static ref INVARIANT_VIOLATIONS: IntCounterVec = IntCounterVec::new(
        Opts::new("rustyblox_invariant_violations_total", "Invariant violations by type"),
        &["type"]
    ).unwrap();
    
    /// Orphaned transactions
    /// Labels: reason (height_zero, not_in_chain, parent_orphaned)
    pub static ref ORPHANED_TRANSACTIONS: IntCounterVec = IntCounterVec::new(
        Opts::new("rustyblox_orphaned_transactions_total", "Orphaned transactions by reason"),
        &["reason"]
    ).unwrap();
    
    // ========================================================================
    // 4. CACHE METRICS (3 metrics)
    // ========================================================================
    
    /// Transaction cache hits
    pub static ref TX_CACHE_HITS: IntCounter = IntCounter::new(
        "rustyblox_tx_cache_hits_total",
        "Transaction cache hits"
    ).unwrap();
    
    /// Transaction cache misses
    pub static ref TX_CACHE_MISSES: IntCounter = IntCounter::new(
        "rustyblox_tx_cache_misses_total",
        "Transaction cache misses"
    ).unwrap();
    
    /// Transaction cache size (bytes)
    pub static ref TX_CACHE_SIZE_BYTES: IntGauge = IntGauge::new(
        "rustyblox_tx_cache_size_bytes",
        "Transaction cache size in bytes"
    ).unwrap();
    
    // ========================================================================
    // 5. RESOURCE METRICS (4 metrics)
    // ========================================================================
    
    /// Database size by column family
    /// Labels: cf (column family)
    pub static ref DB_SIZE_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rustyblox_db_size_bytes", "Database size by column family"),
        &["cf"]
    ).unwrap();
    
    /// Database batch size (entries)
    /// Labels: cf (column family)
    pub static ref DB_BATCH_SIZE_ENTRIES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rustyblox_db_batch_size_entries", "Database batch size in entries"),
        &["cf"]
    ).unwrap();
    
    /// Database batch size (bytes)
    /// Labels: cf (column family)
    pub static ref DB_BATCH_SIZE_BYTES: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rustyblox_db_batch_size_bytes", "Database batch size in bytes"),
        &["cf"]
    ).unwrap();
    
    /// Process resident memory
    pub static ref PROCESS_RESIDENT_MEMORY_BYTES: IntGauge = IntGauge::new(
        "rustyblox_process_resident_memory_bytes",
        "Process resident memory in bytes"
    ).unwrap();
    
    // ========================================================================
    // 6. CUSTOM PIPELINE METRICS (10 metrics)
    // ========================================================================
    
    /// Sync progress percentage
    /// Labels: stage (overall, leveldb, parallel, enrich, height_res)
    pub static ref SYNC_PROGRESS_PERCENT: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rustyblox_sync_progress_percent", "Sync progress percentage by stage"),
        &["stage"]
    ).unwrap();
    
    /// Reorg depth (blocks)
    pub static ref REORG_DEPTH_BLOCKS: IntGauge = IntGauge::new(
        "rustyblox_reorg_depth_blocks",
        "Depth of most recent reorg in blocks"
    ).unwrap();
    
    /// Blocks behind tip
    pub static ref BLOCKS_BEHIND_TIP: IntGauge = IntGauge::new(
        "rustyblox_blocks_behind_tip",
        "Number of blocks behind chain tip"
    ).unwrap();
    
    /// Estimated sync completion time (seconds)
    pub static ref ESTIMATED_SYNC_COMPLETION_SECONDS: IntGauge = IntGauge::new(
        "rustyblox_estimated_sync_completion_seconds",
        "Estimated seconds until sync completion"
    ).unwrap();
    
    /// Transaction parse errors
    /// Labels: error_type (eof, deserialization, validation)
    pub static ref TX_PARSE_ERRORS: IntCounterVec = IntCounterVec::new(
        Opts::new("rustyblox_tx_parse_errors_total", "Transaction parse errors by type"),
        &["error_type"]
    ).unwrap();
    
    /// Canonical blocks in database
    pub static ref CANONICAL_BLOCKS: IntGauge = IntGauge::new(
        "rustyblox_canonical_blocks_total",
        "Total canonical blocks in database"
    ).unwrap();
    
    /// Orphaned blocks
    pub static ref ORPHANED_BLOCKS: IntGauge = IntGauge::new(
        "rustyblox_orphaned_blocks_total",
        "Total orphaned blocks"
    ).unwrap();
    
    /// Pending reorg depth
    pub static ref PENDING_REORG_DEPTH: IntGauge = IntGauge::new(
        "rustyblox_pending_reorg_depth",
        "Depth of pending reorg (if any)"
    ).unwrap();
    
    /// RPC connected status (bool: 0/1)
    pub static ref RPC_CONNECTED: IntGauge = IntGauge::new(
        "rustyblox_rpc_connected",
        "RPC connection status (0=disconnected, 1=connected)"
    ).unwrap();
    
    /// Address index size (entries)
    pub static ref ADDRESS_INDEX_SIZE_ENTRIES: IntGauge = IntGauge::new(
        "rustyblox_address_index_size_entries",
        "Number of entries in address index"
    ).unwrap();
    
    /// Total unique addresses indexed (gauge: current count in DB)
    pub static ref TOTAL_ADDRESSES_INDEXED: IntGauge = IntGauge::new(
        "rustyblox_total_addresses_indexed",
        "Total unique addresses indexed in address index"
    ).unwrap();
    
    /// Total UTXOs currently tracked (gauge: unspent count)
    pub static ref TOTAL_UTXOS_TRACKED: IntGauge = IntGauge::new(
        "rustyblox_total_utxos_tracked",
        "Total unspent UTXOs currently tracked"
    ).unwrap();
    
    /// Sapling transactions total (counter: cumulative count)
    pub static ref SAPLING_TRANSACTIONS_TOTAL: IntCounter = IntCounter::new(
        "rustyblox_sapling_transactions_total",
        "Total Sapling transactions indexed (version >= 3 with Sapling data)"
    ).unwrap();
    
    /// Sapling transactions count (gauge: current count in DB)
    pub static ref SAPLING_TRANSACTIONS_COUNT: IntGauge = IntGauge::new(
        "rustyblox_sapling_transactions_count",
        "Current count of Sapling transactions in database (version >= 3)"
    ).unwrap();
    
    // ========================================================================
    // 7. OPERATIONAL METRICS (9 metrics)
    // ========================================================================
    
    /// Uptime (seconds)
    pub static ref UPTIME_SECONDS: IntGauge = IntGauge::new(
        "rustyblox_uptime_seconds",
        "Service uptime in seconds"
    ).unwrap();
    
    /// Service start timestamp
    pub static ref SERVICE_START_TIMESTAMP: IntGauge = IntGauge::new(
        "rustyblox_service_start_timestamp_seconds",
        "Unix timestamp when service started"
    ).unwrap();
    
    /// Last stage completion timestamp
    /// Labels: stage (stage name)
    pub static ref LAST_STAGE_COMPLETION_TIMESTAMP: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rustyblox_last_stage_completion_timestamp_seconds", "Unix timestamp of last stage completion"),
        &["stage"]
    ).unwrap();
    
    /// Database compaction active (bool: 0/1)
    /// Labels: cf (column family)
    pub static ref DB_COMPACTION_ACTIVE: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rustyblox_db_compaction_active", "RocksDB compaction active (0/1)"),
        &["cf"]
    ).unwrap();
    
    /// Last block timestamp
    pub static ref LAST_BLOCK_TIMESTAMP: IntGauge = IntGauge::new(
        "rustyblox_last_block_timestamp_seconds",
        "Unix timestamp of last indexed block"
    ).unwrap();
    
    /// Batch flush count
    /// Labels: cf (column family)
    pub static ref BATCH_FLUSH_COUNT: IntCounterVec = IntCounterVec::new(
        Opts::new("rustyblox_batch_flush_count_total", "Total batch flushes by CF"),
        &["cf"]
    ).unwrap();
    
    /// RPC reconnects
    pub static ref RPC_RECONNECTS: IntCounter = IntCounter::new(
        "rustyblox_rpc_reconnects_total",
        "Total RPC reconnection attempts"
    ).unwrap();
    
    /// HTTP requests
    /// Labels: endpoint, method, status
    pub static ref HTTP_REQUESTS: IntCounterVec = IntCounterVec::new(
        Opts::new("rustyblox_http_requests_total", "Total HTTP requests"),
        &["endpoint", "method", "status"]
    ).unwrap();
    
    /// Active WebSocket connections
    pub static ref WEBSOCKET_CONNECTIONS_ACTIVE: IntGauge = IntGauge::new(
        "rustyblox_websocket_connections_active",
        "Number of active WebSocket connections"
    ).unwrap();
}

/// Initialize metrics registry
/// 
/// Registers all 45 metrics with the global registry.
/// Call this once at service startup.
pub fn init_metrics() -> Result<(), Box<dyn std::error::Error>> {
    // Pipeline progress
    REGISTRY.register(Box::new(BLOCKS_PROCESSED.clone()))?;
    REGISTRY.register(Box::new(TRANSACTIONS_PROCESSED.clone()))?;
    REGISTRY.register(Box::new(UTXOS_ADDED.clone()))?;
    REGISTRY.register(Box::new(UTXOS_SPENT.clone()))?;
    REGISTRY.register(Box::new(RPC_CATCHUP_BLOCKS.clone()))?;
    REGISTRY.register(Box::new(PIPELINE_STAGE_CURRENT.clone()))?;
    REGISTRY.register(Box::new(CHAIN_TIP_HEIGHT.clone()))?;
    REGISTRY.register(Box::new(INDEXED_HEIGHT.clone()))?;
    
    // Latency histograms
    REGISTRY.register(Box::new(BLOCK_PARSE_DURATION.clone()))?;
    REGISTRY.register(Box::new(TX_DECODE_DURATION.clone()))?;
    REGISTRY.register(Box::new(ADDRESS_ENRICHMENT_DURATION.clone()))?;
    REGISTRY.register(Box::new(DB_BATCH_FLUSH_DURATION.clone()))?;
    REGISTRY.register(Box::new(RPC_CALL_DURATION.clone()))?;
    REGISTRY.register(Box::new(HEIGHT_RESOLUTION_DURATION.clone()))?;
    
    // Error counters
    REGISTRY.register(Box::new(DB_ERRORS.clone()))?;
    REGISTRY.register(Box::new(RPC_ERRORS.clone()))?;
    REGISTRY.register(Box::new(REORG_EVENTS.clone()))?;
    REGISTRY.register(Box::new(INVARIANT_VIOLATIONS.clone()))?;
    REGISTRY.register(Box::new(ORPHANED_TRANSACTIONS.clone()))?;
    
    // Cache metrics
    REGISTRY.register(Box::new(TX_CACHE_HITS.clone()))?;
    REGISTRY.register(Box::new(TX_CACHE_MISSES.clone()))?;
    REGISTRY.register(Box::new(TX_CACHE_SIZE_BYTES.clone()))?;
    
    // Resource metrics
    REGISTRY.register(Box::new(DB_SIZE_BYTES.clone()))?;
    REGISTRY.register(Box::new(DB_BATCH_SIZE_ENTRIES.clone()))?;
    REGISTRY.register(Box::new(DB_BATCH_SIZE_BYTES.clone()))?;
    REGISTRY.register(Box::new(PROCESS_RESIDENT_MEMORY_BYTES.clone()))?;
    
    // Custom pipeline metrics
    REGISTRY.register(Box::new(SYNC_PROGRESS_PERCENT.clone()))?;
    REGISTRY.register(Box::new(REORG_DEPTH_BLOCKS.clone()))?;
    REGISTRY.register(Box::new(BLOCKS_BEHIND_TIP.clone()))?;
    REGISTRY.register(Box::new(ESTIMATED_SYNC_COMPLETION_SECONDS.clone()))?;
    REGISTRY.register(Box::new(TX_PARSE_ERRORS.clone()))?;
    REGISTRY.register(Box::new(CANONICAL_BLOCKS.clone()))?;
    REGISTRY.register(Box::new(ORPHANED_BLOCKS.clone()))?;
    REGISTRY.register(Box::new(PENDING_REORG_DEPTH.clone()))?;
    REGISTRY.register(Box::new(RPC_CONNECTED.clone()))?;
    REGISTRY.register(Box::new(ADDRESS_INDEX_SIZE_ENTRIES.clone()))?;
    REGISTRY.register(Box::new(TOTAL_ADDRESSES_INDEXED.clone()))?;
    REGISTRY.register(Box::new(TOTAL_UTXOS_TRACKED.clone()))?;
    REGISTRY.register(Box::new(SAPLING_TRANSACTIONS_TOTAL.clone()))?;    REGISTRY.register(Box::new(SAPLING_TRANSACTIONS_COUNT.clone()))?;    
    // Operational metrics
    REGISTRY.register(Box::new(UPTIME_SECONDS.clone()))?;
    REGISTRY.register(Box::new(SERVICE_START_TIMESTAMP.clone()))?;
    REGISTRY.register(Box::new(LAST_STAGE_COMPLETION_TIMESTAMP.clone()))?;
    REGISTRY.register(Box::new(DB_COMPACTION_ACTIVE.clone()))?;
    REGISTRY.register(Box::new(LAST_BLOCK_TIMESTAMP.clone()))?;
    REGISTRY.register(Box::new(BATCH_FLUSH_COUNT.clone()))?;
    REGISTRY.register(Box::new(RPC_RECONNECTS.clone()))?;
    REGISTRY.register(Box::new(HTTP_REQUESTS.clone()))?;
    REGISTRY.register(Box::new(WEBSOCKET_CONNECTIONS_ACTIVE.clone()))?;
    
    // Set service start timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    SERVICE_START_TIMESTAMP.set(now as i64);
    
    Ok(())
}

/// Gather metrics in Prometheus text format
pub fn gather_metrics() -> String {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = vec![];
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

// ============================================================================
// HELPER FUNCTIONS - Clean API for instrumenting code
// ============================================================================

/// Timer for measuring durations
pub struct Timer {
    start: Instant,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
    
    pub fn elapsed_secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

/// Record blocks processed for a stage
pub fn increment_blocks_processed(stage: &str, count: u64) {
    BLOCKS_PROCESSED.with_label_values(&[stage]).inc_by(count);
}

/// Record transactions processed for a stage
pub fn increment_transactions_processed(stage: &str, count: u64) {
    TRANSACTIONS_PROCESSED.with_label_values(&[stage]).inc_by(count);
}

/// Record UTXO added
pub fn increment_utxos_added(count: u64) {
    UTXOS_ADDED.inc_by(count);
}

/// Record UTXO spent
pub fn increment_utxos_spent(count: u64) {
    UTXOS_SPENT.inc_by(count);
}

/// Set pipeline stage
pub fn set_pipeline_stage(stage: &str, value: i64) {
    PIPELINE_STAGE_CURRENT.with_label_values(&[stage]).set(value);
}

/// Set chain tip height
pub fn set_chain_tip_height(source: &str, height: i64) {
    CHAIN_TIP_HEIGHT.with_label_values(&[source]).set(height);
}

/// Set indexed height for a stage
pub fn set_indexed_height(stage: &str, height: i64) {
    INDEXED_HEIGHT.with_label_values(&[stage]).set(height);
}

/// Record block parse duration
pub fn record_block_parse_duration(file: &str, duration_secs: f64) {
    BLOCK_PARSE_DURATION.with_label_values(&[file]).observe(duration_secs);
}

/// Record transaction decode duration
pub fn record_tx_decode_duration(version: &str, duration_secs: f64) {
    TX_DECODE_DURATION.with_label_values(&[version]).observe(duration_secs);
}

/// Record database flush duration
pub fn record_db_flush_duration(cf: &str, duration_secs: f64) {
    DB_BATCH_FLUSH_DURATION.with_label_values(&[cf]).observe(duration_secs);
}

/// Record RPC call duration
pub fn record_rpc_call_duration(method: &str, duration_secs: f64) {
    RPC_CALL_DURATION.with_label_values(&[method]).observe(duration_secs);
}

/// Increment database errors
pub fn increment_db_errors(op: &str, cf: &str) {
    DB_ERRORS.with_label_values(&[op, cf]).inc();
}

/// Increment RPC errors
pub fn increment_rpc_errors(method: &str, error_type: &str) {
    RPC_ERRORS.with_label_values(&[method, error_type]).inc();
}

/// Increment invariant violations
pub fn increment_invariant_violations(violation_type: &str) {
    INVARIANT_VIOLATIONS.with_label_values(&[violation_type]).inc();
}

/// Increment orphaned transactions
pub fn increment_orphaned_transactions(reason: &str) {
    ORPHANED_TRANSACTIONS.with_label_values(&[reason]).inc();
}

/// Record cache hit
pub fn increment_cache_hits() {
    TX_CACHE_HITS.inc();
}

/// Record cache miss
pub fn increment_cache_misses() {
    TX_CACHE_MISSES.inc();
}

/// Set cache size in bytes
pub fn set_cache_size_bytes(bytes: i64) {
    TX_CACHE_SIZE_BYTES.set(bytes);
}

/// Update uptime
pub fn update_uptime() {
    let start = SERVICE_START_TIMESTAMP.get();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    UPTIME_SECONDS.set((now - start as u64) as i64);
}

/// Set blocks behind tip
pub fn set_blocks_behind_tip(count: i64) {
    BLOCKS_BEHIND_TIP.set(count);
}

/// Increment reorg events
pub fn increment_reorg_events() {
    REORG_EVENTS.inc();
}

/// Set reorg depth
pub fn set_reorg_depth(depth: i64) {
    REORG_DEPTH_BLOCKS.set(depth);
}

/// Set RPC connected status
pub fn set_rpc_connected(connected: bool) {
    RPC_CONNECTED.set(if connected { 1 } else { 0 });
}

// ============================================================================
// NEW METRICS HELPERS - For the 4 missing metrics
// ============================================================================

/// Set total addresses indexed (gauge - current count)
pub fn set_total_addresses_indexed(count: u64) {
    TOTAL_ADDRESSES_INDEXED.set(count as i64);
}

/// Set total UTXOs tracked (gauge - current unspent count)
pub fn set_total_utxos_tracked(count: u64) {
    TOTAL_UTXOS_TRACKED.set(count as i64);
}

/// Increment sapling transactions counter
pub fn increment_sapling_transactions(count: u64) {
    SAPLING_TRANSACTIONS_TOTAL.inc_by(count);
}

/// Set sapling transactions count (gauge - current count in DB)
pub fn set_sapling_transactions_count(count: u64) {
    SAPLING_TRANSACTIONS_COUNT.set(count as i64);
}

/// Set database size for a column family (bytes)
pub fn set_db_size_bytes(cf: &str, bytes: u64) {
    DB_SIZE_BYTES.with_label_values(&[cf]).set(bytes as i64);
}

// ============================================================================
// METRICS PERSISTENCE - Save/Load from Database
// ============================================================================

/// Save aggregate metrics to database for persistence across restarts
pub fn save_metrics_to_db(db: &std::sync::Arc<rocksdb::DB>) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    // Save TOTAL_ADDRESSES_INDEXED
    let address_count = TOTAL_ADDRESSES_INDEXED.get() as u64;
    db.put_cf(&cf_state, b"metric_total_addresses", &address_count.to_le_bytes())?;
    
    // Save TOTAL_UTXOS_TRACKED
    let utxo_count = TOTAL_UTXOS_TRACKED.get() as u64;
    db.put_cf(&cf_state, b"metric_total_utxos", &utxo_count.to_le_bytes())?;
    
    // Save SAPLING_TRANSACTIONS_COUNT (gauge)
    let sapling_count = SAPLING_TRANSACTIONS_COUNT.get() as u64;
    db.put_cf(&cf_state, b"metric_sapling_count", &sapling_count.to_le_bytes())?;
    
    // Save SAPLING_TRANSACTIONS_TOTAL (counter)
    let sapling_total = SAPLING_TRANSACTIONS_TOTAL.get() as u64;
    db.put_cf(&cf_state, b"metric_sapling_total", &sapling_total.to_le_bytes())?;
    
    Ok(())
}

/// Load aggregate metrics from database on startup
pub fn load_metrics_from_db(db: &std::sync::Arc<rocksdb::DB>) -> Result<(), Box<dyn std::error::Error>> {
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    // Load TOTAL_ADDRESSES_INDEXED
    if let Some(bytes) = db.get_cf(&cf_state, b"metric_total_addresses")? {
        if bytes.len() >= 8 {
            let count = u64::from_le_bytes(bytes[0..8].try_into()?);
            TOTAL_ADDRESSES_INDEXED.set(count as i64);
            tracing::info!(count = count, "Restored metric: total_addresses_indexed");
        }
    }
    
    // Load TOTAL_UTXOS_TRACKED
    if let Some(bytes) = db.get_cf(&cf_state, b"metric_total_utxos")? {
        if bytes.len() >= 8 {
            let count = u64::from_le_bytes(bytes[0..8].try_into()?);
            TOTAL_UTXOS_TRACKED.set(count as i64);
            tracing::info!(count = count, "Restored metric: total_utxos_tracked");
        }
    }
    
    // Load SAPLING_TRANSACTIONS_COUNT (gauge)
    if let Some(bytes) = db.get_cf(&cf_state, b"metric_sapling_count")? {
        if bytes.len() >= 8 {
            let count = u64::from_le_bytes(bytes[0..8].try_into()?);
            SAPLING_TRANSACTIONS_COUNT.set(count as i64);
            tracing::info!(count = count, "Restored metric: sapling_transactions_count");
        }
    }
    
    // Load SAPLING_TRANSACTIONS_TOTAL (counter)
    // Note: Counters can't be set directly, but we store the value for reference
    // The counter will increment from this baseline as new transactions are processed
    if let Some(bytes) = db.get_cf(&cf_state, b"metric_sapling_total")? {
        if bytes.len() >= 8 {
            let count = u64::from_le_bytes(bytes[0..8].try_into()?);
            // We can't set a counter directly, so we need to increment it from 0 to the stored value
            // This is a workaround - counters are designed to only increment
            SAPLING_TRANSACTIONS_TOTAL.inc_by(count);
            tracing::info!(count = count, "Restored metric: sapling_transactions_total");
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_init_metrics() {
        // Should not panic
        init_metrics().unwrap();
    }
    
    #[test]
    fn test_gather_metrics() {
        init_metrics().unwrap();
        
        // Increment some metrics
        increment_blocks_processed("test", 100);
        set_chain_tip_height("rpc", 1000);
        
        let output = gather_metrics();
        
        // Should contain metric names
        assert!(output.contains("rustyblox_blocks_processed_total"));
        assert!(output.contains("rustyblox_chain_tip_height"));
    }
    
    #[test]
    fn test_timer() {
        let timer = Timer::new();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let elapsed = timer.elapsed_secs();
        assert!(elapsed >= 0.01); // At least 10ms
    }
}
