# Live Metrics Examples from rustyblox

**Captured**: January 12, 2026  
**Endpoint**: http://localhost:3005/metrics  
**Current Sync Status**: Block 5,240,078 / 5,240,193 (99.98% synced)

---

## 📊 Key Metrics Overview

### Chain Status
```prometheus
# Current indexed height
rustyblox_indexed_height{stage="block_index"} 5240078

# Network tip height (from RPC)
rustyblox_chain_tip_height{source="rpc"} 5240193

# Blocks behind tip
rustyblox_blocks_behind_tip 0

# RPC connection status (0=disconnected, 1=connected)
rustyblox_rpc_connected 0
```

### Processing Progress
```prometheus
# Total blocks processed by stage
rustyblox_blocks_processed_total{stage="leveldb_import"} 15720107

# Canonical blocks in database
rustyblox_canonical_blocks_total 0

# Orphaned blocks encountered
rustyblox_orphaned_blocks_total 0

# Pipeline stage (0-7, where 4 = enrichment)
rustyblox_pipeline_stage_current{stage="current"} 4
```

### Performance Metrics

#### Database Flush Latency (chain_metadata CF)
```prometheus
# Histogram buckets showing distribution
rustyblox_db_batch_flush_duration_seconds_bucket{cf="chain_metadata",le="0.001"} 8
rustyblox_db_batch_flush_duration_seconds_bucket{cf="chain_metadata",le="0.005"} 29
rustyblox_db_batch_flush_duration_seconds_bucket{cf="chain_metadata",le="0.01"} 127
rustyblox_db_batch_flush_duration_seconds_bucket{cf="chain_metadata",le="0.05"} 3983
rustyblox_db_batch_flush_duration_seconds_bucket{cf="chain_metadata",le="0.1"} 3990
rustyblox_db_batch_flush_duration_seconds_bucket{cf="chain_metadata",le="0.5"} 3991
rustyblox_db_batch_flush_duration_seconds_bucket{cf="chain_metadata",le="1"} 3991
rustyblox_db_batch_flush_duration_seconds_bucket{cf="chain_metadata",le="5"} 4016
rustyblox_db_batch_flush_duration_seconds_bucket{cf="chain_metadata",le="+Inf"} 4016

# Summary statistics
rustyblox_db_batch_flush_duration_seconds_sum{cf="chain_metadata"} 154.218
rustyblox_db_batch_flush_duration_seconds_count{cf="chain_metadata"} 4016

# Average flush time: 154.218 / 4016 = ~38ms per flush
```

#### RPC Call Latency (getblockcount method)
```prometheus
rustyblox_rpc_call_duration_seconds_bucket{method="getblockcount",le="0.001"} 293
rustyblox_rpc_call_duration_seconds_bucket{method="getblockcount",le="0.005"} 588
rustyblox_rpc_call_duration_seconds_bucket{method="getblockcount",le="0.01"} 590
rustyblox_rpc_call_duration_seconds_bucket{method="getblockcount",le="0.05"} 594
rustyblox_rpc_call_duration_seconds_bucket{method="getblockcount",le="0.1"} 639
rustyblox_rpc_call_duration_seconds_bucket{method="getblockcount",le="0.5"} 643
rustyblox_rpc_call_duration_seconds_bucket{method="getblockcount",le="+Inf"} 644

# Summary
rustyblox_rpc_call_duration_seconds_sum{method="getblockcount"} 6.651
rustyblox_rpc_call_duration_seconds_count{method="getblockcount"} 644

# Average: ~10ms per call
# P95: < 100ms (639/644 calls under 100ms)
```

#### RPC Call Latency (getblockhash method)
```prometheus
# Most calls are in 0.1-0.5s range (797 out of 824)
rustyblox_rpc_call_duration_seconds_bucket{method="getblockhash",le="0.1"} 27
rustyblox_rpc_call_duration_seconds_bucket{method="getblockhash",le="0.5"} 824
rustyblox_rpc_call_duration_seconds_sum{method="getblockhash"} 85.621
rustyblox_rpc_call_duration_seconds_count{method="getblockhash"} 824

# Average: ~104ms per call (slower due to disk seeks)
```

### Database Statistics
```prometheus
# Batch flush counts by column family
rustyblox_batch_flush_count_total{cf="chain_metadata"} 4016
rustyblox_batch_flush_count_total{cf="transactions"} 4016

# Current batch sizes (entries pending)
rustyblox_db_batch_size_entries{cf="chain_metadata"} 2598
rustyblox_db_batch_size_entries{cf="transactions"} 3635
```

