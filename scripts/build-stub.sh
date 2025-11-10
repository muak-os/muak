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
        OUTPUT_NAME="muak-stub-x86_64.efi"
        ;;
    aarch64)
        TARGET="aarch64-unknown-uefi"
        OUTPUT_NAME="muak-stub-aarch64.efi"
        ;;
    *)
        echo -e "${RED}ERROR: Unsupported architecture: ${ARCH}${NC}"
        echo -e "${RED}Supported: x86_64, aarch64${NC}"
        exit 1
        ;;
esac

# Check if cargo is installed
if ! command -v cargo &> /dev/null; then
    echo -e "${RED}ERROR: cargo not found${NC}"
    echo -e "${RED}Please install Rust: https://rustup.rs/${NC}"
    exit 1
fi

# Add the target if rustup is available
if command -v rustup &> /dev/null; then
    echo -e "${YELLOW}Ensuring ${TARGET} target is installed...${NC}"
    rustup target add "$TARGET"
    rustup component add rust-src --toolchain nightly-x86_64-unknown-linux-gnu
else
    echo -e "${YELLOW}Note: rustup not found, assuming ${TARGET} target is available${NC}"
fi

# Build the stub
echo -e "${YELLOW}Building stub for ${TARGET}...${NC}"
cd "$STUB_DIR"

# Check if we have rustup or system cargo
if command -v rustup &> /dev/null; then
    # Use rustup with nightly toolchain
    cargo +nightly build \
        --release \
        --target "$TARGET" \
        -Z build-std=core,alloc \
        -Z build-std-features=compiler-builtins-mem
else
    # Try with system cargo
    RUSTFLAGS="-C target-feature=+crt-static" cargo build \
        --release \
        --target "$TARGET" \
        -Z build-std=core,alloc \
        -Z build-std-features=compiler-builtins-mem
fi

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Copy the built EFI to config directory
BUILT_EFI="target/${TARGET}/release/muak-stub.efi"
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
