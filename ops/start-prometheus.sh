#!/bin/bash
# Start Prometheus for rusty-blox monitoring

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OPS_DIR="$SCRIPT_DIR"
DATA_DIR="$OPS_DIR/prometheus/data"

# Create data directory
mkdir -p "$DATA_DIR"

echo "🔥 Starting Prometheus on port 9091..."
echo "   Config: $OPS_DIR/prometheus/prometheus.yml"
echo "   Data: $DATA_DIR"
echo ""

prometheus \
  --config.file="$OPS_DIR/prometheus/prometheus.yml" \
  --storage.tsdb.path="$DATA_DIR" \
  --storage.tsdb.retention.time=30d \
  --web.listen-address=:9091 \
  --web.enable-lifecycle

# If prometheus not found:
# brew install prometheus
