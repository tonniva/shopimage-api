#!/bin/bash

# ============ CONFIG ============
IMAGE_NAME="tonniva/shopimage"         # เปลี่ยนเป็นชื่อ Docker Hub ของคุณ
SERVICE_NAME="shopimage"              # ชื่อ Cloud Run service
REGION="asia-southeast1"              # โซน Cloud Run (สิงคโปร์)
# =================================

if [ -z "$1" ]; then
  echo "❗ ต้องระบุ version เช่น ./deploy.sh v0.4.0"
  exit 1
fi

VERSION=$1
FULL_IMAGE="$IMAGE_NAME:$VERSION"

echo "🚀 เริ่ม Deploy Version $VERSION"
echo "📦 Docker Image: $FULL_IMAGE"

echo "🔨 Step 1: Build image..."
docker buildx build --platform linux/amd64 -t $FULL_IMAGE . || exit 1

echo "⬆️ Step 2: Push ไป Docker Hub..."
docker push $FULL_IMAGE || exit 1

echo "☁️ Step 3: Deploy ไป Cloud Run..."
gcloud run services update $SERVICE_NAME \
  --image $FULL_IMAGE \
  --region $REGION \
  --platform managed || exit 1

echo "✅ 🎉 Deploy สำเร็จ!"