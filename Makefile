REGISTRY ?= ghcr.io/sawangg
TAG ?= $(shell git describe --tag --always --dirty --match v[0-9]\* 2>/dev/null || echo dev)
SHA ?= $(shell git describe --match=none --always --abbrev=8 --dirty)
SOURCE_DATE_EPOCH ?= $(shell git log -1 --pretty=%ct)

PUSH ?= false
LATEST ?= false
PROGRESS ?= auto
CI_ARGS ?=
SIGNING_ARGS ?=

ARTIFACTS := _out

ARCH ?= $(shell uname -m)

# Commands that take package arguments - remaining args should be treated as params, not targets
PARAM_COMMANDS := oci local test coverage
FIRST_GOAL := $(firstword $(MAKECMDGOALS))
OTHER_GOALS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))
IS_PARAM_CMD := $(filter $(FIRST_GOAL),$(PARAM_COMMANDS))

ifeq ($(ARCH),$(filter $(ARCH),aarch64 arm64))
    override ARCH := aarch64
    CARGO_TARGET := aarch64-unknown-linux-musl
    UEFI_TARGET := aarch64-unknown-uefi
    KERNEL_CONFIG := config-arm64
    BOOT_FILE := BOOTAA64.EFI
    PLATFORM := linux/arm64
    CMDLINE_FILE := pkgs/kernel/cmdline-arm64.txt
else
    CARGO_TARGET := x86_64-unknown-linux-musl
    UEFI_TARGET := x86_64-unknown-uefi
    KERNEL_CONFIG := config-amd64
    BOOT_FILE := BOOTX64.EFI
    PLATFORM := linux/amd64
    CMDLINE_FILE := pkgs/kernel/cmdline-amd64.txt
endif

RELEASE_DIR := target/$(CARGO_TARGET)/release
UEFI_RELEASE_DIR := target/$(UEFI_TARGET)/release

EXTENSIONS ?=
DTB ?=

CONTAINER_RUNTIME ?= $(shell command -v docker >/dev/null 2>&1 && echo docker || echo podman)

ifeq ($(CONTAINER_RUNTIME),podman)
	BUILD := podman build
	PULL_ARG := --pull=never
else
	BUILD := docker buildx build
	PULL_ARG :=
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

# Build Abstractions
$(ARTIFACTS):
	@mkdir -p $(ARTIFACTS)

# Catch-all for package names passed as arguments to param commands
.PHONY: $(OTHER_GOALS)
$(OTHER_GOALS):
	@:

define require
	@test -f $(1) || { printf "$(RED)$(BOLD)Error:$(RESET) $(1) not found. Run $(GREEN)$(2)$(RESET) first\n"; exit 1; }
endef

define require-pkg
	@test -f pkgs/$(1)/Dockerfile || { printf "$(RED)$(BOLD)Error:$(RESET) pkgs/$(1)/Dockerfile not found\n"; exit 1; }
endef

define require-docker-for-push
	@if [ "$(PUSH)" = "true" ] && [ "$(CONTAINER_RUNTIME)" = "podman" ]; then \
		printf "$(RED)$(BOLD)Error:$(RESET) PUSH=true requires Docker (podman does not support --push)\n"; \
		printf "$(YELLOW)Hint:$(RESET) Set CONTAINER_RUNTIME=docker\n"; \
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

# Build a single package as local OCI layout (internal helper)
define build-local
	@test -f pkgs/$(1)/Dockerfile || { printf "$(RED)$(BOLD)Error:$(RESET) pkgs/$(1)/Dockerfile not found\n"; exit 1; }
	@printf "$(CYAN)Building local:$(RESET) $(1) -> $(ARTIFACTS)/oci/$(1)\n"
	@mkdir -p $(ARTIFACTS)/oci
	@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) $(PULL_ARG) \
		--tag localhost/muak-$(1):$(TAG) \
		--load \
		--file pkgs/$(1)/Dockerfile \
		.
	@$(CONTAINER_RUNTIME) save --format oci-dir -o $(ARTIFACTS)/oci/$(1) localhost/muak-$(1):$(TAG)
	@$(CONTAINER_RUNTIME) rmi localhost/muak-$(1):$(TAG) >/dev/null 2>&1 || true
endef

.PHONY: local
local: $(ARTIFACTS) ## Build packages as local OCI layout (e.g. make local granola modd)
	@if [ -z "$(filter-out local,$(MAKECMDGOALS))" ]; then \
		printf "$(RED)$(BOLD)Error:$(RESET) No packages specified. Usage: make local <pkg1> [pkg2...]\n"; \
		exit 1; \
	fi
	$(foreach pkg,$(filter-out local,$(MAKECMDGOALS)),$(call build-local,$(pkg)))

