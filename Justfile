# Muak - A minimal, immutable, API-driven Linux distribution for running VMs
#
# Prerequisites: rustup with musl targets, docker/podman, git
# Run `just --list` for available recipes

set positional-arguments := true
set shell := ["bash", "-euo", "pipefail", "-c"]
set script-interpreter := ["bash", "-euo", "pipefail"]

# ─────────────────────────────────────────────────────────────────────────────
# Configuration
# ─────────────────────────────────────────────────────────────────────────────

# Global settings

registry := env_var_or_default("REGISTRY", "ghcr.io/sawangg")
push := env_var_or_default("PUSH", "false")
latest := env_var_or_default("LATEST", "false")
ci_args := env_var_or_default("CI_ARGS", "")
signing_args := env_var_or_default("SIGNING_ARGS", "")
extensions := env_var_or_default("EXTENSIONS", "")
artifacts := "_out"

# Architecture - override with ARCH=aarch64 for arm build

arch := env_var_or_default("ARCH", "x86_64")
release_dir := "target" / (arch + "-unknown-linux-musl") / "release"
debug_dir := "target" / (arch + "-unknown-linux-musl") / "debug"

# Container runtime

container_runtime := if `command -v docker >/dev/null 2>&1 && echo docker || echo podman` == "docker" { "docker" } else { "podman" }
build_cmd := if container_runtime == "podman" { "podman build" } else { "docker buildx build" }
pull_arg := if container_runtime == "podman" { "--pull=never" } else { "" }
push_arg := if container_runtime == "podman" { "" } else { "--push=" + push }
platform := if arch == "aarch64" { "linux/arm64" } else { "linux/amd64" }
progress := env_var_or_default("PROGRESS", "auto")
source_date_epoch := `git log -1 --pretty=%ct`
tag := env_var_or_default("TAG", `git describe --tag --always --dirty --match 'v[0-9]*' 2>/dev/null || echo dev`)
provenance_arg := if container_runtime == "podman" { "" } else { "--provenance=false" }
common_args := "--platform=" + platform + " --progress=" + progress + " --build-arg SOURCE_DATE_EPOCH=" + source_date_epoch + " --build-arg TAG=" + tag + " " + provenance_arg

# Colors

bold := '\e[1m'
cyan := '\e[36m'
green := '\e[32m'
yellow := '\e[33m'
red := '\e[31m'
reset := '\e[0m'

# ─────────────────────────────────────────────────────────────────────────────
# Main Recipes
# ─────────────────────────────────────────────────────────────────────────────

# Full local development build (packages → installer → extensions → uki → iso)
dev: build-release installer extensions uki iso
    @printf "{{ green }}{{ bold }}Build complete:{{ reset }} {{ artifacts }}/muak-{{ arch }}.iso\n"

# Build kernel to local artifacts
kernel: _ensure-artifacts (_require-pkg "kernel")
    @printf "{{ cyan }}Building kernel locally{{ reset }}\n"
    {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ signing_args }} {{ pull_arg }} \
        --output type=local,dest={{ artifacts }} \
        --file pkgs/kernel/Dockerfile \
        .

# Build all Rust packages with cargo (debug)
build:
    @printf "{{ cyan }}Building Rust packages (debug){{ reset }}\n"
    cargo build --target {{ arch }}-unknown-linux-musl
    cargo +nightly build --target {{ arch }}-unknown-uefi --features uefi -p stub

# Build all Rust packages with cargo (release)
build-release:
    @printf "{{ cyan }}Building Rust packages (release){{ reset }}\n"
    cargo build --release --target {{ arch }}-unknown-linux-musl
    cargo +nightly build --release --target {{ arch }}-unknown-uefi --features uefi -p stub

# Build installer with local binaries
installer: _ensure-artifacts (_require artifacts / "vmlinuz" "just kernel")
    @printf "{{ cyan }}Building installer with local binaries{{ reset }}\n"
    {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} \
        --build-context pkg-granola={{ release_dir }} \
        --build-context pkg-provisiond={{ release_dir }} \
        --build-context pkg-modd={{ release_dir }} \
        --build-context pkg-networkd={{ release_dir }} \
        --build-context pkg-apid={{ release_dir }} \
        --build-context pkg-vmd={{ release_dir }} \
        --build-context pkg-init={{ release_dir }} \
        --build-context pkg-stub=target/{{ arch }}-unknown-uefi/release \
        --build-context pkg-kernel={{ artifacts }} \
        --output type=local,dest={{ artifacts }} \
        --file Dockerfile \
        .
    @printf "{{ green }}Installer assets extracted to {{ artifacts }}/{{ reset }}\n"

