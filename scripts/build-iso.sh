#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

ARCH="${1:-x86_64}"
UKI_FILE="$PROJECT_ROOT/build/muak-${ARCH}.efi"
ISO_DIR="$PROJECT_ROOT/build/iso"
OUTPUT_DIR="$PROJECT_ROOT/build"
ISO_OUTPUT="${OUTPUT_DIR}/muak-${ARCH}.iso"

echo -e "${GREEN}==== Muak ISO Build ====${NC}"
echo -e "${GREEN}Architecture: ${ARCH}${NC}"
echo -e "${GREEN}UKI: ${UKI_FILE}${NC}"
echo

if [ ! -f "${UKI_FILE}" ]; then
    echo -e "${RED}ERROR: UKI not found at ${UKI_FILE}${NC}"
    echo -e "${YELLOW}Run ./scripts/build-uki.sh ${ARCH} first${NC}"
    exit 1
fi

if ! command -v xorriso &> /dev/null; then
    echo -e "${RED}ERROR: xorriso not found${NC}"
    echo -e "${RED}Install xorriso package${NC}"
    exit 1
fi

if ! command -v mtools &> /dev/null; then
    echo -e "${RED}ERROR: mtools not found${NC}"
    echo -e "${RED}Install mtools package${NC}"
    exit 1
fi

rm -rf "${ISO_DIR}"
mkdir -p "${ISO_DIR}/EFI/BOOT"

if [ "${ARCH}" = "arm64" ]; then
    cp "${UKI_FILE}" "${ISO_DIR}/EFI/BOOT/BOOTAA64.EFI"
else
    cp "${UKI_FILE}" "${ISO_DIR}/EFI/BOOT/BOOTX64.EFI"
fi

echo -e "${YELLOW}Creating ISO image...${NC}"

# Create ESP image for El Torito
ESP_IMG="${ISO_DIR}/efiboot.img"
dd if=/dev/zero of="${ESP_IMG}" bs=1M count=10
mkfs.vfat "${ESP_IMG}"
mmd -i "${ESP_IMG}" ::/EFI
mmd -i "${ESP_IMG}" ::/EFI/BOOT
mcopy -i "${ESP_IMG}" "${ISO_DIR}/EFI/BOOT/BOOT$([ "${ARCH}" = "arm64" ] && echo "AA64" || echo "X64").EFI" ::/EFI/BOOT/

xorriso -as mkisofs \
    -o "${ISO_OUTPUT}" \
    -e efiboot.img \
    -no-emul-boot \
    -V "MUAK" \
    "${ISO_DIR}"

rm -rf "${ISO_DIR}"

echo
echo -e "${GREEN}==== ISO Build Complete ====${NC}"
echo -e "${GREEN}ISO: ${ISO_OUTPUT}${NC}"
