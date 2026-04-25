# Grafana Dashboard Plan
## PIVX Blockchain Indexer (rusty-blox)

**Dashboard Title**: rusty-blox Production Monitoring  
**Refresh**: 30s  
**Time Range**: Last 6 hours (default)

---

## Layout Overview

```
┌──────────────────────────────────────────────────────────────┐
│                    HEADER ROW (Status)                       │
│  [Sync %]  [Behind Tip]  [RPC OK]  [Uptime]  [Last Block]  │
└──────────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────┐
│               ROW 1: SYNC PROGRESS (2 panels)                │
│  [ Sync Progress Gauge ]  [ Blocks Behind Tip Graph ]       │
└──────────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────┐
│              ROW 2: THROUGHPUT (2 panels)                    │
│  [ Blocks/sec Graph ]  [ Transactions/sec Graph ]           │
└──────────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────┐
│         ROW 3: LATENCY P50/P95 (3 panels)                    │
│  [ Block Parse ]  [ DB Flush ]  [ RPC Calls ]               │
└──────────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────┐
│              ROW 4: ERRORS (2 panels)                        │
│  [ Error Rate Graph ]  [ Error Details Table ]              │
└──────────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────┘
│              ROW 5: CACHE & RESOURCES (3 panels)             │
│  [ Cache Hit Rate ]  [ Memory Usage ]  [ DB Size ]          │
└──────────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────┐
│              ROW 6: REORGS & EVENTS (2 panels)               │
│  [ Reorg Count ]  [ Reorg Depth History ]                   │
└──────────────────────────────────────────────────────────────┘
```

---

## Panel Definitions

### HEADER ROW - Status Indicators (5 stat panels)

#### Panel 1: Sync Progress
- **Type**: Stat
- **Query**:
  ```promql
  (rustyblox_indexed_height{stage="block_index"} / rustyblox_chain_tip_height{source="rpc"}) * 100
  ```
- **Unit**: percent (0-100)
- **Thresholds**:
  - Red: < 90%
  - Yellow: 90-99%
  - Green: 99-100%
- **Title**: "Sync Progress"

#### Panel 2: Blocks Behind Tip
- **Type**: Stat
- **Query**:
  ```promql
  rustyblox_blocks_behind_tip
  ```
- **Unit**: blocks
- **Thresholds**:
  - Green: 0-10
  - Yellow: 11-100
  - Red: >100
- **Title**: "Behind Tip"

#### Panel 3: RPC Connected
- **Type**: Stat
- **Query**:
  ```promql
  rustyblox_rpc_connected
  ```
- **Value Mappings**:
  - 0 → "Disconnected" (Red)
  - 1 → "Connected" (Green)
- **Title**: "RPC Status"

#### Panel 4: Uptime
- **Type**: Stat
- **Query**:
  ```promql
  rustyblox_uptime_seconds
  ```
- **Unit**: seconds → duration (hh:mm:ss)
- **Title**: "Uptime"

#### Panel 5: Last Block Time
- **Type**: Stat
- **Query**:
  ```promql
  rustyblox_last_block_timestamp_seconds
  ```
- **Unit**: unix timestamp → datetime
- **Title**: "Last Block"

---

### ROW 1 - Sync Progress

#### Panel 1.1: Sync Progress Gauge
- **Type**: Gauge
- **Query**:
  ```promql
  (rustyblox_indexed_height{stage="block_index"} / rustyblox_chain_tip_height{source="rpc"}) * 100
  ```
- **Min**: 0
- **Max**: 100
- **Unit**: percent
- **Thresholds**:
  - 0-50: Red
  - 50-90: Yellow
  - 90-100: Green
- **Title**: "Overall Sync Progress"
- **Description**: "Percentage of blockchain indexed"

#### Panel 1.2: Blocks Behind Tip (Time Series)
- **Type**: Graph
- **Queries**:
  ```promql
  # Blocks behind
  rustyblox_blocks_behind_tip
  
  # Chain tip (reference line)
  rustyblox_chain_tip_height{source="rpc"}
  
  # Indexed height
  rustyblox_indexed_height{stage="block_index"}
  ```