# Extend base initramfs with specified extensions (set EXTENSIONS="ext1 ext2")
[script]
extensions: _ensure-artifacts (_require artifacts / "base-initramfs.img" "just installer")
    if [ -z "{{ extensions }}" ]; then
        printf "{{ yellow }}No extensions specified, using base initramfs{{ reset }}\n"
        cp {{ artifacts }}/base-initramfs.img {{ artifacts }}/initramfs.img
    else
        printf "{{ cyan }}Building initramfs with extensions:{{ reset }} {{ extensions }}\n"
        ext_args=""
        for ext in {{ extensions }}; do
            ext_args="$ext_args --extension $ext"
        done
        {{ release_dir }}/imager build \
            --base {{ artifacts }}/base-initramfs.img \
            $ext_args \
            --output {{ artifacts }}/initramfs.img
    fi
    printf "{{ green }}Initramfs ready:{{ reset }} {{ artifacts }}/initramfs.img\n"

# Build UKI (Unified Kernel Image)
[script]
uki: _ensure-artifacts (_require artifacts / "stub.efi" "just installer") (_require artifacts / "vmlinuz" "just installer") (_require artifacts / "initramfs.img" "just extensions")
    printf "{{ cyan }}Building UKI for {{ arch }}{{ reset }}\n"
    { tr -d '\n' < pkgs/kernel/cmdline-{{ if arch == "aarch64" { "arm64" } else { "amd64" } }}.txt; printf ' muak.mode=live'; } > {{ artifacts }}/cmdline.txt
    {{ release_dir }}/yuki \
        --stub {{ artifacts }}/stub.efi \
        --linux {{ artifacts }}/vmlinuz \
        --initrd {{ artifacts }}/initramfs.img \
        --cmdline {{ artifacts }}/cmdline.txt \
        ${DTB:+--dtb "$DTB"} \
        --output {{ artifacts }}/muak-{{ arch }}.efi
    printf "{{ green }}UKI built:{{ reset }} {{ artifacts }}/muak-{{ arch }}.efi\n"

# Build bootable ISO
[script]
iso: _ensure-artifacts (_require artifacts / "muak-" + arch + ".efi" "just uki")
    printf "{{ cyan }}Building ISO for {{ arch }}{{ reset }}\n"
    {{ container_runtime }} run --rm --network=host -v {{ justfile_directory() }}/{{ artifacts }}:/out \
        -e BOOT_FILE={{ if arch == "aarch64" { "BOOTAA64.EFI" } else { "BOOTX64.EFI" } }} -e ARCH={{ arch }} alpine:3.23 sh -c '
        set -euo pipefail
        apk add --no-cache mtools dosfstools xorriso >/dev/null 2>&1
        rm -rf /out/iso && mkdir -p /out/iso/EFI/BOOT
        cp /out/muak-${ARCH}.efi /out/iso/EFI/BOOT/${BOOT_FILE}
        EFI_SIZE=$(stat -c%s /out/muak-${ARCH}.efi)
        FAT_SIZE=$(( (EFI_SIZE / 1024 / 1024) + 10 ))
        dd if=/dev/zero of=/out/iso/efiboot.img bs=1M count=${FAT_SIZE} 2>/dev/null
        mkfs.vfat /out/iso/efiboot.img >/dev/null
        mmd -i /out/iso/efiboot.img ::/EFI ::/EFI/BOOT
        mcopy -i /out/iso/efiboot.img /out/muak-${ARCH}.efi ::/EFI/BOOT/${BOOT_FILE}
        xorriso -as mkisofs -o /out/muak-${ARCH}.iso \
            -e efiboot.img -no-emul-boot \
            -append_partition 2 0xEF /out/iso/efiboot.img \
            -appended_part_as_gpt \
            -V MUAK /out/iso
        rm -rf /out/iso'
    printf "{{ green }}ISO built:{{ reset }} {{ artifacts }}/muak-{{ arch }}.iso\n"

