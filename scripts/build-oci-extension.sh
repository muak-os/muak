#!/bin/bash
# Build OCI image for Firecracker extension
set -euo pipefail

EXTENSION_NAME="firecracker"
EXTENSION_DIR="extensions/${EXTENSION_NAME}"
IMAGE_NAME="muak-${EXTENSION_NAME}"
IMAGE_TAG="latest"

if [ ! -f "${EXTENSION_DIR}/Dockerfile" ]; then
    echo "Error: Dockerfile not found in ${EXTENSION_DIR}"
    exit 1
fi

echo "Building OCI image: ${IMAGE_NAME}:${IMAGE_TAG}"
cd "$EXTENSION_DIR"

docker build -t "${IMAGE_NAME}:${IMAGE_TAG}" .

echo ""
echo "✓ Built: ${IMAGE_NAME}:${IMAGE_TAG}"
echo ""
echo "To push to a registry:"
echo "  docker tag ${IMAGE_NAME}:${IMAGE_TAG} ghcr.io/USERNAME/${IMAGE_NAME}:${IMAGE_TAG}"
echo "  docker push ghcr.io/USERNAME/${IMAGE_NAME}:${IMAGE_TAG}"
echo ""
echo "To export as tar:"
echo "  docker save ${IMAGE_NAME}:${IMAGE_TAG} -o ${EXTENSION_NAME}.tar"
echo ""
echo "To convert to squashfs:"
echo "  ./scripts/oci-to-sqsh.sh ${IMAGE_NAME}:${IMAGE_TAG} ${EXTENSION_NAME}"
