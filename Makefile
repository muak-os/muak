# Registry and versioning
REGISTRY ?= ghcr.io/sawangg
TAG ?= latest
PLATFORM ?= linux/amd64

# Architecture
ARCH ?= x86_64

# Artifacts directory
ARTIFACTS := build

# Cargo targets
CARGO_TARGET := x86_64-unknown-linux-musl
UEFI_TARGET := x86_64-unknown-uefi
RELEASE_DIR := target/$(CARGO_TARGET)/release
UEFI_RELEASE_DIR := target/$(UEFI_TARGET)/release

# Extensions to include (space-separated OCI image refs)
EXTENSIONS ?=

# Source date for reproducible builds
SOURCE_DATE_EPOCH ?= $(shell git log -1 --pretty=%ct 2>/dev/null || date +%s)

# Detect container runtime (podman or docker)
CONTAINER_RUNTIME ?= $(shell command -v podman >/dev/null 2>&1 && echo podman || echo docker)

ifeq ($(CONTAINER_RUNTIME),podman)
	BUILD := podman build
	COMMON_ARGS := --platform=$(PLATFORM)
	COMMON_ARGS += --build-arg SOURCE_DATE_EPOCH=$(SOURCE_DATE_EPOCH)
else
	BUILD := docker buildx build
	COMMON_ARGS := --provenance=false
	COMMON_ARGS += --platform=$(PLATFORM)
	COMMON_ARGS += --build-arg SOURCE_DATE_EPOCH=$(SOURCE_DATE_EPOCH)
	COMMON_ARGS += --progress=plain
endif

# Package definitions (packages with Dockerfiles in pkgs/)
PACKAGES := granola init imager yuki stub cloud-hypervisor firecracker qemu kernel

.PHONY: help clean dev packages initramfs uki iso kernel-pull

help: ## Show this help
	@grep -E '^[a-zA-Z_%-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-20s\033[0m %s\n", $$1, $$2}'
	@echo ""
	@echo "Available packages: $(PACKAGES)"

$(ARTIFACTS):
	@mkdir -p $(ARTIFACTS)

local-%: ## Build package and output locally to ARTIFACTS (e.g., make local-kernel)
	@if [ ! -f "pkgs/$*/Dockerfile" ]; then \
		echo "Error: pkgs/$*/Dockerfile not found"; \
		exit 1; \
	fi
	@mkdir -p $(ARTIFACTS)
	@echo "Building $* locally (using $(CONTAINER_RUNTIME))"
	@$(BUILD) \
		$(COMMON_ARGS) \
		$(TARGET_ARGS) \
		--output type=local,dest=$(ARTIFACTS) \
		--file pkgs/$*/Dockerfile \
		.

kernel: ## Build kernel and output to ARTIFACTS
	@$(MAKE) local-kernel

oci-%: ## Build OCI image and tag for registry (e.g., make oci-granola)
	@if [ ! -f "pkgs/$*/Dockerfile" ]; then \
		echo "Error: pkgs/$*/Dockerfile not found"; \
		exit 1; \
	fi
	@echo "Building OCI image: $* (using $(CONTAINER_RUNTIME))"
	@$(BUILD) \
		$(COMMON_ARGS) \
		$(TARGET_ARGS) \
		--tag $(REGISTRY)/pkgs/$*:$(TAG) \
		--file pkgs/$*/Dockerfile \
		.

oci-installer: ## Build installer OCI image (uses registry packages)
	@echo "Building installer OCI image (using $(CONTAINER_RUNTIME))"
	@$(BUILD) \
		$(COMMON_ARGS) \
		--tag $(REGISTRY)/pkgs/installer:$(TAG) \
		--file Dockerfile \
		.

local-installer:
	@$(MAKE) installer

installer: packages $(ARTIFACTS) ## Build installer with local binaries and extract to ARTIFACTS
	@test -f $(ARTIFACTS)/bzImage || { echo "Error: Kernel not found. Run 'make kernel' first"; exit 1; }
	@echo "Building installer with local binaries (using $(CONTAINER_RUNTIME))"
	@$(BUILD) \
		$(COMMON_ARGS) \
		--build-context pkg-granola=$(RELEASE_DIR) \
		--build-context pkg-init=$(RELEASE_DIR) \
		--build-context pkg-yuki=$(RELEASE_DIR) \
		--build-context pkg-imager=$(RELEASE_DIR) \
		--build-context pkg-stub=$(UEFI_RELEASE_DIR) \
		--build-context pkg-kernel=$(ARTIFACTS) \
		--output type=local,dest=$(ARTIFACTS) \
		--file Dockerfile \
		.
	@echo "Installer assets extracted to $(ARTIFACTS)/"

