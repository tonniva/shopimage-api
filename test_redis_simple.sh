#!/bin/bash

echo "🚀 Testing Redis connection..."

# Read token from test_redis_fixed.sh
TOKEN=$(grep 'UPSTASH_REDIS_REST_TOKEN=' test_redis_fixed.sh | cut -d'"' -f2)
URL=$(grep 'UPSTASH_REDIS_REST_URL=' test_redis_fixed.sh | cut -d'"' -f2)

echo "🔗 Redis URL: $URL"
echo "🔑 Token: ${TOKEN:0:10}..."

# Set environment variables
export UPSTASH_REDIS_REST_URL="$URL"
export UPSTASH_REDIS_REST_TOKEN="$TOKEN"
export AZURE_STORAGE_CONNECTION_STRING="dummy"
export API_BASE_URL="http://localhost:8080"
export PORT=8080

echo "🔨 Building..."
cargo build --quiet

if [ $? -eq 0 ]; then
    echo "✅ Build successful!"
    echo "🚀 Starting server..."
    echo ""
    echo "📋 Watch for these messages:"
    echo "✅ Redis connection test successful"
    echo "✅ Redis connection successful"
    echo ""
    
    cargo run
else
    echo "❌ Build failed!"
    exit 1
fi
