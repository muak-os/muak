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

registry := env_var_or_default("REGISTRY", "ghcr.io/muak-os")
push := env_var_or_default("PUSH", "false")
latest := env_var_or_default("LATEST", "false")
ci_args := env_var_or_default("CI_ARGS", "")
kernel_signing := env_var_or_default("KERNEL_SIGNING", "")
signature:= env_var_or_default("SIGNATURE", "signature.key")
extensions := env_var_or_default("EXTENSIONS", "")
artifacts := `test -f .git && realpath -m "$(git rev-parse --git-common-dir)/../_out" || realpath -m _out`

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
tag := env_var_or_default("TAG", "latest")
provenance_arg := if container_runtime == "podman" { "" } else { "--provenance=false" }
common_args := "--platform=" + platform + " --progress=" + progress + " --build-arg SOURCE_DATE_EPOCH=" + source_date_epoch +  " " + provenance_arg

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

# Full local development build (build → installer → sign → extract → extensions → uki → iso)
dev: (build "--release" "") installer sign extract extensions uki iso
    @printf "{{ green }}{{ bold }}Build complete:{{ reset }} {{ artifacts }}/muak-{{ arch }}.iso\n"

# Build kernel image (use `just extract --image ...` to extract artifacts locally)
kernel:
  @printf "{{ cyan }}Building kernel{{ reset }}\n"
  {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ kernel_signing }} {{ pull_arg }} \
      --tag {{ registry }}/kernel:{{ tag }} \
      {{ push_arg }} \
      --file core/kernel/Dockerfile \
      .

# Build Rust packages with cargo (e.g., just build, just build --release, just build granola, just build --release granola)
[arg("release", long, value="--release")]
[script]
build release="" *pkgs:
    printf "{{ cyan }}Building Rust packages{{ reset }}\n"
    if [ -n "{{ pkgs }}" ]; then
        for pkg in {{ pkgs }}; do
            if [ "$pkg" = "stub" ]; then
                cargo +nightly build {{ release }} --target {{ arch }}-unknown-uefi --features uefi -p stub
            else
                cargo build {{ release }} --target {{ arch }}-unknown-linux-musl -p "$pkg"
            fi
        done
    else
        cargo build {{ release }} --target {{ arch }}-unknown-linux-musl
        cargo +nightly build {{ release }} --target {{ arch }}-unknown-uefi --features uefi -p stub
    fi

# Build installer image (default uses local binaries, --prod pulls from registry)
[arg("prod", long="prod", value="true")]
[script]
installer prod="false":
    printf "{{ cyan }}Building installer{{ reset }}\n"
    if [ "{{ prod }}" = "false" ]; then
        test -f {{ artifacts }}/vmlinuz || { printf "{{ red }}{{ bold }}Error:{{ reset }} {{ artifacts }}/vmlinuz not found. Run {{ green }}just kernel{{ reset }} first\n"; exit 1; }
        pkg_args=(
            --build-context pkg-granola={{ release_dir }}
            --build-context pkg-provisiond={{ release_dir }}
            --build-context pkg-modd={{ release_dir }}
            --build-context pkg-networkd={{ release_dir }}
            --build-context pkg-apid={{ release_dir }}
            --build-context pkg-vmd={{ release_dir }}
            --build-context pkg-timed={{ release_dir }}
            --build-context pkg-consoled={{ release_dir }}
            --build-context pkg-init={{ release_dir }}
            --build-context pkg-stub=target/{{ arch }}-unknown-uefi/release
            --build-context pkg-kernel={{ artifacts }}
        )
    else
        pkg_args=(
            --build-arg pkg-kernel={{ registry }}/kernel:{{ tag }}
            --build-arg pkg-granola={{ registry }}/pkgs/granola:{{ tag }}
            --build-arg pkg-provisiond={{ registry }}/pkgs/provisiond:{{ tag }}
            --build-arg pkg-modd={{ registry }}/pkgs/modd:{{ tag }}
            --build-arg pkg-networkd={{ registry }}/pkgs/networkd:{{ tag }}
            --build-arg pkg-apid={{ registry }}/pkgs/apid:{{ tag }}
            --build-arg pkg-vmd={{ registry }}/pkgs/vmd:{{ tag }}
            --build-arg pkg-timed={{ registry }}/pkgs/timed:{{ tag }}
            --build-arg pkg-consoled={{ registry }}/pkgs/consoled:{{ tag }}
            --build-arg pkg-init={{ registry }}/pkgs/init:{{ tag }}
            --build-arg pkg-stub={{ registry }}/pkgs/stub:{{ tag }}
        )
    fi
    {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} {{ push_arg }} \
        --build-context services=. \
        "${pkg_args[@]}" \
        --tag {{ registry }}/installer:{{ tag }} \
        --file Dockerfile \
        .
    if [ "{{ push }}" = "true" ] && [ "{{ container_runtime }}" = "podman" ]; then
        podman push {{ registry }}/installer:{{ tag }} --tls-verify=false
    fi
    printf "{{ green }}Installer image built: {{ registry }}/installer:{{ tag }}{{ reset }}\n"