- **Y-Axis**: Block height
- **Legend**: Right
- **Title**: "Sync Status"
- **Description**: "Gap between indexed height and chain tip"

---

### ROW 2 - Throughput

#### Panel 2.1: Blocks Per Second
- **Type**: Graph
- **Query**:
  ```promql
  rate(rustyblox_blocks_processed_total[1m])
  ```
- **Y-Axis**: blocks/sec
- **Legend**: Right (by stage)
- **Title**: "Block Processing Rate"
- **Description**: "Blocks indexed per second by stage"
- **Series**:
  - leveldb_import
  - parallel
  - rpc_catchup
  - address_enrich

#### Panel 2.2: Transactions Per Second
- **Type**: Graph
- **Query**:
  ```promql
  rate(rustyblox_transactions_processed_total[1m])
  ```
- **Y-Axis**: tx/sec
- **Legend**: Right (by stage)
- **Title**: "Transaction Processing Rate"
- **Description**: "Transactions processed per second"
- **Series**:
  - parse
  - index
  - enrich

---

### ROW 3 - Latency (P50 and P95)

#### Panel 3.1: Block Parse Latency
- **Type**: Graph
- **Queries**:
  ```promql
  # P50
  histogram_quantile(0.50, rate(rustyblox_block_parse_duration_seconds_bucket[5m]))
  
  # P95
  histogram_quantile(0.95, rate(rustyblox_block_parse_duration_seconds_bucket[5m]))
  
  # P99
  histogram_quantile(0.99, rate(rustyblox_block_parse_duration_seconds_bucket[5m]))
  ```
- **Y-Axis**: seconds (log scale)
- **Legend**: Right
- **Title**: "Block Parse Latency"
- **Threshold Line**: 5s (warning)

#### Panel 3.2: Database Flush Latency
- **Type**: Graph
- **Queries**:
  ```promql
  # P95 by CF
  histogram_quantile(0.95, sum by (le, cf) (rate(rustyblox_db_batch_flush_duration_seconds_bucket[5m])))
  ```
- **Y-Axis**: seconds
- **Legend**: Right (by CF)
- **Title**: "DB Flush Latency (P95)"
- **Threshold Line**: 30s (critical)
- **Description**: "Database batch flush time by column family"

#### Panel 3.3: RPC Call Latency
- **Type**: Graph
- **Queries**:
  ```promql
  # P95 by method
  histogram_quantile(0.95, sum by (le, method) (rate(rustyblox_rpc_call_duration_seconds_bucket[5m])))
  ```
- **Y-Axis**: seconds
- **Legend**: Right (by method)
- **Title**: "RPC Latency (P95)"
- **Threshold Line**: 10s (critical)
- **Description**: "RPC call duration by method"

---

### ROW 4 - Errors

#### Panel 4.1: Error Rate
- **Type**: Graph
- **Queries**:
  ```promql
  # Database errors
  sum(rate(rustyblox_db_errors_total[5m])) by (op, cf)
  
  # RPC errors
  sum(rate(rustyblox_rpc_errors_total[5m])) by (method, error_type)
  
  # Invariant violations
  rate(rustyblox_invariant_violations_total[5m])
  
  # Parse errors
  sum(rate(rustyblox_tx_parse_errors_total[5m])) by (error_type)
  ```
- **Y-Axis**: errors/sec
- **Legend**: Right
- **Title**: "Error Rates"
- **Alert Threshold**: 0.16 errors/sec (10/min)

#### Panel 4.2: Error Details (Table)
- **Type**: Table
- **Query**:
  ```promql
  # Top errors in last hour
  topk(20, increase(rustyblox_db_errors_total[1h]) > 0)
  or
  topk(20, increase(rustyblox_rpc_errors_total[1h]) > 0)
  or
  topk(20, increase(rustyblox_invariant_violations_total[1h]) > 0)
  ```
- **Columns**:
  - Metric Name
  - Labels (op, cf, method, error_type, type)
  - Count (last hour)
- **Sort**: By count descending
- **Title**: "Top Errors (Last Hour)"

---

### ROW 5 - Cache & Resources

