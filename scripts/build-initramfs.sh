#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

ARCH="${1:-x86_64}"
EXTENSIONS="${2:-}"
OUTPUT_DIR="$PROJECT_ROOT/build"
TEMP_DIR="$(mktemp -d)"

trap "rm -rf $TEMP_DIR" EXIT

if [ -n "$EXTENSIONS" ]; then
    EXT_LIST=$(echo "$EXTENSIONS" | tr ',' ' ')
    SCHEMATIC_ID=$("$SCRIPT_DIR/hash-schematic.sh" $EXT_LIST)
else
    EXT_LIST=""
    SCHEMATIC_ID="base"
fi

echo -e "${GREEN}==== Muak initramfs build ====${NC}"
echo -e "${GREEN}Architecture: ${ARCH}${NC}"
echo -e "${GREEN}Schematic ID: ${SCHEMATIC_ID}${NC}"
echo -e "${GREEN}Extensions: ${EXTENSIONS:-none}${NC}"
echo -e "${GREEN}Output: ${OUTPUT_DIR}/initramfs.img${NC}"
echo

echo -e "${YELLOW}Building init binary...${NC}"
"$SCRIPT_DIR/build-init.sh" "$ARCH"

echo -e "${YELLOW}Building granola init system...${NC}"
cd "$PROJECT_ROOT/granola"
RUSTFLAGS='-C target-feature=+crt-static' cargo build --release --target x86_64-unknown-linux-gnu --quiet
cd - > /dev/null

echo
echo -e "${YELLOW}Creating initramfs structure...${NC}"

mkdir -p "$TEMP_DIR/rootfs_source/sbin"
mkdir -p "$TEMP_DIR/rootfs_source/dev"
mkdir -p "$TEMP_DIR/rootfs_source/proc"
mkdir -p "$TEMP_DIR/rootfs_source/sys"
mkdir -p "$TEMP_DIR/rootfs_source/run"
mkdir -p "$TEMP_DIR/rootfs_source/etc"

echo -e "${YELLOW}Installing granola as /sbin/init...${NC}"
cp "$PROJECT_ROOT/granola/target/x86_64-unknown-linux-gnu/release/granola" "$TEMP_DIR/rootfs_source/sbin/init"
chmod +x "$TEMP_DIR/rootfs_source/sbin/init"

if [ -n "$EXTENSIONS" ]; then
    echo -e "${YELLOW}Preparing extensions manifest...${NC}"
    echo "extensions:" > "$TEMP_DIR/extensions.yaml"

    for ext in $EXT_LIST; do
        EXT_FILE="${ext}.sqsh"
        echo "  - name: $ext" >> "$TEMP_DIR/extensions.yaml"
        echo "    file: $EXT_FILE" >> "$TEMP_DIR/extensions.yaml"
    done

    cp "$TEMP_DIR/extensions.yaml" "$TEMP_DIR/rootfs_source/etc/extensions.yaml"

    echo
    echo -e "${YELLOW}Extensions manifest:${NC}"
    cat "$TEMP_DIR/extensions.yaml"
    echo
fi

if ! command -v mksquashfs &> /dev/null; then
    echo -e "${RED}ERROR: mksquashfs not found${NC}"
    echo -e "${RED}Install squashfs-tools package${NC}"
    exit 1
fi

echo -e "${YELLOW}Creating squashfs root filesystem...${NC}"
mksquashfs "$TEMP_DIR/rootfs_source" "$TEMP_DIR/rootfs.sqsh" -comp xz -Xbcj x86 -b 1M -noappend -no-progress

mkdir -p "$TEMP_DIR/initramfs"
cp "$OUTPUT_DIR/init" "$TEMP_DIR/initramfs/init"
chmod +x "$TEMP_DIR/initramfs/init"
cp "$TEMP_DIR/rootfs.sqsh" "$TEMP_DIR/initramfs/rootfs.sqsh"

if [ -n "$EXTENSIONS" ]; then
    cp "$TEMP_DIR/extensions.yaml" "$TEMP_DIR/initramfs/extensions.yaml"

    for ext in $EXT_LIST; do
        EXT_FILE="${ext}.sqsh"
        cp "$PROJECT_ROOT/build/extensions/${EXT_FILE}" "$TEMP_DIR/initramfs/"
    done
fi

echo
echo -e "${YELLOW}Packaging initramfs with cpio and xz...${NC}"

cd "$TEMP_DIR/initramfs"
find . -print0 | cpio -o -H newc --null --quiet 2>/dev/null | xz --check=crc32 --x86 --lzma2=dict=1MiB > "$OUTPUT_DIR/initramfs.img"

echo
echo -e "${GREEN}==== initramfs Build Complete ====${NC}"
echo -e "${GREEN}Schematic ID: ${SCHEMATIC_ID}${NC}"
echo -e "${GREEN}initramfs: ${OUTPUT_DIR}/initramfs.img${NC}"
ls -lh "${OUTPUT_DIR}/initramfs.img"
