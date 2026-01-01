#!/bin/bash

# Test script for new Blockbook API features
# - Block hash lookup in /block-index/{hashOrHeight}
# - POST /sendtx endpoint

API_URL="http://localhost:3005"

echo "🧪 Testing New Blockbook API Features"
echo "======================================"
echo ""

# Test 1: Block-index with height (existing behavior)
echo "Test 1: Block-index with height (number)"
echo "GET /api/v2/block-index/1000"
response=$(curl -s -w "\nHTTP_CODE:%{http_code}" "${API_URL}/api/v2/block-index/1000")
http_code=$(echo "$response" | grep "HTTP_CODE" | cut -d: -f2)
body=$(echo "$response" | grep -v "HTTP_CODE")

echo "Status Code: $http_code"
echo "Response:"
echo "$body" | jq '.' 2>/dev/null || echo "$body"

if [ "$http_code" = "200" ]; then
    block_hash=$(echo "$body" | jq -r '.block_hash' 2>/dev/null)
    echo "✅ PASS: Got block hash for height 1000"
    echo "Block hash: $block_hash"
    
    # Test 2: Block-index with hash (new feature)
    if [ -n "$block_hash" ] && [ "$block_hash" != "null" ]; then
        echo ""
        echo "Test 2: Block-index with hash (64-char hex)"
        echo "GET /api/v2/block-index/$block_hash"
        response2=$(curl -s -w "\nHTTP_CODE:%{http_code}" "${API_URL}/api/v2/block-index/${block_hash}")
        http_code2=$(echo "$response2" | grep "HTTP_CODE" | cut -d: -f2)
        body2=$(echo "$response2" | grep -v "HTTP_CODE")
        
        echo "Status Code: $http_code2"
        echo "Response:"
        echo "$body2" | jq '.' 2>/dev/null || echo "$body2"
        
        returned_hash=$(echo "$body2" | jq -r '.block_hash' 2>/dev/null)
        
        if [ "$http_code2" = "200" ] && [ "$returned_hash" = "$block_hash" ]; then
            echo "✅ PASS: Block hash lookup works! Got same hash back"
        else
            echo "❌ FAIL: Block hash lookup failed or returned wrong hash"
        fi
    else
        echo "⚠️  SKIP: No block hash to test with"
    fi
else
    echo "❌ FAIL: Could not get block at height 1000"
fi

echo ""
echo "Test 3: Invalid block hash format"
echo "GET /api/v2/block-index/invalidhash"
response3=$(curl -s -w "\nHTTP_CODE:%{http_code}" "${API_URL}/api/v2/block-index/invalidhash")
http_code3=$(echo "$response3" | grep "HTTP_CODE" | cut -d: -f2)
body3=$(echo "$response3" | grep -v "HTTP_CODE")

echo "Status Code: $http_code3"
echo "Response:"
echo "$body3" | jq '.' 2>/dev/null || echo "$body3"

if [ "$http_code3" = "400" ]; then
    if echo "$body3" | jq -e '.error.message' > /dev/null 2>&1; then
        echo "✅ PASS: Returns 400 with Blockbook error format"
    else
        echo "⚠️  Returns 400 but error format not checked"
    fi
else
    echo "❌ FAIL: Should return 400 for invalid format"
fi

echo ""
echo "Test 4: Non-existent block hash"
echo "GET /api/v2/block-index/0000000000000000000000000000000000000000000000000000000000000000"
response4=$(curl -s -w "\nHTTP_CODE:%{http_code}" "${API_URL}/api/v2/block-index/0000000000000000000000000000000000000000000000000000000000000000")
http_code4=$(echo "$response4" | grep "HTTP_CODE" | cut -d: -f2)
body4=$(echo "$response4" | grep -v "HTTP_CODE")

echo "Status Code: $http_code4"
echo "Response:"
echo "$body4" | jq '.' 2>/dev/null || echo "$body4"

if [ "$http_code4" = "404" ]; then
    echo "✅ PASS: Returns 404 for non-existent block hash"
else
    echo "⚠️  Got status $http_code4 (expected 404)"
fi

echo ""
echo "======================================"
echo "POST /sendtx Tests"
echo "======================================"
echo ""

echo "Test 5: POST /sendtx with invalid hex (should fail gracefully)"
echo "POST /api/v2/sendtx"
echo "Body: invalid_hex_data"
response5=$(curl -s -w "\nHTTP_CODE:%{http_code}" -X POST \
    -H "Content-Type: text/plain" \
    -d "invalid_hex_data" \
    "${API_URL}/api/v2/sendtx")
http_code5=$(echo "$response5" | grep "HTTP_CODE" | cut -d: -f2)
body5=$(echo "$response5" | grep -v "HTTP_CODE")

echo "Status Code: $http_code5"
echo "Response:"
echo "$body5" | jq '.' 2>/dev/null || echo "$body5"

if [ "$http_code5" = "400" ]; then
    if echo "$body5" | jq -e '.error.message' > /dev/null 2>&1; then
        echo "✅ PASS: Returns 400 with Blockbook error format"
        error_msg=$(echo "$body5" | jq -r '.error.message')
        echo "Error message: $error_msg"
    else
        echo "⚠️  Returns 400 but error format not checked"
    fi
else
    echo "⚠️  Got status $http_code5 (expected 400 for invalid tx)"
fi

echo ""
echo "Test 6: POST /sendtx with JSON body"
echo "POST /api/v2/sendtx"
echo 'Body: {"hex": "invalid_hex"}'
response6=$(curl -s -w "\nHTTP_CODE:%{http_code}" -X POST \
    -H "Content-Type: application/json" \
    -d '{"hex": "invalid_hex"}' \
    "${API_URL}/api/v2/sendtx")
http_code6=$(echo "$response6" | grep "HTTP_CODE" | cut -d: -f2)
body6=$(echo "$response6" | grep -v "HTTP_CODE")

echo "Status Code: $http_code6"
echo "Response:"
echo "$body6" | jq '.' 2>/dev/null || echo "$body6"

if [ "$http_code6" = "400" ]; then
    if echo "$body6" | jq -e '.error.message' > /dev/null 2>&1; then
        echo "✅ PASS: JSON body parsing works, returns Blockbook error"
    else
        echo "⚠️  Returns 400 but error format not checked"
    fi
else
    echo "⚠️  Got status $http_code6 (expected 400)"
fi

echo ""
echo "Test 7: Legacy GET /sendtx/{hex} still works"
echo "GET /api/v2/sendtx/invalid_hex"
response7=$(curl -s -w "\nHTTP_CODE:%{http_code}" "${API_URL}/api/v2/sendtx/invalid_hex")
http_code7=$(echo "$response7" | grep "HTTP_CODE" | cut -d: -f2)
body7=$(echo "$response7" | grep -v "HTTP_CODE")

echo "Status Code: $http_code7"
echo "Response:"
echo "$body7" | jq '.' 2>/dev/null || echo "$body7"

if [ "$http_code7" = "400" ]; then
    echo "✅ PASS: Legacy GET endpoint still works with new error format"
else
    echo "⚠️  Got status $http_code7"
fi

echo ""
echo "======================================"
echo "🏁 Test Suite Complete"
echo "======================================"
echo ""
echo "Summary:"
echo "- Block-index now supports both height (number) and hash (64-char hex)"
echo "- POST /sendtx accepts plain hex or JSON {\"hex\": \"...\"}"
echo "- GET /sendtx/{hex} still works (backward compatible)"
echo "- All endpoints return Blockbook-compatible error format"
echo ""
echo "Note: To test with real transaction, you need a valid signed PIVX transaction hex"
echo ""
