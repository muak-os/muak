REGISTRY ?= ghcr.io/sawangg
TAG ?= $(shell git describe --tag --always --dirty --match v[0-9]\* 2>/dev/null || echo dev)
SHA ?= $(shell git describe --match=none --always --abbrev=8 --dirty)
SOURCE_DATE_EPOCH ?= $(shell git log -1 --pretty=%ct)

PUSH ?= false
LATEST ?= false
PLATFORM ?= linux/amd64
PROGRESS ?= auto
CI_ARGS ?=
SIGNING_ARGS ?=

ARTIFACTS := _out
ARCH ?= x86_64
CARGO_TARGET := x86_64-unknown-linux-musl
UEFI_TARGET := x86_64-unknown-uefi
RELEASE_DIR := target/$(CARGO_TARGET)/release
UEFI_RELEASE_DIR := target/$(UEFI_TARGET)/release

EXTENSIONS ?=

CONTAINER_RUNTIME ?= $(shell command -v docker >/dev/null 2>&1 && echo docker || echo podman)

ifeq ($(CONTAINER_RUNTIME),podman)
	BUILD := podman build
else
	BUILD := docker buildx build
endif

COMMON_ARGS := --platform=$(PLATFORM)
COMMON_ARGS += --progress=$(PROGRESS)
COMMON_ARGS += --build-arg SOURCE_DATE_EPOCH=$(SOURCE_DATE_EPOCH)
COMMON_ARGS += --build-arg TAG=$(TAG)
ifneq ($(CONTAINER_RUNTIME),podman)
	COMMON_ARGS += --provenance=false
	PUSH_ARG := --push=$(PUSH)
else
	PUSH_ARG :=
endif

