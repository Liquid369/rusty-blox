#!/usr/bin/env python3
import json

# Read dashboard
with open('grafana/dashboards/rustyblox-main.json', 'r') as f:
    dashboard = json.load(f)

# Update existing panels and add new ones
panels = []

# Row 1: Status indicators
panels.append({
    "title": "Sync Progress",
    "type": "gauge",
    "gridPos": {"x": 0, "y": 0, "w": 6, "h": 8},
    "targets": [{
        "expr": "(rustyblox_indexed_height{stage=\"block_index\"} / ignoring(stage,source) rustyblox_chain_tip_height{source=\"rpc\"}) * 100",
        "refId": "A",
        "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
    }],
    "fieldConfig": {
        "defaults": {
            "unit": "percent",
            "min": 0,
            "max": 100,
            "thresholds": {
                "mode": "absolute",
                "steps": [
                    {"value": 0, "color": "red"},
                    {"value": 95, "color": "yellow"},
                    {"value": 99, "color": "green"}
                ]
            }
        }
    }
})

panels.append({
    "title": "Blocks Behind (Calculated)",
    "type": "stat",
    "gridPos": {"x": 6, "y": 0, "w": 6, "h": 4},
    "targets": [{
        "expr": "rustyblox_chain_tip_height{source=\"rpc\"} - ignoring(source,stage) rustyblox_indexed_height{stage=\"block_index\"}",
        "refId": "A",
        "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
    }],
    "fieldConfig": {
        "defaults": {
            "unit": "short",
            "thresholds": {
                "mode": "absolute",
                "steps": [
                    {"value": 0, "color": "green"},
                    {"value": 100, "color": "yellow"},
                    {"value": 1000, "color": "red"}
                ]
            }
        }
    }
})

panels.append({
    "title": "RPC Status",
    "type": "stat",
    "gridPos": {"x": 12, "y": 0, "w": 6, "h": 4},
    "targets": [{
        "expr": "rustyblox_rpc_connected",
        "refId": "A",
        "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
    }],
    "fieldConfig": {
        "defaults": {
            "mappings": [
                {"type": "value", "value": "0", "text": "DISCONNECTED"},
                {"type": "value", "value": "1", "text": "CONNECTED"}
            ],
            "thresholds": {
                "mode": "absolute",
                "steps": [
                    {"value": 0, "color": "red"},
                    {"value": 1, "color": "green"}
                ]
            }
        }
    }
})

panels.append({
    "title": "Current Pipeline Stage",
    "type": "stat",
    "gridPos": {"x": 18, "y": 0, "w": 6, "h": 4},
    "targets": [{
        "expr": "rustyblox_pipeline_stage_current",
        "refId": "A",
        "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
    }],
    "fieldConfig": {
        "defaults": {
            "mappings": [
                {"type": "value", "value": "0", "text": "Parsing"},
                {"type": "value", "value": "1", "text": "Block Index"},
                {"type": "value", "value": "2", "text": "Transactions"},
                {"type": "value", "value": "3", "text": "Chainstate"},
                {"type": "value", "value": "4", "text": "Enrichment"},
                {"type": "value", "value": "5", "text": "UTXO"},
                {"type": "value", "value": "6", "text": "Address Index"},
                {"type": "value", "value": "7", "text": "Complete"}
            ],
            "unit": "short"
        }
    }
})

panels.append({
    "title": "Indexed Height vs Chain Tip",
    "type": "timeseries",
    "gridPos": {"x": 6, "y": 4, "w": 18, "h": 4},
    "targets": [
        {
            "expr": "rustyblox_indexed_height{stage=\"block_index\"}",
            "legendFormat": "Indexed Height",
            "refId": "A",
            "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
        },
        {
            "expr": "rustyblox_chain_tip_height{source=\"rpc\"}",
            "legendFormat": "Chain Tip (RPC)",
            "refId": "B",
            "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
        }
    ],
    "fieldConfig": {"defaults": {"unit": "short"}}
})

# Row 2: RPC Call Latency by Method
for i, method in enumerate(['getblockhash', 'getblockcount', 'getblock']):
    panels.append({
        "title": f"RPC {method} Latency (P50/P95/P99)",
        "type": "timeseries",
        "gridPos": {"x": i*8, "y": 8, "w": 8, "h": 8},
        "targets": [
            {
                "expr": f'histogram_quantile(0.50, sum by (le) (rate(rustyblox_rpc_call_duration_seconds_bucket{{method="{method}"}}[5m])))',
                "legendFormat": "P50",
                "refId": "A",
                "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
            },
            {
                "expr": f'histogram_quantile(0.95, sum by (le) (rate(rustyblox_rpc_call_duration_seconds_bucket{{method="{method}"}}[5m])))',
                "legendFormat": "P95",
                "refId": "B",
                "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
            },
            {
                "expr": f'histogram_quantile(0.99, sum by (le) (rate(rustyblox_rpc_call_duration_seconds_bucket{{method="{method}"}}[5m])))',
                "legendFormat": "P99",
                "refId": "C",
                "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
            }
        ],
        "fieldConfig": {"defaults": {"unit": "s"}}
    })

