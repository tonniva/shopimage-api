#!/bin/bash

echo "🚀 Fast Redis Test..."

# Read token from test_redis_fixed.sh
TOKEN=$(grep 'UPSTASH_REDIS_REST_TOKEN=' test_redis_fixed.sh | cut -d'"' -f2)
URL=$(grep 'UPSTASH_REDIS_REST_URL=' test_redis_fixed.sh | cut -d'"' -f2)

# Set environment variables
export UPSTASH_REDIS_REST_URL="$URL"
export UPSTASH_REDIS_REST_TOKEN="$TOKEN"
export AZURE_STORAGE_CONNECTION_STRING="dummy"
export API_BASE_URL="http://localhost:8081"
export PORT=8081

echo "🔨 Quick build..."
cargo build --quiet

if [ $? -eq 0 ]; then
    echo "✅ Build OK!"
    echo "🚀 Starting server..."
    echo ""
    echo "📋 Expected output:"
    echo "✅ Redis connection test successful"
    echo "✅ Redis connection successful"
    echo ""
    
    # Start server
    cargo run
else
    echo "❌ Build failed!"
    exit 1
fi
