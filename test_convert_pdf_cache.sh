#!/bin/bash

# Test script for /api/convert-pdf cache functionality
# This script tests cache hit/miss scenarios for PDF conversion

echo "🚀 Testing /api/convert-pdf Cache Functionality"
echo "=============================================="

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Server URL
SERVER_URL="http://localhost:8080"

# Check if server is running
echo -e "${BLUE}🔍 Checking if server is running...${NC}"
if ! curl -s "$SERVER_URL/healthz" > /dev/null; then
    echo -e "${RED}❌ Server is not running! Please start the server first:${NC}"
    echo "   cargo run"
    exit 1
fi
echo -e "${GREEN}✅ Server is running${NC}"

# Check if test PDF exists
TEST_PDF="test.pdf"
if [ ! -f "$TEST_PDF" ]; then
    echo -e "${YELLOW}⚠️  Test PDF file '$TEST_PDF' not found${NC}"
    echo -e "${YELLOW}   Please create a test PDF file or use an existing one${NC}"
    echo -e "${YELLOW}   You can create one with:${NC}"
    echo -e "${YELLOW}   echo 'Test PDF content' | ps2pdf - test.pdf${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Test PDF file found: $TEST_PDF${NC}"
echo ""

# Test 1: First request (should be cache MISS)
echo -e "${BLUE}📄 Test 1: First request (Cache MISS)${NC}"
echo "   This should process the PDF and cache the result"
echo ""

response1=$(curl -s -X POST "$SERVER_URL/api/convert-pdf" \
    -F "file=@$TEST_PDF" \
    -w "\n%{http_code}")

http_code1=$(echo "$response1" | tail -n1)
response_body1=$(echo "$response1" | head -n -1)

echo "   HTTP Status: $http_code1"
if [ "$http_code1" = "200" ]; then
    echo -e "   ${GREEN}✅ Request successful${NC}"
    echo "   Response: $response_body1" | head -c 200
    echo "..."
else
    echo -e "   ${RED}❌ Request failed${NC}"
    echo "   Response: $response_body1"
fi

echo ""
echo -e "${YELLOW}⏳ Waiting 2 seconds before next test...${NC}"
sleep 2

# Test 2: Second request (should be cache HIT)
echo -e "${BLUE}📄 Test 2: Second request (Cache HIT)${NC}"
echo "   This should use the cached result (much faster)"
echo ""

response2=$(curl -s -X POST "$SERVER_URL/api/convert-pdf" \
    -F "file=@$TEST_PDF" \
    -w "\n%{http_code}")

http_code2=$(echo "$response2" | tail -n1)
response_body2=$(echo "$response2" | head -n -1)

echo "   HTTP Status: $http_code2"
if [ "$http_code2" = "200" ]; then
    echo -e "   ${GREEN}✅ Request successful${NC}"
    echo "   Response: $response_body2" | head -c 200
    echo "..."
else
    echo -e "   ${RED}❌ Request failed${NC}"
    echo "   Response: $response_body2"
fi

echo ""

# Test 3: Different PDF (should be cache MISS)
echo -e "${BLUE}📄 Test 3: Different PDF (Cache MISS)${NC}"
echo "   This should process a different PDF (if available)"
echo ""

# Try to find a different PDF file
DIFFERENT_PDF=""
for pdf in "test2.pdf" "sample.pdf" "document.pdf"; do
    if [ -f "$pdf" ]; then
        DIFFERENT_PDF="$pdf"
        break
    fi
done

if [ -n "$DIFFERENT_PDF" ]; then
    echo "   Using different PDF: $DIFFERENT_PDF"
    
    response3=$(curl -s -X POST "$SERVER_URL/api/convert-pdf" \
        -F "file=@$DIFFERENT_PDF" \
        -w "\n%{http_code}")
    
    http_code3=$(echo "$response3" | tail -n1)
    response_body3=$(echo "$response3" | head -n -1)
    
    echo "   HTTP Status: $http_code3"
    if [ "$http_code3" = "200" ]; then
        echo -e "   ${GREEN}✅ Request successful${NC}"
        echo "   Response: $response_body3" | head -c 200
        echo "..."
    else
        echo -e "   ${RED}❌ Request failed${NC}"
        echo "   Response: $response_body3"
    fi
else
    echo -e "   ${YELLOW}⚠️  No different PDF file found, skipping this test${NC}"
    echo "   To test with different PDF, create another PDF file"
fi

echo ""
echo -e "${BLUE}📊 Cache Test Summary${NC}"
echo "=================="
echo "Test 1 (First request): $([ "$http_code1" = "200" ] && echo -e "${GREEN}✅ PASS${NC}" || echo -e "${RED}❌ FAIL${NC}")"
echo "Test 2 (Second request): $([ "$http_code2" = "200" ] && echo -e "${GREEN}✅ PASS${NC}" || echo -e "${RED}❌ FAIL${NC}")"
if [ -n "$DIFFERENT_PDF" ]; then
    echo "Test 3 (Different PDF): $([ "$http_code3" = "200" ] && echo -e "${GREEN}✅ PASS${NC}" || echo -e "${RED}❌ FAIL${NC}")"
fi

echo ""
echo -e "${BLUE}💡 Expected Console Logs:${NC}"
echo "   Test 1: '💭 PDF cache MISS! Processing PDF...'"
echo "   Test 2: '🎯 PDF cache HIT! Using cached result'"
echo "   Test 3: '💭 PDF cache MISS! Processing PDF...' (if different PDF used)"

echo ""
echo -e "${GREEN}🎉 Cache testing completed!${NC}"
echo "   Check the server console for cache hit/miss logs"