#### Panel 5.1: Cache Hit Rate
- **Type**: Gauge
- **Query**:
  ```promql
  rate(rustyblox_tx_cache_hits_total[5m]) / 
  (rate(rustyblox_tx_cache_hits_total[5m]) + rate(rustyblox_tx_cache_misses_total[5m]))
  ```
- **Min**: 0
- **Max**: 1
- **Unit**: percentunit (0.0-1.0)
- **Thresholds**:
  - Red: < 0.70
  - Yellow: 0.70-0.90
  - Green: > 0.90
- **Title**: "TX Cache Hit Rate"
- **Description**: "Higher is better (>90% ideal)"

#### Panel 5.2: Memory Usage
- **Type**: Graph
- **Queries**:
  ```promql
  # Process memory
  rustyblox_process_resident_memory_bytes
  
  # Cache size
  rustyblox_tx_cache_size_bytes
  ```
- **Y-Axis**: bytes (IEC: GiB, MiB)
- **Legend**: Right
- **Title**: "Memory Usage"
- **Threshold Line**: 16 GiB (warning)

#### Panel 5.3: Database Size
- **Type**: Graph
- **Query**:
  ```promql
  rustyblox_db_size_bytes
  ```
- **Y-Axis**: bytes (IEC: GiB)
- **Legend**: Right (by CF)
- **Title**: "Database Size by CF"
- **Description**: "RocksDB column family sizes"

---

### ROW 6 - Reorgs & Events

#### Panel 6.1: Reorg Events
- **Type**: Stat
- **Query**:
  ```promql
  rustyblox_reorg_events_total
  ```
- **Unit**: count
- **Sparkline**: Enabled
- **Title**: "Total Reorgs"
- **Description**: "Cumulative blockchain reorganizations"

#### Panel 6.2: Reorg Depth History
- **Type**: Graph
- **Queries**:
  ```promql
  # Current reorg depth
  rustyblox_reorg_depth_blocks
  
  # Reorg rate
  rate(rustyblox_reorg_events_total[10m])
  ```
- **Y-Axis**: blocks
- **Legend**: Right
- **Title**: "Reorg Depth & Rate"
- **Threshold Line**: 10 blocks (warning for deep reorg)

---

## Variables (Dashboard-level)

Define these for filtering/drilling down:

### Variable: `stage`
- **Type**: Query
- **Query**:
  ```promql
  label_values(rustyblox_blocks_processed_total, stage)
  ```
- **Multi-value**: Yes
- **Include All**: Yes

### Variable: `cf` (Column Family)
- **Type**: Query
- **Query**:
  ```promql
  label_values(rustyblox_db_batch_flush_duration_seconds_bucket, cf)
  ```
- **Multi-value**: Yes
- **Include All**: Yes

### Variable: `method` (RPC Method)
- **Type**: Query
- **Query**:
  ```promql
  label_values(rustyblox_rpc_call_duration_seconds_bucket, method)
  ```
- **Multi-value**: Yes
- **Include All**: Yes

---

## Annotations

Enable these to mark significant events on graphs:

### Annotation: Service Restarts
- **Query**:
  ```promql
  changes(rustyblox_service_start_timestamp_seconds[1m]) > 0
  ```
- **Color**: Red
- **Icon**: Arrow down
- **Text**: "Service Restarted"

### Annotation: Reorg Events
- **Query**:
  ```promql
  changes(rustyblox_reorg_events_total[1m]) > 0
  ```
- **Color**: Orange
- **Icon**: Alert triangle
- **Text**: "Reorg Detected"

### Annotation: Invariant Violations
- **Query**:
  ```promql
  increase(rustyblox_invariant_violations_total[1m]) > 0
  ```
- **Color**: Red
- **Icon**: Exclamation
- **Text**: "INVARIANT VIOLATION"

---

## Dashboard Links

Add these links to dashboard header:

1. **Alerts** → Link to Prometheus alerts page
2. **Runbook** → Link to RUNBOOK.md on GitHub
3. **Metrics Catalog** → Link to METRICS_CATALOG.md
4. **Logs** → Link to Grafana Loki or log viewer

---

## Prometheus Data Source Configuration