# Build a single package as OCI image (handles special cases: kernel, installer, cli)
define build-oci
	$(if $(filter kernel,$(1)),\
		@printf "$(CYAN)Building kernel OCI$(RESET) (push=$(PUSH), latest=$(LATEST))\n"
		@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) $(SIGNING_ARGS) $(PULL_ARG) \
			--tag $(REGISTRY)/kernel:$(TAG) \
			$(if $(filter true,$(LATEST)),--tag $(REGISTRY)/kernel:latest) \
			$(PUSH_ARG) \
			--target kernel-package \
			--file pkgs/kernel/Dockerfile \
			.,\
	$(if $(filter installer,$(1)),\
		@printf "$(CYAN)Building installer OCI$(RESET) (push=$(PUSH), latest=$(LATEST))\n"
		@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) $(PULL_ARG) \
			--build-arg PKG_KERNEL=$(REGISTRY)/kernel:$(TAG) \
			--build-arg PKG_GRANOLA=$(REGISTRY)/pkgs/granola:$(TAG) \
			--build-arg PKG_MODD=$(REGISTRY)/pkgs/modd:$(TAG) \
			--build-arg PKG_NETWORKD=$(REGISTRY)/pkgs/networkd:$(TAG) \
			--build-arg PKG_APID=$(REGISTRY)/pkgs/apid:$(TAG) \
			--build-arg PKG_VMD=$(REGISTRY)/pkgs/vmd:$(TAG) \
			--build-arg PKG_INIT=$(REGISTRY)/pkgs/init:$(TAG) \
			--build-arg PKG_STUB=$(REGISTRY)/pkgs/stub:$(TAG) \
			--tag $(REGISTRY)/installer:$(TAG) \
			$(if $(filter true,$(LATEST)),--tag $(REGISTRY)/installer:latest) \
			$(PUSH_ARG) \
			--file Dockerfile \
			.,\
	$(if $(filter cli,$(1)),\
		@printf "$(CYAN)Building muakctl OCI$(RESET) (push=$(PUSH), latest=$(LATEST))\n"
		@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) $(PULL_ARG) \
			--tag $(REGISTRY)/muakctl:$(TAG) \
			$(if $(filter true,$(LATEST)),--tag $(REGISTRY)/muakctl:latest) \
			$(PUSH_ARG) \
			--file pkgs/muakctl/Dockerfile \
			.,\
		@test -f pkgs/$(1)/Dockerfile || { printf "$(RED)$(BOLD)Error:$(RESET) pkgs/$(1)/Dockerfile not found\n"; exit 1; }
		@printf "$(CYAN)Building OCI:$(RESET) $(1) (push=$(PUSH), latest=$(LATEST))\n"
		@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) $(PULL_ARG) \
			--tag $(REGISTRY)/pkgs/$(1):$(TAG) \
			$(if $(filter true,$(LATEST)),--tag $(REGISTRY)/pkgs/$(1):latest) \
			$(PUSH_ARG) \
			--file pkgs/$(1)/Dockerfile \
			.)))
endef

.PHONY: oci
oci: ## Build OCI images (e.g. make oci granola kernel installer cli)
	$(call require-docker-for-push)
	@if [ -z "$(filter-out oci,$(MAKECMDGOALS))" ]; then \
		printf "$(RED)$(BOLD)Error:$(RESET) No packages specified. Usage: make oci <pkg1> [pkg2...]\n"; \
		printf "$(YELLOW)Special packages:$(RESET) kernel, installer, cli\n"; \
		exit 1; \
	fi
	$(foreach pkg,$(filter-out oci,$(MAKECMDGOALS)),$(call build-oci,$(pkg)))

## Kernel
.PHONY: kernel
ifeq ($(IS_PARAM_CMD),)
kernel: $(ARTIFACTS) ## Build kernel to local artifacts
	$(call require-pkg,kernel)
	@printf "$(CYAN)Building kernel locally$(RESET)\n"
	@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) $(SIGNING_ARGS) $(PULL_ARG) \
		--output type=local,dest=$(ARTIFACTS) \
		--file pkgs/kernel/Dockerfile \
		.
else
kernel: ; @:
endif

## Installer
.PHONY: installer
ifeq ($(IS_PARAM_CMD),)
installer: $(ARTIFACTS) ## Build installer with local binaries
	$(call require,$(ARTIFACTS)/vmlinuz,make kernel)
	@printf "$(CYAN)Building installer with local binaries$(RESET)\n"
	@$(BUILD) $(COMMON_ARGS) $(CI_ARGS) $(PULL_ARG) \
		--build-context pkg-granola=$(RELEASE_DIR) \
		--build-context pkg-modd=$(RELEASE_DIR) \
		--build-context pkg-networkd=$(RELEASE_DIR) \
		--build-context pkg-apid=$(RELEASE_DIR) \
		--build-context pkg-vmd=$(RELEASE_DIR) \
		--build-context pkg-init=$(RELEASE_DIR) \
		--build-context pkg-stub=$(UEFI_RELEASE_DIR) \
		--build-context pkg-kernel=$(ARTIFACTS) \
		--output type=local,dest=$(ARTIFACTS) \
		--file Dockerfile \
		.
	@printf "$(GREEN)Installer assets extracted to $(ARTIFACTS)/$(RESET)\n"
