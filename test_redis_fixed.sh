#!/bin/bash

# Test Redis caching locally with proper token
echo "🚀 Testing Redis caching locally..."

# ⚠️  IMPORTANT: Replace with your actual Upstash Redis token
# Get your token from: https://console.upstash.com/redis/guiding-pigeon-8014
export UPSTASH_REDIS_REST_URL="https://guiding-pigeon-8014.upstash.io"
export UPSTASH_REDIS_REST_TOKEN="AR9OAAImcDIxY2M3MzRkZGZjNzc0NTMxOTcxYTc0NGMzZGVkYmVmNHAyODAxNA"

# Other environment variables
export AZURE_STORAGE_CONNECTION_STRING="DefaultEndpointsProtocol=https;AccountName=namahayan;AccountKey=3NL09/t2ycXq5vEQI/LpT8JaYE59x0XIvAiT5kG7hzgT6crV1TJZBU8cHU3iMKcowGJ0I5b4U7rG+ASt2obdFw==;EndpointSuffix=core.windows.net"
export AZURE_BLOB_CONTAINER="shopimage"
export API_BASE_URL="http://localhost:8080"
export PORT=8080

echo "🔧 Instructions:"
echo "1. Go to: https://console.upstash.com/redis/guiding-pigeon-8014"
echo "2. Copy your REST Token"
echo "3. Replace 'REPLACE_WITH_YOUR_ACTUAL_TOKEN_HERE' with your actual token"
echo "4. Run this script again"
echo ""

# Check if token is still placeholder
if [ "$UPSTASH_REDIS_REST_TOKEN" = "REPLACE_WITH_YOUR_ACTUAL_TOKEN_HERE" ]; then
    echo "❌ Please replace the token with your actual Upstash Redis token!"
    echo "🔗 Get your token from: https://console.upstash.com/redis/guiding-pigeon-8014"
    exit 1
fi

echo "✅ Using token: ${UPSTASH_REDIS_REST_TOKEN:0:10}..."
echo "🔗 Redis URL: $UPSTASH_REDIS_REST_URL"
echo ""

# Build the application
echo "🔨 Building application..."
cargo build

if [ $? -eq 0 ]; then
    echo "✅ Build successful!"
    echo "🚀 Starting server..."
    echo ""
    echo "📋 To test Redis caching:"
    echo "1. Upload the same image twice with same parameters"
    echo "2. First request should show '💭 Cache miss'"
    echo "3. Second request should show '🎯 Cache hit'"
    echo ""
    
    # Start the server
    cargo run
else
    echo "❌ Build failed!"
    exit 1
fi
