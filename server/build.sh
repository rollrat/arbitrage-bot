#!/bin/bash

# Docker 이미지 빌드 스크립트
# 사용법: ./build.sh [태그]

set -e

IMAGE_NAME="perp-scanner-server"
TAG="${1:-latest}"
FULL_IMAGE_NAME="${IMAGE_NAME}:${TAG}"

echo "🔨 Docker 이미지 빌드 중..."
echo "이미지 이름: ${FULL_IMAGE_NAME}"

docker build -t "${FULL_IMAGE_NAME}" .

echo "✅ 빌드 완료: ${FULL_IMAGE_NAME}"
echo ""
echo "실행 방법:"
echo "  docker run -p 12090:12090 ${FULL_IMAGE_NAME} perp-scanner-server"
echo "  docker run -v /path/to/data:/app/data ${FULL_IMAGE_NAME} analysis-chat --files /app/data/file.json"