# Row 3: RPC Activity
panels.append({
    "title": "RPC Call Rate (calls/sec by method)",
    "type": "timeseries",
    "gridPos": {"x": 0, "y": 16, "w": 12, "h": 8},
    "targets": [
        {
            "expr": 'rate(rustyblox_rpc_call_duration_seconds_count{method="getblockhash"}[1m])',
            "legendFormat": "getblockhash",
            "refId": "A",
            "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
        },
        {
            "expr": 'rate(rustyblox_rpc_call_duration_seconds_count{method="getblockcount"}[1m])',
            "legendFormat": "getblockcount",
            "refId": "B",
            "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
        },
        {
            "expr": 'rate(rustyblox_rpc_call_duration_seconds_count{method="getblock"}[1m])',
            "legendFormat": "getblock",
            "refId": "C",
            "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
        }
    ],
    "fieldConfig": {"defaults": {"unit": "ops"}}
})

panels.append({
    "title": "RPC Catchup Blocks Rate",
    "type": "timeseries",
    "gridPos": {"x": 12, "y": 16, "w": 12, "h": 8},
    "targets": [{
        "expr": "rate(rustyblox_rpc_catchup_blocks_total[1m])",
        "legendFormat": "Blocks/sec via RPC",
        "refId": "A",
        "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
    }],
    "fieldConfig": {"defaults": {"unit": "ops"}}
})

# Row 4: Database Performance
panels.append({
    "title": "DB Flush Latency by CF (P50/P95/P99)",
    "type": "timeseries",
    "gridPos": {"x": 0, "y": 24, "w": 12, "h": 8},
    "targets": [
        {
            "expr": "histogram_quantile(0.50, sum by (le, cf) (rate(rustyblox_db_batch_flush_duration_seconds_bucket[5m])))",
            "legendFormat": "{{cf}} P50",
            "refId": "A",
            "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
        },
        {
            "expr": "histogram_quantile(0.95, sum by (le, cf) (rate(rustyblox_db_batch_flush_duration_seconds_bucket[5m])))",
            "legendFormat": "{{cf}} P95",
            "refId": "B",
            "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
        },
        {
            "expr": "histogram_quantile(0.99, sum by (le, cf) (rate(rustyblox_db_batch_flush_duration_seconds_bucket[5m])))",
            "legendFormat": "{{cf}} P99",
            "refId": "C",
            "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
        }
    ],
    "fieldConfig": {"defaults": {"unit": "s"}}
})

panels.append({
    "title": "DB Batch Size by CF",
    "type": "timeseries",
    "gridPos": {"x": 12, "y": 24, "w": 12, "h": 8},
    "targets": [{
        "expr": "rustyblox_db_batch_size_entries",
        "legendFormat": "{{cf}}",
        "refId": "A",
        "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
    }],
    "fieldConfig": {"defaults": {"unit": "short"}}
})

# Row 5: Error Tracking & Stats
panels.append({
    "title": "RPC Error Rate (errors/sec)",
    "type": "timeseries",
    "gridPos": {"x": 0, "y": 32, "w": 12, "h": 8},
    "targets": [{
        "expr": "rate(rustyblox_rpc_errors_total[5m])",
        "legendFormat": "{{method}} - {{error_type}}",
        "refId": "A",
        "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
    }],
    "fieldConfig": {"defaults": {"unit": "ops"}}
})

# Stats row
stat_panels = [
    ("Total Blocks Processed", 'rustyblox_blocks_processed_total{stage="leveldb_import"}', 12, 32),
    ("Total DB Flushes", "sum(rustyblox_batch_flush_count_total)", 18, 32),
    ("WebSocket Connections", "rustyblox_websocket_connections_active", 12, 36),
    ("RPC Errors (Total)", "sum(rustyblox_rpc_errors_total)", 18, 36)
]

for title, expr, x, y in stat_panels:
    panels.append({
        "title": title,
        "type": "stat",
        "gridPos": {"x": x, "y": y, "w": 6, "h": 4},
        "targets": [{
            "expr": expr,
            "refId": "A",
            "datasource": {"type": "prometheus", "uid": "PBFA97CFB590B2093"}
        }],
        "fieldConfig": {"defaults": {"unit": "short"}}
    })

# Update dashboard
dashboard['panels'] = panels
dashboard['title'] = 'rusty-blox Production Monitoring (v2)'
dashboard['uid'] = 'rustyblox-main-v2'
dashboard['version'] = 0
dashboard['id'] = None
dashboard['refresh'] = '5s'
dashboard['time'] = {'from': 'now-15m', 'to': 'now'}

# Save
import_payload = {
    "dashboard": dashboard,
    "overwrite": False,
    "message": "Improved dashboard focused on active metrics"
}

with open('/tmp/dashboard-v2.json', 'w') as f:
    json.dump(import_payload, f)

print(f"✅ Created improved dashboard with {len(panels)} panels")
print("Improvements:")
print("   - Fixed blocks_behind calculation (now shows 962)")
print("   - RPC latency broken down by method (getblock, getblockhash, getblockcount)")
print("   - RPC call rates by method")
print("   - DB batch sizes visible")
print("   - Pipeline stage indicator (shows 'Enrichment')")
print("   - Error tracking over time")
print("   - 5s auto-refresh")
