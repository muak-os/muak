REGISTRY ?= ghcr.io/sawangg
TAG ?= latest
PLATFORM ?= linux/amd64

ARCH ?= x86_64

ARTIFACTS := _out

CARGO_TARGET := x86_64-unknown-linux-musl
UEFI_TARGET := x86_64-unknown-uefi
RELEASE_DIR := target/$(CARGO_TARGET)/release
UEFI_RELEASE_DIR := target/$(UEFI_TARGET)/release

EXTENSIONS ?=

SOURCE_DATE_EPOCH ?= $(shell git log -1 --pretty=%ct 2>/dev/null || date +%s)

CONTAINER_RUNTIME ?= $(shell command -v podman >/dev/null 2>&1 && echo podman || echo docker)

PACKAGES := granola init imager yuki stub cloud-hypervisor firecracker qemu kernel

BOLD := \e[1m
CYAN := \e[36m
GREEN := \e[32m
YELLOW := \e[33m
RED := \e[31m
RESET := \e[0m

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

define require
	@test -f $(1) || { printf "$(RED)$(BOLD)Error:$(RESET) $(1) not found. Run $(GREEN)$(2)$(RESET) first\n"; exit 1; }
endef

define require-pkg
	@test -f pkgs/$(1)/Dockerfile || { printf "$(RED)$(BOLD)Error:$(RESET) pkgs/$(1)/Dockerfile not found\n"; exit 1; }
endef

.PHONY: help
help: ## Show this help
	@printf "\n$(BOLD)Muak$(RESET)\n\n"
	@printf "A minimal, immutable, API-driven Linux distribution for running VMs.\n\n"
	@printf "$(BOLD)$(CYAN)Prerequisites$(RESET)\n\n"
	@printf "To build this project, you must have the following installed:\n\n"
	@printf "  - rustup with musl targets (see README.md)\n"
	@printf "  - make\n"
	@printf "  - docker (with buildx) or podman\n\n"
	@printf "$(BOLD)$(CYAN)Quick Start$(RESET)\n\n"
	@printf "  $(GREEN)make kernel$(RESET)         Build the kernel locally\n"
	@printf "  $(GREEN)make dev$(RESET)            Full build chain\n\n"
	@printf "$(BOLD)$(CYAN)Artifacts$(RESET)\n\n"
	@printf "All artifacts will be output to $(YELLOW)./$(ARTIFACTS)$(RESET). Images will be tagged with\n"
	@printf "the registry $(YELLOW)$(REGISTRY)$(RESET) and tag $(YELLOW)$(TAG)$(RESET).\n\n"
	@printf "$(BOLD)$(CYAN)Targets$(RESET)\n\n"
	@grep -E '^[a-zA-Z_%-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[32m%-20s\033[0m %s\n", $$1, $$2}'
	@printf "\n$(BOLD)$(CYAN)Available packages$(RESET): $(PACKAGES)\n\n"

$(ARTIFACTS):
	@mkdir -p $(ARTIFACTS)

local-%: ## Build package and output locally to ARTIFACTS (e.g., make local-kernel)
	$(call require-pkg,$*)
	@mkdir -p $(ARTIFACTS)
	@echo "Building $* locally (using $(CONTAINER_RUNTIME))"
	@$(BUILD) \
		$(COMMON_ARGS) \
		$(TARGET_ARGS) \
		--output type=local,dest=$(ARTIFACTS) \
		--file pkgs/$*/Dockerfile \
		.

.PHONY: kernel
kernel: ## Build kernel and output to ARTIFACTS
	@$(MAKE) local-kernel

oci-%: ## Build OCI image and tag for registry (e.g., make oci-granola)
	$(call require-pkg,$*)
	@echo "Building OCI image: $* (using $(CONTAINER_RUNTIME))"
	@$(BUILD) \
		$(COMMON_ARGS) \
		$(TARGET_ARGS) \
		--tag $(REGISTRY)/pkgs/$*:$(TAG) \
		--file pkgs/$*/Dockerfile \
		.

.PHONY: oci-installer
oci-installer: ## Build installer OCI image (uses registry packages)
	@echo "Building installer OCI image (using $(CONTAINER_RUNTIME))"
	@$(BUILD) \
		$(COMMON_ARGS) \
		--tag $(REGISTRY)/pkgs/installer:$(TAG) \
		--file Dockerfile \
		.

.PHONY: local-installer
local-installer:
	@$(MAKE) installer

.PHONY: installer
installer: packages $(ARTIFACTS) ## Build installer with local binaries and extract to ARTIFACTS
	$(call require,$(ARTIFACTS)/bzImage,make kernel)
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

.PHONY: dev
dev: packages installer extensions uki iso ## Full development build chain
	@echo "Build complete: $(ARTIFACTS)/muak-$(ARCH).iso"

.PHONY: packages
packages: ## Build Rust packages with cargo
	@cargo build --release --target $(CARGO_TARGET)
	@cargo +nightly build --release --target $(UEFI_TARGET) --features uefi -p stub

.PHONY: extensions
extensions: $(ARTIFACTS) ## Extend base initramfs with extension
	$(call require,$(ARTIFACTS)/run/install/$(ARCH)/base-initramfs.img,make installer)
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

.PHONY: uki
uki: $(ARTIFACTS) ## Build UKI installer assets
	$(call require,$(ARTIFACTS)/run/install/$(ARCH)/stub.efi,make installer)
	$(call require,$(ARTIFACTS)/run/install/$(ARCH)/bzImage,make installer)
	$(call require,$(ARTIFACTS)/initramfs.img,make extensions)
	@echo -n "console=tty0 console=ttyS0 init=/init" > $(ARTIFACTS)/cmdline.txt
	@$(RELEASE_DIR)/yuki \
		--stub $(ARTIFACTS)/run/install/$(ARCH)/stub.efi \
		--linux $(ARTIFACTS)/run/install/$(ARCH)/bzImage \
		--initrd $(ARTIFACTS)/initramfs.img \
		--cmdline $(ARTIFACTS)/cmdline.txt \
		--output $(ARTIFACTS)/muak-$(ARCH).efi
	@echo "UKI built: $(ARTIFACTS)/muak-$(ARCH).efi"

.PHONY: iso
iso: $(ARTIFACTS) ## Builds the ISO and outputs it to the artifact directory
	$(call require,$(ARTIFACTS)/muak-$(ARCH).efi,make uki)
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

.PHONY: clean
clean: ## Remove build artifacts
	@echo "Cleaning build artifacts..."
	@cargo clean
	@rm -rf $(ARTIFACTS)
	@$(CONTAINER_RUNTIME) rm -f kernel-extract 2>/dev/null || true
