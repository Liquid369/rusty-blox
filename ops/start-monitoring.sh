#!/bin/bash
# Quick start monitoring stack for rusty-blox

echo "🚀 Starting rusty-blox Monitoring Stack"
echo "========================================"
echo ""

# Check prerequisites
echo "Checking prerequisites..."

if ! command -v prometheus &> /dev/null; then
    echo "❌ Prometheus not installed"
    echo "   Install: brew install prometheus"
    exit 1
fi

if ! command -v grafana-server &> /dev/null; then
    echo "❌ Grafana not installed"  
    echo "   Install: brew install grafana"
    exit 1
fi

echo "✅ Prerequisites OK"
echo ""

# Start Prometheus in background
echo "📊 Starting Prometheus on http://localhost:9091..."
./start-prometheus.sh > prometheus.log 2>&1 &
PROM_PID=$!
echo "   PID: $PROM_PID"
sleep 2

# Check if Prometheus started
if ! ps -p $PROM_PID > /dev/null; then
    echo "❌ Prometheus failed to start. Check prometheus.log"
    exit 1
fi

# Start Grafana in background
echo "📈 Starting Grafana on http://localhost:3002..."
./start-grafana.sh > grafana.log 2>&1 &
GRAFANA_PID=$!
echo "   PID: $GRAFANA_PID"
sleep 3

# Check if Grafana started
if ! ps -p $GRAFANA_PID > /dev/null; then
    echo "❌ Grafana failed to start. Check grafana.log"
    kill $PROM_PID 2>/dev/null
    exit 1
fi

echo ""
echo "✅ Monitoring stack started!"
echo ""
echo "📍 Access points:"
echo "   • Prometheus: http://localhost:9091"
echo "   • Grafana:    http://localhost:3002 (admin / rustyblox_admin)"
echo ""
echo "📋 Process IDs:"
echo "   • Prometheus: $PROM_PID"
echo "   • Grafana:    $GRAFANA_PID"
echo ""
echo "🛑 To stop:"
echo "   kill $PROM_PID $GRAFANA_PID"
echo "   or run: ./stop-monitoring.sh"
echo ""
echo "📊 Next steps:"
echo "   1. Make sure rusty-blox is running (cargo run --release)"
echo "   2. Open Grafana: http://localhost:3002"
echo "   3. Add Prometheus data source: http://localhost:9091"
echo "   4. Import dashboard from: grafana/dashboards/rustyblox-main.json"
echo ""

# Save PIDs for stop script
echo "$PROM_PID" > .pids
echo "$GRAFANA_PID" >> .pids

echo "Logs:"
echo "   tail -f prometheus.log"
echo "   tail -f grafana.log"
