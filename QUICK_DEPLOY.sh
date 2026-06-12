#!/bin/bash
# Quick deployment script for rusty-blox

set -e

echo "🚀 Rusty-Blox Quick Deploy"
echo "=========================="
echo ""

# Check for Docker
if ! command -v docker &> /dev/null; then
    echo "❌ Docker is not installed. Please install Docker first:"
    echo "   https://docs.docker.com/get-docker/"
    exit 1
fi

# Check for Docker Compose
if ! command -v docker-compose &> /dev/null; then
    echo "❌ Docker Compose is not installed. Please install Docker Compose first:"
    echo "   https://docs.docker.com/compose/install/"
    exit 1
fi

# Check if config.toml exists
if [ ! -f "config.toml" ]; then
    echo "⚠️  config.toml not found. Creating from example..."
    cp config.toml.example config.toml
    echo ""
    echo "📝 Please edit config.toml with your PIVX RPC credentials:"
    echo "   - RPC host (e.g., 192.168.1.100:51473)"
    echo "   - RPC username"
    echo "   - RPC password"
    echo ""
    read -p "Press Enter after editing config.toml to continue..."
fi

# Create data directory
echo "📁 Creating data directory..."
mkdir -p data

# Check if PIVX node is reachable
echo "🔍 Checking PIVX RPC connection..."
RPC_HOST=$(grep "host" config.toml | cut -d'"' -f2)
RPC_USER=$(grep "user" config.toml | cut -d'"' -f2)
RPC_PASS=$(grep "pass" config.toml | cut -d'"' -f2)

if command -v curl &> /dev/null; then
    if curl -s --user "$RPC_USER:$RPC_PASS" --data-binary '{"jsonrpc":"1.0","id":"test","method":"getblockcount","params":[]}' -H 'content-type: text/plain;' "http://$RPC_HOST" &> /dev/null; then
        echo "✅ PIVX RPC connection successful"
    else
        echo "⚠️  Warning: Could not connect to PIVX RPC. Please verify your configuration."
        read -p "Continue anyway? (y/N) " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            exit 1
        fi
    fi
fi

# Build and start services
echo ""
echo "🔨 Building Docker images..."
docker-compose build

echo ""
echo "🚀 Starting services..."
docker-compose up -d

echo ""
echo "⏳ Waiting for services to start..."
sleep 10

# Check health
echo ""
echo "🏥 Checking service health..."
if docker-compose ps | grep -q "Up"; then
    echo "✅ Services are running!"
else
    echo "❌ Some services failed to start. Check logs with: docker-compose logs"
    exit 1
fi

echo ""
echo "======================================"
echo "✨ Deployment Complete!"
echo "======================================"
echo ""
echo "Access your services at:"
echo "  📊 Frontend:   http://localhost:3002"
echo "  🔌 API:        http://localhost:3001/api/v2/"
echo "  📈 Grafana:    http://localhost:3000 (admin/admin)"
echo "  📉 Prometheus: http://localhost:9090"
echo ""
echo "Useful commands:"
echo "  View logs:     docker-compose logs -f"
echo "  Stop:          docker-compose stop"
echo "  Restart:       docker-compose restart"
echo "  Remove:        docker-compose down"
echo ""
echo "⏱️  Initial sync may take several hours to days."
echo "   Monitor progress at: http://localhost:3000"
echo ""