BOLD := \e[1m
CYAN := \e[36m
GREEN := \e[32m
YELLOW := \e[33m
RED := \e[31m
RESET := \e[0m

define require
	@test -f $(1) || { printf "$(RED)$(BOLD)Error:$(RESET) $(1) not found. Run $(GREEN)$(2)$(RESET) first\n"; exit 1; }
endef

define require-pkg
	@test -f pkgs/$(1)/Dockerfile || { printf "$(RED)$(BOLD)Error:$(RESET) pkgs/$(1)/Dockerfile not found\n"; exit 1; }
endef

define require-docker-for-push
	@if [ "$(PUSH)" = "true" ] && [ "$(CONTAINER_RUNTIME)" = "podman" ]; then \
		printf "$(RED)$(BOLD)Error:$(RESET) PUSH=true requires Docker (podman does not support --push)\n"; \
		printf "$(YELLOW)Hint:$(RESET) Set CONTAINER_RUNTIME=docker or use 'make local-%%' instead\n"; \
		exit 1; \
	fi
endef

# Help Menu
.PHONY: help
help: ## Show this help
	@printf "\n$(BOLD)Muak$(RESET)\n\n"
	@printf "A minimal, immutable, API-driven Linux distribution for running VMs.\n\n"
	@printf "$(BOLD)$(CYAN)Prerequisites$(RESET)\n\n"
	@printf "To build this project, you must have the following installed:\n\n"
	@printf "  - rustup with musl targets (see README.md)\n"
	@printf "  - make\n"
	@printf "  - docker (with buildx) or podman\n"
	@printf "  - git\n\n"
	@printf "$(BOLD)$(CYAN)Quick Start$(RESET)\n\n"
	@printf "  $(GREEN)make kernel$(RESET)         Build the kernel locally\n"
	@printf "  $(GREEN)make dev$(RESET)            Full build chain\n\n"
	@printf "$(BOLD)$(CYAN)Artifacts$(RESET)\n\n"
	@printf "All artifacts will be output to $(YELLOW)./$(ARTIFACTS)$(RESET). Images will be tagged with\n"
	@printf "the registry $(YELLOW)$(REGISTRY)$(RESET) and tag $(YELLOW)$(TAG)$(RESET).\n\n"
	@printf "$(BOLD)$(CYAN)Targets$(RESET)\n\n"
	@grep -E '^[a-zA-Z_%-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[32m%-20s\033[0m %s\n", $$1, $$2}'

# Build Abstractions
$(ARTIFACTS):
	@mkdir -p $(ARTIFACTS)

local-%: $(ARTIFACTS) ## Build package as local OCI layout (e.g. make local-granola)
	$(call require-pkg,$*)
	@printf "$(CYAN)Building local:$(RESET) $* -> $(ARTIFACTS)/oci/$*\n"
	@mkdir -p $(ARTIFACTS)/oci
	@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) \
		--tag localhost/muak-$*:$(TAG) \
		--load \
		--file pkgs/$*/Dockerfile \
		.
	@$(CONTAINER_RUNTIME) save --format oci-dir -o $(ARTIFACTS)/oci/$* localhost/muak-$*:$(TAG)
	@$(CONTAINER_RUNTIME) rmi localhost/muak-$*:$(TAG) >/dev/null 2>&1 || true

oci-%: $(ARTIFACTS) ## Build OCI image (e.g. make oci-granola)
	$(call require-pkg,$*)
	$(call require-docker-for-push)
	@printf "$(CYAN)Building OCI:$(RESET) $* (push=$(PUSH), latest=$(LATEST))\n"
	@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) \
		--tag $(REGISTRY)/pkgs/$*:$(TAG) \
		$(if $(filter true,$(LATEST)),--tag $(REGISTRY)/pkgs/$*:latest) \
		$(PUSH_ARG) \
		--file pkgs/$*/Dockerfile \
		.

## Kernel
.PHONY: kernel
kernel: $(ARTIFACTS) ## Build kernel to local artifacts (unsigned)
	$(call require-pkg,kernel)
	@printf "$(CYAN)Building kernel locally$(RESET)\n"
	@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) \
		--output type=local,dest=$(ARTIFACTS) \
		--file pkgs/kernel/Dockerfile \
		.

.PHONY: oci-kernel
oci-kernel: ## Build kernel OCI image (signed in CI)
	$(call require-docker-for-push)
	@printf "$(CYAN)Building kernel OCI$(RESET) (push=$(PUSH), latest=$(LATEST))\n"
	@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) $(SIGNING_ARGS) \
		--tag $(REGISTRY)/kernel:$(TAG) \
		$(if $(filter true,$(LATEST)),--tag $(REGISTRY)/kernel:latest) \
		$(PUSH_ARG) \
		--target kernel-package \
		--file pkgs/kernel/Dockerfile \
		.

.PHONY: kspp
kspp: ## Check kernel config against KSPP security hardening recommendations
	@printf "$(CYAN)Checking kernel config against KSPP recommendations$(RESET)\n"
	@$(CONTAINER_RUNTIME) run --rm --network=host -v $(PWD)/pkgs/kernel/config-amd64:/config:ro \
		alpine:3.23 sh -c '\
		apk add --no-cache git python3 >/dev/null 2>&1 && \
		git clone --depth 1 --quiet https://github.com/a13xp0p0v/kernel-hardening-checker.git /tmp/khc && \
		/tmp/khc/bin/kernel-hardening-checker -c /config'

## Installer
.PHONY: installer
installer: $(ARTIFACTS) ## Build installer with local binaries
	$(call require,$(ARTIFACTS)/bzImage,make kernel)
	@printf "$(CYAN)Building installer with local binaries$(RESET)\n"
	@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) \
		--build-context pkg-granola=$(RELEASE_DIR) \
		--build-context pkg-modd=$(RELEASE_DIR) \
		--build-context pkg-networkd=$(RELEASE_DIR) \
		--build-context pkg-apid=$(RELEASE_DIR) \
		--build-context pkg-vmd=$(RELEASE_DIR) \
		--build-context pkg-init=$(RELEASE_DIR) \
		--build-context pkg-yuki=$(RELEASE_DIR) \
		--build-context pkg-imager=$(RELEASE_DIR) \
		--build-context pkg-stub=$(UEFI_RELEASE_DIR) \
		--build-context pkg-kernel=$(ARTIFACTS) \
		--output type=local,dest=$(ARTIFACTS) \
		--file Dockerfile \
		.
	@printf "$(GREEN)Installer assets extracted to $(ARTIFACTS)/$(RESET)\n"

.PHONY: oci-installer
oci-installer: ## Build installer OCI image from registry packages
	$(call require-docker-for-push)
	@printf "$(CYAN)Building installer OCI$(RESET) (push=$(PUSH), latest=$(LATEST))\n"
	@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) \
		--build-arg PKG_KERNEL=$(REGISTRY)/kernel:$(TAG) \
		--build-arg PKG_GRANOLA=$(REGISTRY)/pkgs/granola:$(TAG) \
		--build-arg PKG_MODD=$(REGISTRY)/pkgs/modd:$(TAG) \
		--build-arg PKG_NETWORKD=$(REGISTRY)/pkgs/networkd:$(TAG) \
		--build-arg PKG_APID=$(REGISTRY)/pkgs/apid:$(TAG) \
		--build-arg PKG_VMD=$(REGISTRY)/pkgs/vmd:$(TAG) \
		--build-arg PKG_INIT=$(REGISTRY)/pkgs/init:$(TAG) \
		--build-arg PKG_IMAGER=$(REGISTRY)/pkgs/imager:$(TAG) \
		--build-arg PKG_YUKI=$(REGISTRY)/pkgs/yuki:$(TAG) \
		--build-arg PKG_STUB=$(REGISTRY)/pkgs/stub:$(TAG) \
		--tag $(REGISTRY)/installer:$(TAG) \
		$(if $(filter true,$(LATEST)),--tag $(REGISTRY)/installer:latest) \
		$(PUSH_ARG) \
		--file Dockerfile \
		.

