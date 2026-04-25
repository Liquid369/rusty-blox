# rusty-blox Monitoring Setup

## Setup Options

You have two options for running the monitoring stack:

### Option A: Docker (Recommended - Easier)
### Option B: Native Binaries (What we'll use now)

---

## Option B: Native Setup (macOS)

### Prerequisites

Install via Homebrew:
```bash
brew install prometheus grafana
```

### 1. Configure Prometheus

Prometheus config is already at `ops/prometheus/prometheus.yml`

### 2. Start Prometheus

```bash
# Start on port 9091 (avoiding default 9090 used by rustyblox)
prometheus \
  --config.file=/Users/liquid/Projects/rusty-blox/ops/prometheus/prometheus.yml \
  --storage.tsdb.path=/Users/liquid/Projects/rusty-blox/ops/prometheus/data \
  --storage.tsdb.retention.time=30d \
  --web.listen-address=:9091 \
  --web.enable-lifecycle &

# Or run in background with brew services:
# First, create a custom plist file
```

### 3. Start Grafana

```bash
# Start on port 3002 (3000: existing Grafana, 3001: frontend)
grafana-server \
  --config=/usr/local/etc/grafana/grafana.ini \
  --homepath=/usr/local/share/grafana \
  --packaging=brew \
  cfg:default.paths.logs=/Users/liquid/Projects/rusty-blox/ops/grafana/logs \
  cfg:default.paths.data=/Users/liquid/Projects/rusty-blox/ops/grafana/data \
  cfg:default.paths.plugins=/usr/local/var/lib/grafana/plugins \
  cfg:default.server.http_port=3002 &

# Or with brew services on different port:
# brew services start grafana
# Then change port in /usr/local/etc/grafana/grafana.ini
```

### 4. Configure Grafana

1. Open http://localhost:3002
2. Login: admin / rustyblox_admin
3. Add Prometheus data source:
   - URL: http://localhost:9091
   - Access: Server (default)
4. Import dashboard from `ops/grafana/dashboards/rustyblox-main.json`

---

## Quick Start Scripts

I'll create helper scripts for you...
