#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

ARCH="${1:-x86_64}"
OUTPUT_DIR="$PROJECT_ROOT/build"
TEMP_DIR="$(mktemp -d)"

trap "rm -rf $TEMP_DIR" EXIT

echo -e "${GREEN}==== Muak initramfs build ====${NC}"
echo -e "${GREEN}Architecture: ${ARCH}${NC}"
echo -e "${GREEN}Output: ${OUTPUT_DIR}/initramfs.img${NC}"
echo

echo -e "${YELLOW}Building init binary...${NC}"
"$SCRIPT_DIR/build-init.sh" "$ARCH"

echo -e "${YELLOW}Building granola init system...${NC}"
cd "$PROJECT_ROOT/granola"
RUSTFLAGS='-C target-feature=+crt-static' cargo build --release --quiet
cd - > /dev/null

echo
echo -e "${YELLOW}Creating initramfs structure...${NC}"

mkdir -p "$TEMP_DIR/rootfs_source/sbin"
mkdir -p "$TEMP_DIR/rootfs_source/dev"
mkdir -p "$TEMP_DIR/rootfs_source/proc"
mkdir -p "$TEMP_DIR/rootfs_source/sys"
mkdir -p "$TEMP_DIR/rootfs_source/run"

echo -e "${YELLOW}Installing granola as /sbin/init...${NC}"
cp "$PROJECT_ROOT/granola/target/release/granola" "$TEMP_DIR/rootfs_source/sbin/init"
chmod +x "$TEMP_DIR/rootfs_source/sbin/init"

if ! command -v mksquashfs &> /dev/null; then
    echo -e "${RED}ERROR: mksquashfs not found${NC}"
    echo -e "${RED}Install squashfs-tools package${NC}"
    exit 1
fi

echo -e "${YELLOW}Creating squashfs root filesystem...${NC}"
mksquashfs "$TEMP_DIR/rootfs_source" "$TEMP_DIR/rootfs.sqsh" -comp zstd -noappend -quiet

mkdir -p "$TEMP_DIR/initramfs"
cp "$OUTPUT_DIR/init" "$TEMP_DIR/initramfs/init"
chmod +x "$TEMP_DIR/initramfs/init"
cp "$TEMP_DIR/rootfs.sqsh" "$TEMP_DIR/initramfs/rootfs.sqsh"

echo -e "${YELLOW}Packaging initramfs with cpio and zstd...${NC}"

cd "$TEMP_DIR/initramfs"
find . -print0 | cpio -o -H newc --null --quiet | zstd -19 -T0 > "$OUTPUT_DIR/initramfs.img"

echo
echo -e "${GREEN}==== initramfs Build Complete ====${NC}"
echo -e "${GREEN}initramfs: ${OUTPUT_DIR}/initramfs.img${NC}"
