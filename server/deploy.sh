#!/bin/bash

# Docker 이미지 배포 스크립트
# 사용법: ./deploy.sh [레지스트리] [태그]
# 예시: ./deploy.sh docker.io/username latest
# 예시: ./deploy.sh ghcr.io/username/perp-scanner-server v1.0.0

set -e

if [ $# -lt 1 ]; then
    echo "사용법: $0 <레지스트리> [태그]"
    echo ""
    echo "예시:"
    echo "  $0 docker.io/username"
    echo "  $0 ghcr.io/username/perp-scanner-server v1.0.0"
    echo "  $0 registry.example.com/perp-scanner-server latest"
    exit 1
fi

REGISTRY="$1"
TAG="${2:-latest}"
IMAGE_NAME="perp-scanner-server"
LOCAL_IMAGE="${IMAGE_NAME}:${TAG}"
REMOTE_IMAGE="${REGISTRY}/${IMAGE_NAME}:${TAG}"

echo "📦 이미지 태깅 중..."
echo "로컬: ${LOCAL_IMAGE}"
echo "원격: ${REMOTE_IMAGE}"

# 이미지가 없으면 빌드
if ! docker image inspect "${LOCAL_IMAGE}" >/dev/null 2>&1; then
    echo "이미지가 없습니다. 빌드를 시작합니다..."
    docker build -t "${LOCAL_IMAGE}" .
fi

# 태그 추가
docker tag "${LOCAL_IMAGE}" "${REMOTE_IMAGE}"

echo "🚀 이미지 푸시 중..."
docker push "${REMOTE_IMAGE}"

echo "✅ 배포 완료: ${REMOTE_IMAGE}"
echo ""
echo "다른 서버에서 실행:"
echo "  docker pull ${REMOTE_IMAGE}"
echo "  docker run -p 12090:12090 ${REMOTE_IMAGE} perp-scanner-server"