# Build OCI images (e.g., just oci granola kernel installer cli)
[script]
oci *pkgs: _require-docker-for-push
    pkgs="{{ pkgs }}"
    if [ -z "$pkgs" ]; then
        printf "{{ red }}{{ bold }}Error:{{ reset }} No packages specified. Usage: just oci <pkg1> [pkg2...]\n"
        printf "{{ yellow }}Special packages:{{ reset }} kernel, installer, cli\n"
        exit 1
    fi
    for pkg in $pkgs; do
        just _oci-build "$pkg"
    done

# Build packages as local OCI layout (e.g., just local granola modd)
[script]
local *pkgs: _ensure-artifacts
    pkgs="{{ pkgs }}"
    if [ -z "$pkgs" ]; then
        printf "{{ red }}{{ bold }}Error:{{ reset }} No packages specified. Usage: just local <pkg1> [pkg2...]\n"
        exit 1
    fi
    for pkg in $pkgs; do
        just _local-build "$pkg"
    done

# ─────────────────────────────────────────────────────────────────────────────
# Testing
# ─────────────────────────────────────────────────────────────────────────────

# Run tests (e.g., just test or just test yuki imager)
[script]
test *pkgs:
    if [ -n "{{ pkgs }}" ]; then
        printf "{{ cyan }}Running tests for {{ pkgs }}{{ reset }}\n"
        pkg_args=""
        for pkg in {{ pkgs }}; do
            pkg_args="$pkg_args -p $pkg"
        done
        cargo nextest run $pkg_args
    else
        cargo nextest run
    fi

# Run tests with coverage (e.g., just coverage or just coverage yuki)
[script]
coverage *pkgs:
    if [ -n "{{ pkgs }}" ]; then
        printf "{{ cyan }}Running tests with coverage for {{ pkgs }}{{ reset }}\n"
        pkg_args=""
        for pkg in {{ pkgs }}; do
            pkg_args="$pkg_args -p $pkg"
        done
        cargo llvm-cov nextest $pkg_args
    else
        cargo llvm-cov nextest
    fi

# Check kernel config against KSPP security hardening recommendations
[script]
kspp:
    config="{{ if arch == "aarch64" { "config-arm64" } else { "config-amd64" } }}"
    printf "{{ cyan }}Checking kernel config ($config) against KSPP recommendations{{ reset }}\n"
    {{ container_runtime }} run --rm --network=host \
        -v {{ justfile_directory() }}/pkgs/kernel/$config:/config:ro \
        alpine:3.23 sh -c '\
        apk add --no-cache git python3 >/dev/null 2>&1 && \
        git clone --depth 1 --quiet https://github.com/a13xp0p0v/kernel-hardening-checker.git /tmp/khc && \
        /tmp/khc/bin/kernel-hardening-checker -c /config'

# ─────────────────────────────────────────────────────────────────────────────
# Utilities
# ─────────────────────────────────────────────────────────────────────────────

# Remove all build artifacts
clean:
    @printf "{{ cyan }}Cleaning build artifacts{{ reset }}\n"
    cargo clean
    rm -rf {{ artifacts }}
    @printf "{{ green }}Clean complete{{ reset }}\n"

# ─────────────────────────────────────────────────────────────────────────────
# Private Helpers
# ─────────────────────────────────────────────────────────────────────────────

[private]
_ensure-artifacts:
    @mkdir -p {{ artifacts }}

[private]
_require file hint:
    @test -f {{ file }} || { printf "{{ red }}{{ bold }}Error:{{ reset }} {{ file }} not found. Run {{ green }}{{ hint }}{{ reset }} first\n"; exit 1; }

[private]
_require-pkg pkg:
    @test -f pkgs/{{ pkg }}/Dockerfile || { printf "{{ red }}{{ bold }}Error:{{ reset }} pkgs/{{ pkg }}/Dockerfile not found\n"; exit 1; }

