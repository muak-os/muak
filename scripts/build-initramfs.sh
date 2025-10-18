#!/bin/bash
set -euo pipefail

NC='\033[0m'
BOLD='\033[1m'
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'

ARCH="${ARCH:-x86_64}"
INITRAMFS_SOURCE="$(pwd)/initramfs"
DINIT_DIR="$(pwd)/output/${ARCH}/dinit"
BUILD_DIR="$(pwd)/build/${ARCH}/initramfs"
OUTPUT_DIR="$(pwd)/output/${ARCH}"

echo -e "${BOLD}${CYAN}==== Muak initramfs Build ====${NC}"
echo -e "${BOLD}${YELLOW}Architecture:${NC} ${ARCH}"
echo -e "${BOLD}${YELLOW}Initramfs Source:${NC} ${INITRAMFS_SOURCE}"
echo -e "${BOLD}${YELLOW}Build Directory:${NC} ${BUILD_DIR}"
echo -e "${BOLD}${YELLOW}Output Directory:${NC} ${OUTPUT_DIR}"
echo

if [ ! -f "${DINIT_DIR}/dinit" ]; then
    echo -e "${BOLD}${RED}ERROR:${NC} dinit binary not found at ${DINIT_DIR}/dinit"
    echo -e "${YELLOW}Please run build-dinit.sh first${NC}"
    exit 1
fi

rm -rf "${BUILD_DIR}"
mkdir -p "${BUILD_DIR}"
mkdir -p "${OUTPUT_DIR}"

echo -e "${BOLD}${GREEN}Creating initramfs directory structure...${NC}"
mkdir -p "${BUILD_DIR}"/{bin,sbin,etc,proc,sys,dev,root,mnt/root,newroot}

echo -e "${BOLD}${GREEN}Copying init script and helpers...${NC}"
cp -a "${INITRAMFS_SOURCE}/init" "${BUILD_DIR}/init"
cp -a "${INITRAMFS_SOURCE}/bin/"* "${BUILD_DIR}/bin/" || true

echo -e "${BOLD}${GREEN}Copying dinit and dinitctl...${NC}"
cp -a "${DINIT_DIR}/dinit" "${BUILD_DIR}/sbin/dinit"
cp -a "${DINIT_DIR}/dinitctl" "${BUILD_DIR}/bin/dinitctl"

echo -e "${BOLD}${GREEN}Copying dinit service files...${NC}"
mkdir -p "${BUILD_DIR}/etc/dinit.d"
cp -a "${INITRAMFS_SOURCE}/etc/dinit.d/"* "${BUILD_DIR}/etc/dinit.d/"

echo -e "${BOLD}${GREEN}Installing busybox...${NC}"
if [ "${ARCH}" = "arm64" ]; then
    if ! command -v qemu-aarch64-static &> /dev/null; then
        echo -e "${BOLD}${RED}ERROR:${NC} qemu-aarch64-static not found. Install qemu-user-static"
        exit 1
    fi

    BUSYBOX_PATH="/usr/bin/busybox"
    if [ ! -f "${BUSYBOX_PATH}" ]; then
        echo -e "${BOLD}${RED}ERROR:${NC} busybox not found at ${BUSYBOX_PATH}"
        exit 1
    fi

    cp "${BUSYBOX_PATH}" "${BUILD_DIR}/bin/busybox"
else
    BUSYBOX_PATH="/bin/busybox"
    if [ ! -f "${BUSYBOX_PATH}" ]; then
        echo -e "${BOLD}${RED}ERROR:${NC} busybox not found at ${BUSYBOX_PATH}"
        echo -e "${YELLOW}Please install busybox with: emerge --ask sys-apps/busybox${NC}"
        exit 1
    fi

    cp "${BUSYBOX_PATH}" "${BUILD_DIR}/bin/busybox"
fi

echo -e "${BOLD}${BLUE}Packaging initramfs...${NC}"
cd "${BUILD_DIR}"
find . -print0 | cpio --null --create --verbose --format=newc | gzip --best > "${OUTPUT_DIR}/initramfs.cpio.gz"

echo -e "${BOLD}${CYAN}==== initramfs Build Complete ====${NC}"
echo -e "${BOLD}${GREEN}initramfs:${NC} ${OUTPUT_DIR}/initramfs.cpio.gz"
ls -lh "${OUTPUT_DIR}/initramfs.cpio.gz"