# ======================================

dev: packages installer extensions uki iso ## Full development build chain
	@echo "Build complete: $(ARTIFACTS)/muak-$(ARCH).iso"

packages: ## Build Rust packages with cargo
	@cargo build --release --target $(CARGO_TARGET)
	@cargo +nightly build --release --target $(UEFI_TARGET) --features uefi -p stub

extensions: $(ARTIFACTS) ## Build initramfs (with extensions if EXTENSIONS is set)
	@test -f $(ARTIFACTS)/run/install/$(ARCH)/base-initramfs.img || { echo "Error: Base initramfs not found. Run 'make installer' first"; exit 1; }
	@if [ -z "$(EXTENSIONS)" ]; then \
		echo "No extensions specified, using base initramfs"; \
		cp $(ARTIFACTS)/run/install/$(ARCH)/base-initramfs.img $(ARTIFACTS)/initramfs.img; \
	else \
		echo "Building initramfs with extensions: $(EXTENSIONS)"; \
		$(RELEASE_DIR)/imager \
			--base $(ARTIFACTS)/run/install/$(ARCH)/base-initramfs.img \
			$(foreach ext,$(EXTENSIONS),--extension $(ext)) \
			--output $(ARTIFACTS)/initramfs.img; \
	fi
	@echo "Initramfs ready: $(ARTIFACTS)/initramfs.img"

uki: $(ARTIFACTS) ## Build UKI from initramfs
	@test -f $(ARTIFACTS)/run/install/$(ARCH)/stub.efi || { echo "Error: Assets not found. Run 'make installer' first"; exit 1; }
	@test -f $(ARTIFACTS)/run/install/$(ARCH)/bzImage || { echo "Error: Assets not found. Run 'make installer' first"; exit 1; }
	@test -f $(ARTIFACTS)/initramfs.img || { echo "Error: Initramfs not found. Run 'make initramfs' first"; exit 1; }
	@echo -n "console=tty0 console=ttyS0 init=/init" > $(ARTIFACTS)/cmdline.txt
	@$(RELEASE_DIR)/yuki \
		--stub $(ARTIFACTS)/run/install/$(ARCH)/stub.efi \
		--linux $(ARTIFACTS)/run/install/$(ARCH)/bzImage \
		--initrd $(ARTIFACTS)/initramfs.img \
		--cmdline $(ARTIFACTS)/cmdline.txt \
		--output $(ARTIFACTS)/muak-$(ARCH).efi
	@echo "UKI built: $(ARTIFACTS)/muak-$(ARCH).efi"

iso: $(ARTIFACTS) ## Builds the ISO and outputs it to the artifact directory
	@test -f $(ARTIFACTS)/muak-$(ARCH).efi || { echo "Error: UKI not found. Run 'make uki' first"; exit 1; }
	@$(CONTAINER_RUNTIME) run --rm -v $(PWD)/$(ARTIFACTS):/out alpine:3.23 sh -c '\
		set -euo pipefail && \
		apk add --no-cache mtools dosfstools xorriso >/dev/null 2>&1 && \
		rm -rf /out/iso && mkdir -p /out/iso/EFI/BOOT && \
		cp /out/muak-$(ARCH).efi /out/iso/EFI/BOOT/BOOTX64.EFI && \
		dd if=/dev/zero of=/out/iso/efiboot.img bs=1M count=29 2>/dev/null && \
		mkfs.vfat /out/iso/efiboot.img >/dev/null && \
		mmd -i /out/iso/efiboot.img ::/EFI ::/EFI/BOOT && \
		mcopy -i /out/iso/efiboot.img /out/muak-$(ARCH).efi ::/EFI/BOOT/BOOTX64.EFI && \
		xorriso -as mkisofs -o /out/muak-$(ARCH).iso -e efiboot.img -no-emul-boot -V MUAK /out/iso && \
		rm -rf /out/iso'
	@echo "ISO built: $(ARTIFACTS)/muak-$(ARCH).iso"

clean: ## Remove build artifacts
	@echo "Cleaning build artifacts..."
	@cargo clean
	@rm -rf $(ARTIFACTS)
	@$(CONTAINER_RUNTIME) rm -f kernel-extract 2>/dev/null || true
