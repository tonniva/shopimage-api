#!/bin/bash

# ============ CONFIG ============
DOCKERHUB_USERNAME="tonniva"           # username Docker Hub ของคุณ
IMAGE_NAME="shopimage"                 # ชื่อ image
VERSION=${1:-"latest"}                 # version (default: latest)
# =================================

if [ -z "$1" ]; then
  echo "❗ ต้องระบุ version เช่น ./deploy-dockerhub.sh v0.4.0"
  echo "   หรือใช้ ./deploy-dockerhub.sh latest"
  exit 1
fi

FULL_IMAGE="$DOCKERHUB_USERNAME/$IMAGE_NAME:$VERSION"

echo "🚀 เริ่ม Deploy ไป Docker Hub"
echo "📦 Docker Image: $FULL_IMAGE"

echo "🔨 Step 1: Build และ Push ไป Docker Hub..."
docker buildx build --platform linux/amd64 -t $FULL_IMAGE --push . || exit 1

echo "✅ 🎉 Deploy ไป Docker Hub สำเร็จ!"
echo "🔗 Image URL: https://hub.docker.com/r/$DOCKERHUB_USERNAME/$IMAGE_NAME"
echo "📝 Pull command: docker pull $FULL_IMAGE"
