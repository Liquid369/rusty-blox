#!/bin/bash
# Stop monitoring stack

echo "🛑 Stopping rusty-blox Monitoring Stack"

if [ ! -f .pids ]; then
    echo "❌ No PID file found. Services may not be running."
    echo "   Try: pkill -f prometheus; pkill -f grafana-server"
    exit 1
fi

PROM_PID=$(sed -n '1p' .pids)
GRAFANA_PID=$(sed -n '2p' .pids)

echo "Stopping Prometheus (PID: $PROM_PID)..."
kill $PROM_PID 2>/dev/null && echo "✅ Prometheus stopped" || echo "⚠️  Prometheus not running"

echo "Stopping Grafana (PID: $GRAFANA_PID)..."
kill $GRAFANA_PID 2>/dev/null && echo "✅ Grafana stopped" || echo "⚠️  Grafana not running"

rm -f .pids
echo ""
echo "✅ Monitoring stack stopped"
