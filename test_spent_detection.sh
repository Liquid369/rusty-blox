#!/bin/bash
# Quick test to verify spent UTXO detection with debug output

echo "🧪 Testing spent UTXO detection..."
echo "This will rebuild address index and show debug output"
echo ""

# Run rebuild-address-index and capture first 200 lines of output
./target/release/rebuild-address-index 2>&1 | head -200
