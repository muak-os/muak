#!/bin/bash
# Convert OCI image to SquashFS for MUAK
set -euo pipefail

if [ $# -lt 2 ]; then
    echo "Usage: $0 <oci-image> <extension-name>"
    echo "Example: $0 muak-firecracker:latest firecracker"
    echo "         $0 ghcr.io/user/muak-firecracker:v1.0 firecracker"
    exit 1
fi

OCI_IMAGE="$1"
EXTENSION_NAME="$2"
OUTPUT_DIR="build/extensions"

mkdir -p "$OUTPUT_DIR"
WORK_DIR=$(mktemp -d)

echo "Converting OCI image to SquashFS..."
echo "  Image: $OCI_IMAGE"
echo "  Output: ${OUTPUT_DIR}/${EXTENSION_NAME}.sqsh"

if command -v crane &> /dev/null; then
    echo "  Using: crane (recommended)"
    crane export "$OCI_IMAGE" - | tar -xC "$WORK_DIR"

elif command -v podman &> /dev/null; then
    echo "  Using: podman"
    if [[ "$OCI_IMAGE" == *"/"* ]] && [[ "$OCI_IMAGE" != "localhost/"* ]]; then
        podman pull "$OCI_IMAGE" >/dev/null
    fi
    CONTAINER_ID=$(podman create "$OCI_IMAGE")
    podman export "$CONTAINER_ID" | tar -xC "$WORK_DIR"
    podman rm "$CONTAINER_ID" >/dev/null

else
    echo "Error: No supported OCI tool found!"
    echo "Please install one of: crane, docker, podman, or skopeo+umoci"
    rm -rf "$WORK_DIR"
    exit 1
fi

# Create squashfs
echo "  Creating SquashFS..."
mksquashfs "$WORK_DIR" "${OUTPUT_DIR}/${EXTENSION_NAME}.sqsh" \
    -comp xz \
    -Xbcj x86 \
    -b 1M \
    -noappend \
    -no-progress

# Cleanup
rm -rf "$WORK_DIR"

echo "✓ Created: ${OUTPUT_DIR}/${EXTENSION_NAME}.sqsh"
ls -lh "${OUTPUT_DIR}/${EXTENSION_NAME}.sqsh"
