#!/bin/bash
# Start Grafana for rusty-blox monitoring on port 3002

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OPS_DIR="$SCRIPT_DIR"
DATA_DIR="$OPS_DIR/grafana/data"
LOGS_DIR="$OPS_DIR/grafana/logs"
PROVISIONING_DIR="$OPS_DIR/grafana/provisioning"

# Create directories
mkdir -p "$DATA_DIR"
mkdir -p "$LOGS_DIR"

echo "📊 Starting Grafana on port 3002..."
echo "   Provisioning: $PROVISIONING_DIR"
echo "   Data: $DATA_DIR"
echo "   Logs: $LOGS_DIR"
echo ""
echo "   Login at: http://localhost:3002"
echo "   Username: admin"
echo "   Password: admin (change on first login)"
echo ""

# Check if grafana is installed
if ! command -v grafana-server &> /dev/null; then
    echo "❌ Grafana not found. Install with: brew install grafana"
    exit 1
fi

# Find Grafana home directory
GRAFANA_HOME="/usr/local/share/grafana"
if [ ! -d "$GRAFANA_HOME" ]; then
    # Try Homebrew on Apple Silicon
    GRAFANA_HOME="/opt/homebrew/share/grafana"
fi

if [ ! -d "$GRAFANA_HOME" ]; then
    echo "⚠️  Could not find Grafana home directory"
    echo "   Trying default location..."
    GRAFANA_HOME="/usr/local/share/grafana"
fi

# Create minimal config if it doesn't exist
GRAFANA_CONFIG="$OPS_DIR/grafana/grafana.ini"
if [ ! -f "$GRAFANA_CONFIG" ]; then
    mkdir -p "$OPS_DIR/grafana"
    touch "$GRAFANA_CONFIG"
fi

grafana-server \
  --homepath="$GRAFANA_HOME" \
  --config="$GRAFANA_CONFIG" \
  cfg:default.paths.logs="$LOGS_DIR" \
  cfg:default.paths.data="$DATA_DIR" \
  cfg:default.paths.provisioning="$PROVISIONING_DIR" \
  cfg:default.server.http_port=3002 \
  cfg:default.security.admin_user=admin \
  cfg:default.security.admin_password=rustyblox_admin \
  cfg:default.analytics.reporting_enabled=false \
  cfg:default.analytics.check_for_updates=false

# If grafana not found:
# brew install grafana
