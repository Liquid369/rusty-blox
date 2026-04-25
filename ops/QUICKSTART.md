# 🚀 Monitoring Stack - Quick Start

## ✅ Status

Your monitoring stack is **UP and RUNNING**:

- ✅ **Prometheus**: http://localhost:9091 (scraping rustyblox:3005)
- ✅ **Grafana**: http://localhost:3002
- ✅ **rustyblox**: http://localhost:3005 (API + /metrics)

## 📊 Access Grafana

1. **Open**: http://localhost:3002
2. **Login**: 
   - Username: `admin`
   - Password: `rustyblox_admin`

3. **Add Data Source**:
   - Configuration → Data Sources → Add data source → Prometheus
   - URL: `http://localhost:9091`
   - Click "Save & Test"

4. **Import Dashboard**:
   - Dashboards → Import → Upload JSON file
   - Select: `ops/grafana/dashboards/rustyblox-main.json`
   - Select Prometheus data source
   - Click "Import"

## 🔍 Verify Everything Works

```bash
# Check Prometheus is scraping
curl http://localhost:9091/api/v1/targets | grep "rustyblox"
# Should show: "health": "up"

# Check metrics are flowing
curl http://localhost:3005/metrics | grep rustyblox_indexed_height

# Check Grafana health
curl http://localhost:3002/api/health
```

## 🛑 Stop Services

```bash
cd ops
./stop-monitoring.sh
```

## 🔄 Restart Services

```bash
cd ops
./start-monitoring.sh
```

## 📁 Important Files

- **Dashboard**: [ops/grafana/dashboards/rustyblox-main.json](grafana/dashboards/rustyblox-main.json)
- **Metrics**: [METRICS_CATALOG.md](../METRICS_CATALOG.md) - All 45 metrics documented
- **Alerts**: [ops/prometheus/alerts.yml](prometheus/alerts.yml) - 5 critical alerts
- **Config**: [ops/prometheus/prometheus.yml](prometheus/prometheus.yml)

## 📈 Dashboard Features

### Status Row (4 panels)
- Sync progress gauge (0-100%)
- Blocks behind chain tip
- RPC connection status
- Current indexed height

### Performance (2 panels)  
- Block processing rate (blocks/sec)
- Transaction processing rate (tx/sec)

### Latency Analysis (3 panels)
- Block parse latency (p50/p95/p99)
- DB flush latency (p50/p95/p99)
- RPC call latency (p50/p95/p99)

### Health (4 panels)
- Error rate by type (DB, RPC, parsing, invariants)
- Cache hit rate percentage
- Reorg detection count
- Reorg depth histogram

## 🔧 Troubleshooting

**Q: Grafana shows "No Data"**  
A: Make sure rustyblox is running: `ps aux | grep rustyblox`

**Q: Prometheus target shows "Down"**  
A: Check rustyblox metrics: `curl http://localhost:3005/metrics`

**Q: Port conflicts**  
A: Check what's using ports:
```bash
lsof -i :3002  # Grafana
lsof -i :9091  # Prometheus
lsof -i :3005  # rustyblox
```

## 📦 Port Summary

- **3000**: Your existing Grafana (unchanged)
- **3001**: Frontend (unchanged)
- **3002**: **rustyblox Grafana** (new)
- **3005**: rustyblox API + metrics
- **9091**: **Prometheus** (new)

---

**Note**: The dashboard auto-refreshes every 5 seconds, so you'll see live updates as rustyblox processes blocks.