# Extract an OCI image filesystem to local artifacts (default to installer image)
[arg("image", long="image")]
[script]
extract image=(registry + "/installer:" + tag): _ensure-artifacts
    printf "{{ cyan }}Extracting assets from {{ image }}{{ reset }}\n"
    cid=$({{ container_runtime }} create "{{ image }}")
    {{ container_runtime }} export "$cid" | tar -x -C {{ artifacts }}
    {{ container_runtime }} rm "$cid" >/dev/null
    printf "{{ green }}Assets extracted to {{ artifacts }}/{{ reset }}\n"

# Sign an OCI image in the registry (default to installer image)
[arg("image", long="image")]
sign image=(registry + "/installer:" + tag): (build "--release" "imager")
    {{ release_dir }}/imager sign \
        --image "{{ image }}" \
        --key "{{ signature }}"

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
        {{ release_dir }}/ramune build \
            --base {{ artifacts }}/base-initramfs.img \
            $ext_args \
            --output {{ artifacts }}/initramfs.img
    fi
    printf "{{ green }}Initramfs ready:{{ reset }} {{ artifacts }}/initramfs.img\n"

# Build UKI (Unified Kernel Image)
[script]
uki: _ensure-artifacts (_require artifacts / "stub.efi" "just installer") (_require artifacts / "vmlinuz" "just kernel") (_require artifacts / "initramfs.img" "just extensions")
    printf "{{ cyan }}Building UKI for {{ arch }}{{ reset }}\n"
    { tr -d '\n' < core/kernel/cmdline-{{ if arch == "aarch64" { "arm64" } else { "amd64" } }}.txt; printf ' muak.mode=live'; } > {{ artifacts }}/cmdline.txt
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
    {{ container_runtime }} run --rm --network=host -v {{ artifacts }}:/out \
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
oci *pkgs:
    pkgs="{{ pkgs }}"
    if [ -z "$pkgs" ]; then
        printf "{{ red }}{{ bold }}Error:{{ reset }} No packages specified. Usage: just oci <pkg1> [pkg2...]\n"
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
        cargo nextest run -E 'not package(e2e)'
    fi

# Run E2E tests suite (requires: qemu, built artifacts)
[script]
e2e: (build "--release" "muakctl")
    printf "{{ cyan }}Running E2E tests{{ reset }}\n"
    if [ ! -f "{{ artifacts }}/OVMF_VARS.fd" ] || [ ! -f "{{ artifacts }}/OVMF_CODE.secboot.fd" ]; then
        printf "{{ cyan }}Fetching OVMF firmware files{{ reset }}\n"
        {{ container_runtime }} run --rm --network=host -v {{ artifacts }}:/out alpine:3.23 sh -c '
        set -euo pipefail
        apk add --no-cache wget libarchive-tools >/dev/null 2>&1
        wget -q -O /tmp/edk2.rpm https://kojipkgs.fedoraproject.org/packages/edk2/20251119/10.fc44/noarch/edk2-ovmf-20251119-10.fc44.noarch.rpm
        bsdtar -xf /tmp/edk2.rpm -C /tmp
        cp /tmp/usr/share/edk2/ovmf/OVMF_VARS.fd /out/OVMF_VARS.fd
        cp /tmp/usr/share/edk2/ovmf/OVMF_CODE.secboot.fd /out/OVMF_CODE.secboot.fd'
        printf "{{ green }}OVMF firmware files ready{{ reset }}\n"
    fi
    MUAK_ARTIFACTS={{ artifacts }} MUAK_CLI=$(realpath "{{ release_dir }}/muakctl") cargo nextest run -E 'package(e2e)' --test-threads 3

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
        cargo llvm-cov nextest -E 'not package(e2e)'
    fi

# Check kernel config against KSPP security hardening recommendations
[script]
kspp:
    config="{{ if arch == "aarch64" { "config-arm64" } else { "config-amd64" } }}"
    printf "{{ cyan }}Checking kernel config ($config) against KSPP recommendations{{ reset }}\n"
    {{ container_runtime }} run --rm --network=host \
        -v {{ justfile_directory() }}/core/kernel/$config:/config:ro \
        alpine:3.23 sh -c '\
        apk add --no-cache git python3 >/dev/null 2>&1 && \
        git clone --depth 1 --quiet https://github.com/a13xp0p0v/kernel-hardening-checker.git /tmp/khc && \
        /tmp/khc/bin/kernel-hardening-checker -c /config'

