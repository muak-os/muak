#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

ARCH="${1:-x86_64}"
STUB_DIR="$PROJECT_ROOT/internal/stub"
OUTPUT_DIR="$PROJECT_ROOT/build"

echo -e "${GREEN}==== Building Muak Stub ====${NC}"
echo -e "${GREEN}Architecture: ${ARCH}${NC}"
echo

# Determine EFI target triple
case "$ARCH" in
    x86_64)
        TARGET="x86_64-unknown-uefi"
        OUTPUT_NAME="stub-x86_64.efi"
        ;;
    aarch64)
        TARGET="aarch64-unknown-uefi"
        OUTPUT_NAME="stub-aarch64.efi"
        ;;
    *)
        echo -e "${RED}ERROR: Unsupported architecture: ${ARCH}${NC}"
        echo -e "${RED}Supported: x86_64, aarch64${NC}"
        exit 1
        ;;
esac

if command -v rustup &> /dev/null; then
    echo -e "${YELLOW}Ensuring ${TARGET} target is installed...${NC}"
    rustup target add "$TARGET"
else
    echo -e "${RED}Error: rustup is required to build the stub${NC}"
    exit 1
fi

echo -e "${YELLOW}Building stub for ${TARGET}...${NC}"
cd "$STUB_DIR"

cargo +nightly build --release --target "$TARGET" --features uefi

mkdir -p "$OUTPUT_DIR"

BUILT_EFI="${PROJECT_ROOT}/target/${TARGET}/release/stub.efi"
OUTPUT_PATH="$OUTPUT_DIR/$OUTPUT_NAME"

if [ ! -f "$BUILT_EFI" ]; then
    echo -e "${RED}ERROR: Built stub not found at ${BUILT_EFI}${NC}"
    exit 1
fi

cp "$BUILT_EFI" "$OUTPUT_PATH"

echo
echo -e "${GREEN}==== Stub Build Complete ====${NC}"
echo -e "${GREEN}Output: ${OUTPUT_PATH}${NC}"
ls -lh "$OUTPUT_PATH"
