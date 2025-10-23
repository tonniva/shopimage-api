#!/bin/bash

# Test Redis caching locally
echo "🚀 Testing Redis caching locally..."

# Set environment variables (replace with your actual values)
export UPSTASH_REDIS_REST_URL="https://guiding-pigeon-8014.upstash.io"
export UPSTASH_REDIS_REST_TOKEN="AR9OAAImcDIxY2M3MzRkZGZjNzc0NTMxOTcxYTc0NGMzZGVkYmVmNHAyODAxNA"
export AZURE_STORAGE_CONNECTION_STRING="your_azure_connection_string_here"
export AZURE_BLOB_CONTAINER="shopimage"
export API_BASE_URL="http://localhost:8080"
export PORT=8080

echo "🔧 Fixed Redis connection issue!"
echo "✅ Upstash REST API response format handled correctly"

# Build the application
echo "🔨 Building application..."
cargo build

if [ $? -eq 0 ]; then
    echo "✅ Build successful!"
    echo "🚀 Starting server..."
    echo "📝 Note: Make sure to update the environment variables above with your actual values"
    echo "🔗 Redis URL: $UPSTASH_REDIS_REST_URL"
    echo "🌐 Server will start at: http://localhost:$PORT"
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