# ─────────────────────────────────────────────────────────────────────────────
# Utilities
# ─────────────────────────────────────────────────────────────────────────────

# Validate SELinux CIL policy
[script]
policy:
    printf "{{ cyan }}Checking SELinux policy{{ reset }}\n"
    {{ container_runtime }} run --rm \
        -v {{ justfile_directory() }}/policy:/policy:ro \
        -v {{ justfile_directory() }}/services:/services:ro \
        -v {{ justfile_directory() }}/core:/core:ro \
        docker.io/debian:trixie-slim sh -c '
    set -euo pipefail
    apt-get update -qq && apt-get install -y -qq --no-install-recommends secilc >/dev/null 2>&1
    cd /tmp
    find /policy /services /core -name "*.cil" -exec cp {} . \;
    secilc -c 34 -o /dev/null -f /dev/null \
        $(find . -name "*.cil" | LC_ALL=c sort)
    echo "Policy OK"'
    printf "{{ green }}SELinux policy is valid{{ reset }}\n"

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
[script]
_require-pkg pkg:
    for dir in core services tools pkgs; do
        if [ -f "$dir/{{ pkg }}/Dockerfile" ]; then exit 0; fi
    done
    printf "{{ red }}{{ bold }}Error:{{ reset }} Dockerfile for {{ pkg }} not found in core/, services/, tools/, or pkgs/\n"; exit 1

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
            just kernel
            ;;
        installer)
            printf "{{ cyan }}Building installer OCI{{ reset }} (push={{ push }}, latest={{ latest }})\n"
            just installer --prod
            ;;
        cli)
            printf "{{ cyan }}Building muakctl OCI{{ reset }} (push={{ push }}, latest={{ latest }})\n"
            {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} \
                --tag {{ registry }}/muakctl:{{ tag }} \
                $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/muakctl:latest" || echo "") \
                {{ push_arg }} \
                --file cli/Dockerfile \
                .
            ;;
        *)
            dockerfile=""
            for dir in core services tools pkgs; do
                if [ -f "$dir/{{ pkg }}/Dockerfile" ]; then
                    dockerfile="$dir/{{ pkg }}/Dockerfile"
                    break
                fi
            done
            if [ -z "$dockerfile" ]; then
                printf "{{ red }}{{ bold }}Error:{{ reset }} Dockerfile for {{ pkg }} not found in core/, services/, tools/, or pkgs/\n"
                exit 1
            fi
            printf "{{ cyan }}Building OCI:{{ reset }} {{ pkg }} (push={{ push }}, latest={{ latest }})\n"
            {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} \
                --tag {{ registry }}/pkgs/{{ pkg }}:{{ tag }} \
                $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/pkgs/{{ pkg }}:latest" || echo "") \
                {{ push_arg }} \
                --file "$dockerfile" \
                .
            if [ "{{ push }}" = "true" ] && [ "{{ container_runtime }}" = "podman" ]; then
                podman push {{ registry }}/pkgs/{{ pkg }}:{{ tag }} --tls-verify=false
                if [ "{{ latest }}" = "true" ]; then
                    podman push {{ registry }}/pkgs/{{ pkg }}:latest --tls-verify=false
                fi
            fi
            ;;
    esac

[private]
[script]
_local-build pkg:
    dockerfile=""
    for dir in core services tools pkgs; do
        if [ -f "$dir/{{ pkg }}/Dockerfile" ]; then
            dockerfile="$dir/{{ pkg }}/Dockerfile"
            break
        fi
    done
    if [ -z "$dockerfile" ]; then
        printf "{{ red }}{{ bold }}Error:{{ reset }} Dockerfile for {{ pkg }} not found in core/, services/, tools/, or pkgs/\n"
        exit 1
    fi
    printf "{{ cyan }}Building local:{{ reset }} {{ pkg }} -> {{ artifacts }}/oci/{{ pkg }}\n"
    mkdir -p {{ artifacts }}/oci
    {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} \
        --tag {{ registry }}/pkgs/{{ pkg }}:{{ tag }} \
        --load \
        --file "$dockerfile" \
        .
    {{ container_runtime }} save --format oci-dir -o {{ artifacts }}/oci/{{ pkg }} {{ registry }}/pkgs/{{ pkg }}:{{ tag }}
    {{ container_runtime }} rmi {{ registry }}/pkgs/{{ pkg }}:{{ tag }} >/dev/null 2>&1 || true