[private]
[script]
_require-docker-for-push:
    if [ "{{ push }}" = "true" ] && [ "{{ container_runtime }}" = "podman" ]; then
        printf "{{ red }}{{ bold }}Error:{{ reset }} PUSH=true requires Docker (podman does not support --push)\n"
        printf "{{ yellow }}Hint:{{ reset }} Set CONTAINER_RUNTIME=docker\n"
        exit 1
    fi

[private]
[script]
_oci-build pkg:
    latest_tag=""
    if [ "{{ latest }}" = "true" ]; then
        latest_tag="--tag {{ registry }}/{{ pkg }}:latest"
    fi

    case "{{ pkg }}" in
        kernel)
            printf "{{ cyan }}Building kernel OCI{{ reset }} (push={{ push }}, latest={{ latest }})\n"
            {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ signing_args }} {{ pull_arg }} \
                --tag {{ registry }}/kernel:{{ tag }} \
                $latest_tag \
                {{ push_arg }} \
                --target kernel-package \
                --file pkgs/kernel/Dockerfile \
                .
            ;;
        installer)
            printf "{{ cyan }}Building installer OCI{{ reset }} (push={{ push }}, latest={{ latest }})\n"
            {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} \
                --build-arg PKG_KERNEL={{ registry }}/kernel:{{ tag }} \
                --build-arg PKG_GRANOLA={{ registry }}/pkgs/granola:{{ tag }} \
                --build-arg PKG_PROVISIOND={{ registry }}/pkgs/provisiond:{{ tag }} \
                --build-arg PKG_MODD={{ registry }}/pkgs/modd:{{ tag }} \
                --build-arg PKG_NETWORKD={{ registry }}/pkgs/networkd:{{ tag }} \
                --build-arg PKG_APID={{ registry }}/pkgs/apid:{{ tag }} \
                --build-arg PKG_VMD={{ registry }}/pkgs/vmd:{{ tag }} \
                --build-arg PKG_INIT={{ registry }}/pkgs/init:{{ tag }} \
                --build-arg PKG_STUB={{ registry }}/pkgs/stub:{{ tag }} \
                --tag {{ registry }}/installer:{{ tag }} \
                $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/installer:latest" || echo "") \
                {{ push_arg }} \
                --file Dockerfile \
                .
            ;;
        cli)
            printf "{{ cyan }}Building muakctl OCI{{ reset }} (push={{ push }}, latest={{ latest }})\n"
            {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} \
                --tag {{ registry }}/muakctl:{{ tag }} \
                $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/muakctl:latest" || echo "") \
                {{ push_arg }} \
                --file pkgs/muakctl/Dockerfile \
                .
            ;;
        *)
            test -f pkgs/{{ pkg }}/Dockerfile || { printf "{{ red }}{{ bold }}Error:{{ reset }} pkgs/{{ pkg }}/Dockerfile not found\n"; exit 1; }
            printf "{{ cyan }}Building OCI:{{ reset }} {{ pkg }} (push={{ push }}, latest={{ latest }})\n"
            {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} \
                --tag {{ registry }}/pkgs/{{ pkg }}:{{ tag }} \
                $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/pkgs/{{ pkg }}:latest" || echo "") \
                {{ push_arg }} \
                --file pkgs/{{ pkg }}/Dockerfile \
                .
            ;;
    esac

[private]
[script]
_local-build pkg:
    test -f pkgs/{{ pkg }}/Dockerfile || { printf "{{ red }}{{ bold }}Error:{{ reset }} pkgs/{{ pkg }}/Dockerfile not found\n"; exit 1; }
    printf "{{ cyan }}Building local:{{ reset }} {{ pkg }} -> {{ artifacts }}/oci/{{ pkg }}\n"
    mkdir -p {{ artifacts }}/oci
    {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} \
        --tag localhost/muak-{{ pkg }}:{{ tag }} \
        --load \
        --file pkgs/{{ pkg }}/Dockerfile \
        .
    {{ container_runtime }} save --format oci-dir -o {{ artifacts }}/oci/{{ pkg }} localhost/muak-{{ pkg }}:{{ tag }}
    {{ container_runtime }} rmi localhost/muak-{{ pkg }}:{{ tag }} >/dev/null 2>&1 || true
