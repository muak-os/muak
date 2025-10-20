#!/bin/bash
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "Usage: $0 <extension-name>"
    echo "Example: $0 firecracker"
    exit 1
fi

EXTENSION_NAME="$1"
EXTENSION_DIR="extensions/${EXTENSION_NAME}"
OUTPUT_DIR="build/extensions"

if [ ! -d "$EXTENSION_DIR" ]; then
    echo "Error: Extension directory not found: $EXTENSION_DIR"
    exit 1
fi

if [ ! -f "${EXTENSION_DIR}/manifest.yaml" ]; then
    echo "Error: manifest.yaml not found in $EXTENSION_DIR"
    exit 1
fi

if [ ! -d "${EXTENSION_DIR}/rootfs" ]; then
    echo "Error: rootfs directory not found in $EXTENSION_DIR"
    exit 1
fi

mkdir -p "$OUTPUT_DIR"

echo "Building extension: $EXTENSION_NAME"

OUTPUT_FILE="${OUTPUT_DIR}/${EXTENSION_NAME}.sqsh"

mksquashfs "${EXTENSION_DIR}/rootfs" "$OUTPUT_FILE" \
    -comp xz \
    -Xbcj x86 \
    -b 1M \
    -noappend \
    -no-progress

echo "Extension built: $OUTPUT_FILE"
ls -lh "$OUTPUT_FILE"