else
installer: ; @:
endif

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
	$(call require,$(ARTIFACTS)/vmlinuz,make installer)
	$(call require,$(ARTIFACTS)/initramfs.img,make extensions)
	@printf "$(CYAN)Building UKI for $(ARCH)$(RESET)\n"
	@tr -d '\n' < $(CMDLINE_FILE) > $(ARTIFACTS)/cmdline.txt
	@$(RELEASE_DIR)/yuki \
		--stub $(ARTIFACTS)/stub.efi \
		--linux $(ARTIFACTS)/vmlinuz \
		--initrd $(ARTIFACTS)/initramfs.img \
		--cmdline $(ARTIFACTS)/cmdline.txt \
		$(if $(DTB),--dtb $(DTB)) \
		--output $(ARTIFACTS)/muak-$(ARCH).efi
	@printf "$(GREEN)UKI built:$(RESET) $(ARTIFACTS)/muak-$(ARCH).efi\n"

.PHONY: iso
iso: $(ARTIFACTS) ## Build bootable ISO
	$(call require,$(ARTIFACTS)/muak-$(ARCH).efi,make uki)
	@printf "$(CYAN)Building ISO for $(ARCH)$(RESET)\n"
	@$(CONTAINER_RUNTIME) run --rm --network=host -v $(PWD)/$(ARTIFACTS):/out -e BOOT_FILE=$(BOOT_FILE) -e ARCH=$(ARCH) alpine:3.23 sh -c '\
		set -euo pipefail && \
		apk add --no-cache mtools dosfstools xorriso >/dev/null 2>&1 && \
		rm -rf /out/iso && mkdir -p /out/iso/EFI/BOOT && \
		cp /out/muak-$${ARCH}.efi /out/iso/EFI/BOOT/$${BOOT_FILE} && \
		EFI_SIZE=$$(stat -c%s /out/muak-$${ARCH}.efi) && \
		FAT_SIZE=$$(( (EFI_SIZE / 1024 / 1024) + 10 )) && \
		dd if=/dev/zero of=/out/iso/efiboot.img bs=1M count=$${FAT_SIZE} 2>/dev/null && \
		mkfs.vfat /out/iso/efiboot.img >/dev/null && \
		mmd -i /out/iso/efiboot.img ::/EFI ::/EFI/BOOT && \
		mcopy -i /out/iso/efiboot.img /out/muak-$${ARCH}.efi ::/EFI/BOOT/$${BOOT_FILE} && \
		xorriso -as mkisofs -o /out/muak-$${ARCH}.iso -e efiboot.img -no-emul-boot -V MUAK /out/iso && \
		rm -rf /out/iso'
	@printf "$(GREEN)ISO built:$(RESET) $(ARTIFACTS)/muak-$(ARCH).iso\n"

# Development
.PHONY: dev
dev: packages installer extensions uki iso ## Full local development build
	@printf "$(GREEN)$(BOLD)Build complete:$(RESET) $(ARTIFACTS)/muak-$(ARCH).iso\n"

# Testing
.PHONY: test
test: ## Run tests (e.g. make test yuki)
	@if [ -n "$(filter-out test,$(MAKECMDGOALS))" ]; then \
		printf "$(CYAN)Running tests for $(filter-out test,$(MAKECMDGOALS))$(RESET)\n"; \
		cargo nextest run $(foreach p,$(filter-out test,$(MAKECMDGOALS)),-p $(p)); \
	else \
		printf "$(CYAN)Running tests$(RESET)\n"; \
		cargo nextest run; \
	fi

.PHONY: coverage
coverage: ## Run tests with coverage (e.g. make coverage yuki)
	@if [ -n "$(filter-out coverage,$(MAKECMDGOALS))" ]; then \
		printf "$(CYAN)Running tests with coverage for $(filter-out coverage,$(MAKECMDGOALS))$(RESET)\n"; \
		cargo llvm-cov nextest $(foreach p,$(filter-out coverage,$(MAKECMDGOALS)),-p $(p)); \
	else \
		printf "$(CYAN)Running tests with coverage$(RESET)\n"; \
		cargo llvm-cov nextest; \
	fi

.PHONY: kspp
kspp: ## Check kernel config against KSPP security hardening recommendations
	@printf "$(CYAN)Checking kernel config ($(KERNEL_CONFIG)) against KSPP recommendations$(RESET)\n"
	@$(CONTAINER_RUNTIME) run --rm --network=host -v $(PWD)/pkgs/kernel/$(KERNEL_CONFIG):/config:ro \
		alpine:3.23 sh -c '\
		apk add --no-cache git python3 >/dev/null 2>&1 && \
		git clone --depth 1 --quiet https://github.com/a13xp0p0v/kernel-hardening-checker.git /tmp/khc && \
		/tmp/khc/bin/kernel-hardening-checker -c /config'

# Cleanup
.PHONY: clean
clean: ## Remove all build artifacts
	@printf "$(CYAN)Cleaning build artifacts$(RESET)\n"
	@cargo clean
	@rm -rf $(ARTIFACTS)
	@printf "$(GREEN)Clean complete$(RESET)\n"