```yaml
# prometheus.yml
global:
  scrape_interval: 15s
  evaluation_interval: 15s

scrape_configs:
  - job_name: 'rustyblox'
    scrape_interval: 15s
    scrape_timeout: 10s
    static_configs:
      - targets: ['localhost:3005']  # rusty-blox metrics endpoint
    metric_relabel_configs:
      # Drop go runtime metrics (if any)
      - source_labels: [__name__]
        regex: 'go_.*'
        action: drop

# Load alert rules
rule_files:
  - 'ops/prometheus/alerts.yml'
```

---

## Quick Start

### 1. Start Prometheus
```bash
prometheus --config.file=prometheus.yml
```

### 2. Access Prometheus UI
```
http://localhost:9090
```

### 3. Query metrics manually
```
http://localhost:9090/graph
```

Example queries:
- `rustyblox_blocks_processed_total`
- `rate(rustyblox_blocks_processed_total[5m])`
- `rustyblox_blocks_behind_tip`

### 4. Import Dashboard to Grafana

**Manual Import**:
1. Open Grafana → Dashboards → New → Import
2. Copy panel JSON from this plan (convert to JSON)
3. Select Prometheus data source
4. Save dashboard

**Automated Import** (TODO):
```bash
# Generate JSON from this plan
./tools/generate-dashboard.sh > ops/grafana/dashboard.json

# Import to Grafana API
curl -X POST http://admin:admin@localhost:3000/api/dashboards/db \
  -H "Content-Type: application/json" \
  -d @ops/grafana/dashboard.json
```

---

## Testing Dashboard

### Verify Metrics Appearing

1. Start rusty-blox:
   ```bash
   cargo run --release
   ```

2. Check metrics endpoint:
   ```bash
   curl http://localhost:3005/metrics | grep rustyblox
   ```

3. Verify in Prometheus:
   ```bash
   curl 'http://localhost:9090/api/v1/query?query=rustyblox_uptime_seconds'
   ```

4. Expected output:
   ```json
   {
     "status": "success",
     "data": {
       "resultType": "vector",
       "result": [{
         "metric": {"__name__": "rustyblox_uptime_seconds"},
         "value": [1704067200, "1234"]
       }]
     }
   }
   ```

### Trigger Test Alerts

Simulate conditions to test alerts:

1. **BlockProcessingStalled**:
   - Stop sync service
   - Wait 5 minutes
   - Alert should fire

2. **InvariantViolation**:
   - Manually increment metric (test mode):
     ```rust
     metrics::increment_invariant_violations("test");
     ```
   - Alert fires immediately

3. **HighRPCLatency**:
   - Stop pivxd or slow down network
   - RPC calls will timeout
   - Alert fires after 3 minutes

---

## Dashboard Maintenance

### Update Queries

When adding new metrics or changing metric names:

1. Update METRICS_CATALOG.md (source of truth)
2. Update alerts.yml
3. Update this dashboard plan
4. Regenerate dashboard JSON
5. Re-import to Grafana

### Dashboard Versions

Keep dashboard versions in git:
- `ops/grafana/dashboard-v1.0.json`
- `ops/grafana/dashboard-v1.1.json`

Export from Grafana UI: Settings → JSON Model → Copy

---

## Additional Dashboards (Future)

### Operator Dashboard (Simplified)
- Focus: Single-screen overview for on-call
- Panels: Status indicators, error count, last 5 alerts
- Auto-refresh: 10s

### Developer Dashboard (Detailed)
- Focus: Deep dive for debugging
- Panels: All histogram buckets, per-CF metrics, cache internals
- Time range: Last 24 hours

### Executive Dashboard (Summary)
- Focus: Weekly reports, capacity planning
- Panels: Uptime percentage, total blocks, weekly error budget
- Time range: Last 30 days

---

**Dashboard Plan Complete**  
**Panels**: 20 total (5 status + 15 graphs/gauges)  
**Queries**: 45+ PromQL expressions  
**Next Step**: Convert to Grafana JSON or build using Grafana UI

For JSON generation: See https://grafana.com/docs/grafana/latest/dashboards/json-model/
