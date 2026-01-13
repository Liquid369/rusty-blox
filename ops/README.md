# rusty-blox Monitoring Stack Setup

## Quick Start

### 1. Start Monitoring Stack

```bash
cd ops/
docker-compose up -d
```

This starts:
- **Prometheus** on port **9091** (http://localhost:9091)
- **Grafana** on port **3002** (http://localhost:3002)

Both run on different ports to avoid conflicts (port 3000: existing Grafana, port 3001: frontend).

### 2. Start rusty-blox

Make sure rusty-blox is running and exposing metrics:

```bash
cd ..
cargo run --release --bin rustyblox
```

Metrics are exposed on `http://localhost:9090/metrics`

### 3. Access Grafana

1. Open http://localhost:3002
2. Login with:
   - Username: `admin`
   - Password: `rustyblox_admin`
3. Navigate to **Dashboards** → **rusty-blox** folder
4. Open **"rusty-blox Production Monitoring"**

### 4. Verify Metrics

Check Prometheus is scraping metrics:
1. Open http://localhost:9091
2. Go to **Status** → **Targets**
3. Verify `rustyblox` target shows as **UP**

## Dashboard Overview

The main dashboard shows:

**Status Row:**
- Sync progress percentage
- Blocks behind network tip
- RPC connection status
- Current block height

**Performance Metrics:**
- Block processing rate (blocks/sec)
- Transaction processing rate (tx/sec)
- P50/P95/P99 latency for parsing, DB writes, RPC calls

**Health Monitoring:**
- Error rates by type (DB, RPC, parsing, invariants)
- Cache hit rates
- Reorg detection and depth

## Useful Commands

```bash
# View logs
docker-compose logs -f grafana
docker-compose logs -f prometheus

# Restart services
docker-compose restart

# Stop services
docker-compose down

# Stop and remove volumes (fresh start)
docker-compose down -v

# Check resource usage
docker stats rustyblox-grafana rustyblox-prometheus
```

## Configuration Files

- **ops/docker-compose.yml** - Container orchestration
- **ops/prometheus/prometheus.yml** - Prometheus scrape config
- **ops/prometheus/alerts.yml** - Alert rules
- **ops/grafana/provisioning/** - Auto-provisioning configs
- **ops/grafana/dashboards/** - Dashboard JSON files

## Customization

### Change Ports

Edit `ops/docker-compose.yml`:
```yaml
grafana:
  ports:
    - "3002:3000"  # Change 3001 to 3002 or any available port
```

### Change Admin Password

Edit `ops/docker-compose.yml`:
```yaml
environment:
  - GF_SECURITY_ADMIN_PASSWORD=your_new_password
```

Then restart:
```bash
docker-compose down
docker-compose up -d
```

### Add More Dashboards

1. Create dashboard in Grafana UI
2. Export as JSON
3. Save to `ops/grafana/dashboards/`
4. Refresh Grafana (it auto-loads within 10s)

## Troubleshooting

### Prometheus not scraping rustyblox

**Check rustyblox is running:**
```bash
curl http://localhost:9090/metrics
```

**Check Docker can reach host:**
- On Mac/Windows: `host.docker.internal` should work
- On Linux: May need to use host IP address

Edit `ops/prometheus/prometheus.yml`:
```yaml
- targets: ['192.168.1.100:9090']  # Use your host IP
```

### Grafana shows "No Data"

1. Check Prometheus target is UP: http://localhost:9091/targets
2. Verify metrics exist in Prometheus: http://localhost:9091/graph
   - Query: `rustyblox_indexed_height`
3. Check time range in Grafana (top right)

### Port Conflicts

If ports 3001 or 9091 are already in use:

```bash
# Find what's using the port
lsof -i :3001
lsof -i :9091

# Change ports in docker-compose.yml
```

## Metrics Catalog

See `METRICS_CATALOG.md` for full list of 45 available metrics.

Key metrics:
- `rustyblox_blocks_processed_total` - Block processing counter
- `rustyblox_indexed_height` - Current sync height
- `rustyblox_blocks_behind_tip` - How far behind network
- `rustyblox_rpc_connected` - RPC connection status (0/1)
- `rustyblox_invariant_violations_total` - Data integrity violations

## Alert Rules

Prometheus includes pre-configured alerts in `ops/prometheus/alerts.yml`:

- **BlockProcessingStalled** - No blocks processed for 5 minutes
- **InvariantViolation** - Data integrity violation detected
- **HighErrorRate** - >10 errors/minute
- **RPCConnectionLost** - RPC disconnected for 2+ minutes
- **ExcessiveReorgs** - >5 reorgs in 1 hour

Alerts show in Prometheus UI: http://localhost:9091/alerts

To receive alert notifications, configure Alertmanager (optional).

## Next Steps

1. ✅ Start monitoring stack
2. ✅ Verify Prometheus is scraping
3. ✅ Open Grafana dashboard
4. 📊 Monitor your sync progress!
5. 🔔 Configure Alertmanager for notifications (optional)

## Support

- Dashboard issues: Check browser console for errors
- Metrics missing: Verify rustyblox is exposing them
- Performance: Grafana/Prometheus use ~200MB RAM each
