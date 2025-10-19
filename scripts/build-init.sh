#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
INIT_DIR="$PROJECT_ROOT/init"
OUTPUT_DIR="$PROJECT_ROOT/build"

ARCH="${1:-x86_64}"

echo "Building muak-init for $ARCH..."

cd "$INIT_DIR"

RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --target x86_64-unknown-linux-gnu

mkdir -p "$OUTPUT_DIR"

cp "target/x86_64-unknown-linux-gnu/release/muak-init" "$OUTPUT_DIR/init"

echo "Init binary built: $OUTPUT_DIR/init"
ls -lh "$OUTPUT_DIR/init"
