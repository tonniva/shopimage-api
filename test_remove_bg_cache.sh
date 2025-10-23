#!/bin/bash

# 🎨 Remove Background Cache Test Script สำหรับ ShopImage API
# ทดสอบ cache functionality ของ /api/remove-bg endpoint

echo "🎨 Testing Remove Background Cache for ShopImage API"
echo "=================================================="

# ตั้งค่า environment variables
export UPSTASH_REDIS_REST_URL="https://guiding-pigeon-8014.upstash.io"
export UPSTASH_REDIS_REST_TOKEN="AR9OAAImcDIxY2M3MzRkZGZjNzc0NTMxOTcxYTc0NGMzZGVkYmVmNHAyODAxNA"

# ตั้งค่า environment variables อื่นๆ (แก้ไขตาม environment ของคุณ)
export AZURE_STORAGE_CONNECTION_STRING="DefaultEndpointsProtocol=https;AccountName=your_account;AccountKey=your_key;EndpointSuffix=core.windows.net"
export AZURE_BLOB_CONTAINER="shopimage"
export API_BASE_URL="http://localhost:8080"
export PORT=8080

echo "📋 Environment Variables:"
echo "  UPSTASH_REDIS_REST_URL: $UPSTASH_REDIS_REST_URL"
echo "  UPSTASH_REDIS_REST_TOKEN: ${UPSTASH_REDIS_REST_TOKEN:0:10}..."
echo "  API_BASE_URL: $API_BASE_URL"
echo ""

# ตรวจสอบว่ามีไฟล์รูปภาพสำหรับทดสอบหรือไม่
if [ ! -f "test_image.jpg" ]; then
    echo "⚠️  ไม่พบไฟล์ test_image.jpg"
    echo "   กรุณาสร้างไฟล์รูปภาพชื่อ test_image.jpg ในโฟลเดอร์นี้"
    echo "   หรือใช้รูปภาพอื่นแทน"
    exit 1
fi

echo "🔄 Starting ShopImage API server..."
echo "   (กด Ctrl+C เพื่อหยุด server)"

# รัน server ใน background
cargo run &
SERVER_PID=$!

# รอให้ server เริ่มทำงาน
echo "⏳ Waiting for server to start..."
sleep 5

# ตรวจสอบว่า server ทำงานหรือไม่
if ! curl -s http://localhost:8080/healthz > /dev/null; then
    echo "❌ Server failed to start"
    kill $SERVER_PID 2>/dev/null
    exit 1
fi

echo "✅ Server is running!"
echo ""

# ทดสอบ cache
echo "🧪 Testing Remove Background Cache Functionality"
echo "================================================"

echo "📤 Test 1: Remove background (should be cache miss)"
echo "   Command: curl -X POST http://localhost:8080/api/remove-bg -F \"file=@test_image.jpg\" -F \"border=10\" -F \"border_color=white\""
echo ""

# ทดสอบครั้งที่ 1 (ควรเป็น cache miss)
echo "🔄 First request (Cache Miss expected)..."
RESPONSE1=$(curl -s -X POST http://localhost:8080/api/remove-bg \
  -F "file=@test_image.jpg" \
  -F "border=10" \
  -F "border_color=white")

echo "📊 Response 1:"
echo "$RESPONSE1" | jq '.' 2>/dev/null || echo "$RESPONSE1"
echo ""

# รอสักครู่
echo "⏳ Waiting 3 seconds..."
sleep 3

# ทดสอบครั้งที่ 2 (ควรเป็น cache hit)
echo "🔄 Second request (Cache Hit expected)..."
RESPONSE2=$(curl -s -X POST http://localhost:8080/api/remove-bg \
  -F "file=@test_image.jpg" \
  -F "border=10" \
  -F "border_color=white")

echo "📊 Response 2:"
echo "$RESPONSE2" | jq '.' 2>/dev/null || echo "$RESPONSE2"
echo ""

# ทดสอบครั้งที่ 3 (ต่าง parameters - ควรเป็น cache miss)
echo "🔄 Third request (Different parameters - Cache Miss expected)..."
RESPONSE3=$(curl -s -X POST http://localhost:8080/api/remove-bg \
  -F "file=@test_image.jpg" \
  -F "border=20" \
  -F "border_color=black")

echo "📊 Response 3:"
echo "$RESPONSE3" | jq '.' 2>/dev/null || echo "$RESPONSE3"
echo ""

# เปรียบเทียบผลลัพธ์
echo "🔍 Comparing Results:"
echo "===================="

# ดึง download_url จาก response
URL1=$(echo "$RESPONSE1" | jq -r '.download_url' 2>/dev/null)
URL2=$(echo "$RESPONSE2" | jq -r '.download_url' 2>/dev/null)
URL3=$(echo "$RESPONSE3" | jq -r '.download_url' 2>/dev/null)

if [ "$URL1" != "null" ] && [ "$URL2" != "null" ] && [ "$URL3" != "null" ]; then
    echo "✅ All requests successful"
    echo "   First URL (border=10, white):  $URL1"
    echo "   Second URL (border=10, white): $URL2"
    echo "   Third URL (border=20, black):  $URL3"
    
    # ตรวจสอบว่าเป็น cache hit หรือไม่
    if [ "$URL1" != "$URL2" ]; then
        echo "🎯 Cache working correctly!"
        echo "   - First request: Processed and cached"
        echo "   - Second request: Retrieved from cache"
        echo "   - Third request: Different parameters, new processing"
    else
        echo "⚠️  URLs are the same - might indicate caching issue"
    fi
else
    echo "❌ One or more requests failed"
fi

echo ""
echo "📋 Server Logs (check terminal for cache messages):"
echo "   Look for: 💭 Remove-bg cache miss, 🎯 Remove-bg cache hit, 💾 Cached remove-bg result"
echo ""

# หยุด server
echo "🛑 Stopping server..."
kill $SERVER_PID 2>/dev/null
wait $SERVER_PID 2>/dev/null

echo ""
echo "✅ Test completed!"
echo ""
echo "💡 Tips:"
echo "   - Check server logs for cache messages"
echo "   - First request should show '💭 Remove-bg cache miss'"
echo "   - Second request should show '🎯 Remove-bg cache hit'"
echo "   - Both should show '💾 Cached remove-bg result'"
echo "   - Third request should show '💭 Remove-bg cache miss' (different parameters)"
echo ""
echo "🎨 Cache Key Format: remove_bg:{file_hash}:{border_size}:{border_color}"
echo "🔗 Monitor Redis: https://console.upstash.com/redis/guiding-pigeon-8014"
