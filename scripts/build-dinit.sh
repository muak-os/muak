#!/bin/bash
set -euo pipefail

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

ARCH="${ARCH:-x86_64}"
DINIT_VERSION="${DINIT_VERSION:-0.19.0}"
BUILD_DIR="$(pwd)/build/${ARCH}/dinit"
OUTPUT_DIR="$(pwd)/output/${ARCH}/dinit"

echo -e "${GREEN}==== Muak dinit Build ====${NC}"
echo -e "${GREEN}Architecture: ${ARCH}${NC}"
echo -e "${GREEN}dinit Version: ${DINIT_VERSION}${NC}"
echo -e "${GREEN}Build Directory: ${BUILD_DIR}${NC}"
echo -e "${GREEN}Output Directory: ${OUTPUT_DIR}${NC}"
echo

mkdir -p "${BUILD_DIR}"
mkdir -p "${OUTPUT_DIR}"

cd "${BUILD_DIR}"

if [ ! -f "dinit-${DINIT_VERSION}.tar.xz" ]; then
    echo -e "${YELLOW}Downloading dinit ${DINIT_VERSION}...${NC}"
    wget "https://github.com/davmac314/dinit/releases/download/v${DINIT_VERSION}/dinit-${DINIT_VERSION}.tar.xz"
fi

if [ ! -d "dinit-${DINIT_VERSION}" ]; then
    echo -e "${YELLOW}Extracting dinit source...${NC}"
    tar -xf "dinit-${DINIT_VERSION}.tar.xz"
fi

cd "dinit-${DINIT_VERSION}"

if [ "${ARCH}" = "arm64" ]; then
    export CXX=aarch64-linux-gnu-g++
    export CC=aarch64-linux-gnu-gcc
    BUILD_OPTS="CXX=aarch64-linux-gnu-g++ BUILD_SHUTDOWN=no"
else
    BUILD_OPTS="BUILD_SHUTDOWN=no"
fi

echo -e "${YELLOW}Building dinit...${NC}"
make ${BUILD_OPTS} -j$(nproc)

echo -e "${YELLOW}Copying dinit binaries to output directory...${NC}"
cp src/dinit "${OUTPUT_DIR}/dinit"
cp src/dinitctl "${OUTPUT_DIR}/dinitctl"

if [ "${ARCH}" = "x86_64" ]; then
    echo -e "${YELLOW}Stripping symbols...${NC}"
    strip "${OUTPUT_DIR}/dinit"
    strip "${OUTPUT_DIR}/dinitctl"
else
    aarch64-linux-gnu-strip "${OUTPUT_DIR}/dinit"
    aarch64-linux-gnu-strip "${OUTPUT_DIR}/dinitctl"
fi

echo
echo -e "${GREEN}==== dinit Build Complete ====${NC}"
echo -e "${GREEN}dinit: ${OUTPUT_DIR}/dinit${NC}"
echo -e "${GREEN}dinitctl: ${OUTPUT_DIR}/dinitctl${NC}"
echo
