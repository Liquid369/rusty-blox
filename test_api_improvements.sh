#!/bin/bash

# Test script for Blockbook API compatibility improvements
# Tests error responses and transaction fields

API_URL="http://localhost:3005"

echo "🧪 Testing Blockbook API Compatibility Improvements"
echo "=================================================="
echo ""

# Test 1: Error response format - Invalid block height
echo "Test 1: Error response format (invalid block height)"
echo "GET /api/v2/block-index/invalid"
response=$(curl -s -w "\nHTTP_CODE:%{http_code}" "${API_URL}/api/v2/block-index/invalid")
http_code=$(echo "$response" | grep "HTTP_CODE" | cut -d: -f2)
body=$(echo "$response" | grep -v "HTTP_CODE")

echo "Status Code: $http_code"
echo "Response Body:"
echo "$body" | jq '.' 2>/dev/null || echo "$body"
echo ""

# Check if error format matches Blockbook spec
if echo "$body" | jq -e '.error.message' > /dev/null 2>&1; then
    echo "✅ PASS: Error response has correct format {error: {message: '...'}}"
else
    echo "❌ FAIL: Error response missing Blockbook format"
fi
echo ""

# Test 2: Error response - Non-existent block
echo "Test 2: Error response (non-existent block)"
echo "GET /api/v2/block-index/99999999"
response=$(curl -s -w "\nHTTP_CODE:%{http_code}" "${API_URL}/api/v2/block-index/99999999")
http_code=$(echo "$response" | grep "HTTP_CODE" | cut -d: -f2)
body=$(echo "$response" | grep -v "HTTP_CODE")

echo "Status Code: $http_code"
echo "Response Body:"
echo "$body" | jq '.' 2>/dev/null || echo "$body"
echo ""

if [ "$http_code" = "404" ]; then
    echo "✅ PASS: Returns 404 for non-existent block"
else
    echo "❌ FAIL: Expected 404, got $http_code"
fi
echo ""

# Test 3: Error response - Non-existent transaction
echo "Test 3: Error response (non-existent transaction)"
echo "GET /api/v2/tx/0000000000000000000000000000000000000000000000000000000000000000"
response=$(curl -s -w "\nHTTP_CODE:%{http_code}" "${API_URL}/api/v2/tx/0000000000000000000000000000000000000000000000000000000000000000")
http_code=$(echo "$response" | grep "HTTP_CODE" | cut -d: -f2)
body=$(echo "$response" | grep -v "HTTP_CODE")

echo "Status Code: $http_code"
echo "Response Body:"
echo "$body" | jq '.' 2>/dev/null || echo "$body"
echo ""

if [ "$http_code" = "404" ]; then
    echo "✅ PASS: Returns 404 for non-existent transaction"
    if echo "$body" | jq -e '.error.message' > /dev/null 2>&1; then
        echo "✅ PASS: Error has Blockbook format"
    fi
else
    echo "❌ FAIL: Expected 404, got $http_code"
fi
echo ""

# Test 4: Transaction structure - Check for new fields
echo "Test 4: Transaction structure (checking new fields)"
echo "GET /api/v2/status (to get current height)"
status_response=$(curl -s "${API_URL}/api/v2/status")
current_height=$(echo "$status_response" | jq -r '.height')

if [ "$current_height" != "null" ] && [ -n "$current_height" ]; then
    echo "Current chain height: $current_height"
    
    # Get a block at recent height
    test_height=$((current_height - 10))
    echo ""
    echo "Fetching block at height $test_height to find a transaction..."
    block_response=$(curl -s "${API_URL}/api/v2/block-detail/${test_height}")
    txid=$(echo "$block_response" | jq -r '.transactions[0].txid' 2>/dev/null)
    
    if [ "$txid" != "null" ] && [ -n "$txid" ]; then
        echo "Testing with transaction: $txid"
        echo ""
        
        tx_response=$(curl -s "${API_URL}/api/v2/tx/${txid}")
        echo "Transaction response (first 500 chars):"
        echo "$tx_response" | head -c 500
        echo ""
        echo ""
        
        # Check for required Blockbook fields
        echo "Checking for required fields:"
        
        has_version=$(echo "$tx_response" | jq 'has("version")')
        has_locktime=$(echo "$tx_response" | jq 'has("lockTime")')
        has_size=$(echo "$tx_response" | jq 'has("size")')
        has_vsize=$(echo "$tx_response" | jq 'has("vsize")')
        has_blockhash=$(echo "$tx_response" | jq 'has("blockHash")')
        has_blockheight=$(echo "$tx_response" | jq 'has("blockHeight")')
        
        echo "  version: $has_version $([ "$has_version" = "true" ] && echo "✅" || echo "❌")"
        echo "  lockTime: $has_locktime $([ "$has_locktime" = "true" ] && echo "✅" || echo "❌")"
        echo "  size: $has_size $([ "$has_size" = "true" ] && echo "✅" || echo "❌")"
        echo "  vsize: $has_vsize $([ "$has_vsize" = "true" ] && echo "✅" || echo "❌")"
        echo "  blockHash: $has_blockhash $([ "$has_blockhash" = "true" ] && echo "✅" || echo "❌")"
        echo "  blockHeight: $has_blockheight $([ "$has_blockheight" = "true" ] && echo "✅" || echo "❌")"
        
        # Check blockHeight is signed integer (can be -1 for mempool)
        block_height_value=$(echo "$tx_response" | jq -r '.blockHeight')
        echo ""
        echo "  blockHeight value: $block_height_value"
        echo "  blockHeight type: $(echo "$tx_response" | jq -r '.blockHeight | type')"
        
        all_fields_present=true
        [ "$has_version" != "true" ] && all_fields_present=false
        [ "$has_locktime" != "true" ] && all_fields_present=false
        [ "$has_size" != "true" ] && all_fields_present=false
        [ "$has_vsize" != "true" ] && all_fields_present=false
        [ "$has_blockhash" != "true" ] && all_fields_present=false
        
        echo ""
        if [ "$all_fields_present" = "true" ]; then
            echo "✅ PASS: All required Blockbook transaction fields present"
        else
            echo "❌ FAIL: Some required fields missing"
        fi
    else
        echo "⚠️  Could not find transaction in block"
    fi
else
    echo "⚠️  Could not determine current height"
fi

echo ""
echo "=================================================="
echo "🏁 Test suite complete"
echo ""
echo "Summary:"
echo "- Error responses now use Blockbook format: {error: {message: '...'}}"
echo "- Transaction includes: version, lockTime, size, vsize, blockHash"
echo "- blockHeight is now i32 (supports -1 for mempool txs)"
echo ""