### Cache Metrics
```prometheus
# Transaction cache (currently empty in early sync)
rustyblox_tx_cache_hits_total 0
rustyblox_tx_cache_misses_total 0
rustyblox_tx_cache_size_bytes 0

# Address index (not yet populated)
rustyblox_address_index_size_entries 0
```

### Error Tracking
```prometheus
# RPC errors encountered
rustyblox_rpc_errors_total{error_type="connection",method="getblockcount"} 2

# RPC reconnection attempts
rustyblox_rpc_reconnects_total 0
```

### Reorg Detection
```prometheus
# Total reorg events detected
rustyblox_reorg_events_total 0

# Depth of most recent reorg
rustyblox_reorg_depth_blocks 0

# Pending reorg depth (if any)
rustyblox_pending_reorg_depth 0
```

### Service Health
```prometheus
# Service start timestamp (Unix time)
rustyblox_service_start_timestamp_seconds 1768209024

# Uptime in seconds
rustyblox_uptime_seconds 3468

# Process memory (placeholder - not yet implemented)
rustyblox_process_resident_memory_bytes 0

# Estimated time to sync completion
rustyblox_estimated_sync_completion_seconds 0
```

### RPC Catchup Activity
```prometheus
# Blocks processed via RPC catchup mechanism
rustyblox_rpc_catchup_blocks_total 180
```

---

## 📈 Prometheus Query Examples

### Calculate Block Processing Rate (last 5 minutes)
```promql
rate(rustyblox_blocks_processed_total[5m])
```

### Calculate P95 DB Flush Latency
```promql
histogram_quantile(0.95, 
  rate(rustyblox_db_batch_flush_duration_seconds_bucket[5m])
)
```

### Calculate P99 RPC Call Latency
```promql
histogram_quantile(0.99, 
  rate(rustyblox_rpc_call_duration_seconds_bucket{method="getblockcount"}[5m])
)
```

### Sync Progress Percentage
```promql
(rustyblox_indexed_height / rustyblox_chain_tip_height) * 100
```

### Average Batch Size
```promql
avg(rustyblox_db_batch_size_entries)
```

### Cache Hit Rate
```promql
rate(rustyblox_tx_cache_hits_total[5m]) / 
(rate(rustyblox_tx_cache_hits_total[5m]) + rate(rustyblox_tx_cache_misses_total[5m]))
```

### Error Rate by Type
```promql
sum by (error_type) (rate(rustyblox_rpc_errors_total[5m]))
```

### Service Uptime
```promql
time() - rustyblox_service_start_timestamp_seconds
```

---

## 🔍 Interpretation

### Current State Analysis

**Sync Status**: 
- Indexed: 5,240,078 blocks
- Network tip: 5,240,193 blocks
- Behind: 115 blocks (~29 minutes at 2-minute block times)
- Progress: 99.98%

**Performance**:
- DB flush latency: ~38ms average (excellent)
- Most flushes (3983/4016) complete under 50ms
- RPC getblockcount: ~10ms average (very fast)
- RPC getblockhash: ~104ms average (reasonable for disk-bound operation)

**Health**:
- No reorgs detected (stable chain)
- Only 2 RPC connection errors out of 644 calls (99.7% reliability)
- No reconnection attempts needed
- Uptime: 58 minutes

**Bottlenecks**:
- RPC getblockhash is the slowest operation at ~104ms
- Batch size shows 2598-3635 entries pending (moderate queue)
- Cache not yet active (early in sync, caching disabled or warming up)

---

## 🎯 Dashboard Panel Queries

These queries are used in the Grafana dashboard:

**Sync Progress Gauge**:
```promql
(rustyblox_indexed_height{stage="block_index"} / rustyblox_chain_tip_height{source="rpc"}) * 100
```

**Blocks Behind Stat**:
```promql
rustyblox_blocks_behind_tip
```

**Block Processing Rate**:
```promql
rate(rustyblox_blocks_processed_total[1m])
```

**DB Flush P95 Latency**:
```promql
histogram_quantile(0.95, sum by (le, cf) (rate(rustyblox_db_batch_flush_duration_seconds_bucket[5m])))
```

**Error Rate Stacked Graph**:
```promql
sum by (error_type) (rate(rustyblox_rpc_errors_total[5m]))
```

---

## 📝 Notes

- **Histogram Buckets**: The `le` (less than or equal) labels define bucket boundaries
- **Counter Metrics**: Always increase, use `rate()` to get per-second values
- **Gauge Metrics**: Can go up or down, represent current state
- **Summary Stats**: Use `_sum` and `_count` to calculate averages: `sum/count`
- **Percentiles**: Use `histogram_quantile()` with histogram buckets

See [METRICS_CATALOG.md](../METRICS_CATALOG.md) for complete metric definitions.