## Rust Packages
.PHONY: packages
packages: ## Build all Rust packages with cargo
	@printf "$(CYAN)Building Rust packages$(RESET)\n"
	@cargo build --release --target $(CARGO_TARGET)
	@cargo +nightly build --release --target $(UEFI_TARGET) --features uefi -p stub

## Extensions
.PHONY: extensions
extensions: $(ARTIFACTS) ## Extend base initramfs with specified extensions
	$(call require,$(ARTIFACTS)/base-initramfs.img,make installer)
	@if [ -z "$(EXTENSIONS)" ]; then \
		printf "$(YELLOW)No extensions specified, using base initramfs$(RESET)\n"; \
		cp $(ARTIFACTS)/base-initramfs.img $(ARTIFACTS)/initramfs.img; \
	else \
		printf "$(CYAN)Building initramfs with extensions:$(RESET) $(EXTENSIONS)\n"; \
		$(RELEASE_DIR)/imager build \
			--base $(ARTIFACTS)/base-initramfs.img \
			$(foreach ext,$(EXTENSIONS),--extension $(ext)) \
			--output $(ARTIFACTS)/initramfs.img; \
	fi
	@printf "$(GREEN)Initramfs ready:$(RESET) $(ARTIFACTS)/initramfs.img\n"

## Images artifacts
.PHONY: uki
uki: $(ARTIFACTS) ## Build UKI (Unified Kernel Image)
	$(call require,$(ARTIFACTS)/stub.efi,make installer)
	$(call require,$(ARTIFACTS)/bzImage,make installer)
	$(call require,$(ARTIFACTS)/initramfs.img,make extensions)
	@printf "$(CYAN)Building UKI$(RESET)\n"
	@echo -n "console=tty0 console=ttyS0 init=/init" > $(ARTIFACTS)/cmdline.txt
	@$(RELEASE_DIR)/yuki \
		--stub $(ARTIFACTS)/stub.efi \
		--linux $(ARTIFACTS)/bzImage \
		--initrd $(ARTIFACTS)/initramfs.img \
		--cmdline $(ARTIFACTS)/cmdline.txt \
		--output $(ARTIFACTS)/muak-$(ARCH).efi
	@printf "$(GREEN)UKI built:$(RESET) $(ARTIFACTS)/muak-$(ARCH).efi\n"

.PHONY: iso
iso: $(ARTIFACTS) ## Build bootable ISO
	$(call require,$(ARTIFACTS)/muak-$(ARCH).efi,make uki)
	@printf "$(CYAN)Building ISO$(RESET)\n"
	@$(CONTAINER_RUNTIME) run --rm --network=host -v $(PWD)/$(ARTIFACTS):/out alpine:3.23 sh -c '\
		set -euo pipefail && \
		apk add --no-cache mtools dosfstools xorriso >/dev/null 2>&1 && \
		rm -rf /out/iso && mkdir -p /out/iso/EFI/BOOT && \
		cp /out/muak-$(ARCH).efi /out/iso/EFI/BOOT/BOOTX64.EFI && \
		dd if=/dev/zero of=/out/iso/efiboot.img bs=1M count=42 2>/dev/null && \
		mkfs.vfat /out/iso/efiboot.img >/dev/null && \
		mmd -i /out/iso/efiboot.img ::/EFI ::/EFI/BOOT && \
		mcopy -i /out/iso/efiboot.img /out/muak-$(ARCH).efi ::/EFI/BOOT/BOOTX64.EFI && \
		xorriso -as mkisofs -o /out/muak-$(ARCH).iso -e efiboot.img -no-emul-boot -V MUAK /out/iso && \
		rm -rf /out/iso'
	@printf "$(GREEN)ISO built:$(RESET) $(ARTIFACTS)/muak-$(ARCH).iso\n"

# Development
.PHONY: dev
dev: packages installer extensions uki iso ## Full local development build
	@printf "$(GREEN)$(BOLD)Build complete:$(RESET) $(ARTIFACTS)/muak-$(ARCH).iso\n"

# Cleanup
.PHONY: clean
clean: ## Remove all build artifacts
	@printf "$(CYAN)Cleaning build artifacts$(RESET)\n"
	@cargo clean
	@rm -rf $(ARTIFACTS)
	@$(CONTAINER_RUNTIME) rm -f kernel-extract 2>/dev/null || true
	@printf "$(GREEN)Clean complete$(RESET)\n"
