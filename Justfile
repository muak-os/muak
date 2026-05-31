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

alpine_version := "3.23"
rust_version := `grep -oP 'channel\s*=\s*"\K[^"]+' rust-toolchain.toml`
registry := env_var_or_default("REGISTRY", "ghcr.io/muak-os")
tag := env_var_or_default("TAG", "latest")
tools := env_var_or_default("TOOLS", "ghcr.io/muak-os/tools:" + tag)
push := env_var_or_default("PUSH", "false")
latest := env_var_or_default("LATEST", "false")
ci_args := env_var_or_default("CI_ARGS", "")
kernel_signing := env_var_or_default("KERNEL_SIGNING", "")
signature := env_var_or_default("SIGNATURE", "signature.key")
artifacts := `test -f .git && realpath -m "$(git rev-parse --git-common-dir)/../_out" || realpath -m _out`

# Architecture

[private]
_arch_env := env_var_or_default("ARCH", "x86_64")
arch := if _arch_env == "amd64" { "x86_64" } else if _arch_env == "arm64" { "aarch64" } else { _arch_env }
oci_arch := if arch == "aarch64" { "arm64" } else if arch == "x86_64" { "amd64" } else { arch }
release_dir := "target" / (arch + "-unknown-linux-musl") / "release"

# Container runtime

container_runtime := env_var_or_default("CONTAINER_RUNTIME", "podman")
build_cmd := if container_runtime == "podman" { "podman build" } else { "docker buildx build" }
pull_arg := if container_runtime == "podman" { "--pull=missing" } else { "" }
push_arg := if container_runtime == "podman" { "" } else { "--push=" + push }
platform := "linux/" + oci_arch
progress := env_var_or_default("PROGRESS", "auto")
source_date_epoch := env_var_or_default("SOURCE_DATE_EPOCH", "0")
provenance_arg := if container_runtime == "podman" { "" } else { "--provenance=false" }
common_args := "--platform=" + platform + " --progress=" + progress + " --build-arg SOURCE_DATE_EPOCH=" + source_date_epoch + " --build-arg ALPINE_VERSION=" + alpine_version + " " + provenance_arg

# Colors

bold := '\e[1m'
cyan := '\e[36m'
green := '\e[32m'
red := '\e[31m'
reset := '\e[0m'

# ─────────────────────────────────────────────────────────────────────────────
# Main Recipes
# ─────────────────────────────────────────────────────────────────────────────

# Full local development build (build → installer → sign → extract → uki → iso)
dev: (build "--release" "") installer sign (extract (registry + "/installer:" + tag)) uki iso
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
[arg("release", long="release", value="--release")]
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
    pkgs="granola provisiond modd networkd apid vmd timed consoled init"
    if [ "{{ prod }}" = "false" ]; then
        test -f {{ artifacts }}/vmlinuz || { printf "{{ red }}{{ bold }}Error:{{ reset }} {{ artifacts }}/vmlinuz not found. Run {{ green }}just kernel{{ reset }} and extract\n"; exit 1; }
        pkg_args=(
            --build-context pkg-kernel={{ artifacts }}
            --build-context pkg-stub=target/{{ arch }}-unknown-uefi/release
        )
        for pkg in $pkgs; do
            pkg_args+=(--build-context "pkg-$pkg={{ release_dir }}")
        done
    else
        pkg_args=(
            --build-arg pkg-kernel={{ registry }}/kernel:{{ tag }}
            --build-arg pkg-stub={{ registry }}/pkgs/stub:{{ tag }}
        )
        for pkg in $pkgs; do
            pkg_args+=(--build-arg "pkg-$pkg={{ registry }}/pkgs/$pkg:{{ tag }}")
        done
    fi
    {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} {{ push_arg }} \
        --build-context services=. \
        --build-arg TOOLS={{ tools }} \
        "${pkg_args[@]}" \
        --tag {{ registry }}/installer:{{ tag }} \
        --file Dockerfile \
        .
    just _podman-push "{{ registry }}/installer:{{ tag }}"
    printf "{{ green }}Installer image built: {{ registry }}/installer:{{ tag }}{{ reset }}\n"

# Extract an OCI image filesystem to local artifacts
[arg("image", long="image")]
[script]
extract image: _ensure-artifacts
    printf "{{ cyan }}Extracting assets from {{ image }}{{ reset }}\n"
    cid=$({{ container_runtime }} create "{{ image }}")
    {{ container_runtime }} export "$cid" | tar -x -C {{ artifacts }}
    {{ container_runtime }} rm "$cid" >/dev/null
    printf "{{ green }}Assets extracted to {{ artifacts }}/{{ reset }}\n"

# Sign an OCI image in the registry (default to installer image)
[arg("image", long="image")]
sign image=(registry + "/installer:" + tag):
    {{ container_runtime }} run --rm --network=host \
        -v "{{ absolute_path(signature) }}:/key:ro" \
        {{ tools }} \
        /koci sign \
            --image "{{ image }}" \
            --key /key

# Build UKI (Unified Kernel Image)
[script]
uki: _ensure-artifacts (_require artifacts / "stub.efi" "just installer") (_require artifacts / "vmlinuz" "just kernel") (_require artifacts / "initramfs.img" "just installer")
    printf "{{ cyan }}Building UKI for {{ arch }}{{ reset }}\n"
    {{ container_runtime }} run --rm \
        -v "{{ artifacts }}:/out" \
        {{ tools }} \
        /yuki \
            --stub /out/stub.efi \
            --linux /out/vmlinuz \
            --initrd /out/initramfs.img \
            --cmdline /out/cmdline \
            ${DTB:+--dtb "$DTB"} \
            --output /out/muak-{{ arch }}.efi
    printf "{{ green }}UKI built:{{ reset }} {{ artifacts }}/muak-{{ arch }}.efi\n"

# Build bootable ISO
iso: _ensure-artifacts (_require artifacts / "muak-" + arch + ".efi" "just uki")
    @printf "{{ cyan }}Building ISO for {{ arch }}{{ reset }}\n"
    {{ container_runtime }} run --rm -v {{ artifacts }}:/out \
        {{ tools }} \
        /miso iso \
            --uki /out/muak-{{ arch }}.efi \
            --output /out/muak-{{ arch }}.iso \
            --arch {{ arch }}
    @printf "{{ green }}ISO built:{{ reset }} {{ artifacts }}/muak-{{ arch }}.iso\n"

# Build bootable raw disk image containing an EFI System Partition
raw: _ensure-artifacts (_require artifacts / "muak-" + arch + ".efi" "just uki")
    @printf "{{ cyan }}Building RAW for {{ arch }}{{ reset }}\n"
    {{ container_runtime }} run --rm -v {{ artifacts }}:/out \
        {{ tools }} \
        /miso raw \
            --uki /out/muak-{{ arch }}.efi \
            --output /out/muak-{{ arch }}.raw \
            --arch {{ arch }}
    @printf "{{ green }}RAW built:{{ reset }} {{ artifacts }}/muak-{{ arch }}.raw\n"

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

# ─────────────────────────────────────────────────────────────────────────────
# Testing
# ─────────────────────────────────────────────────────────────────────────────

# Run rustfmt
format:
    @printf "{{ cyan }}Running formatting{{ reset }}\n"
    cargo +nightly fmt

# Run clippy and rustfmt (e.g., just lint or just lint yuki koci)
[script]
lint *pkgs: format
    printf "{{ cyan }}Running lints{{ reset }}\n"
    if [ -n "{{ pkgs }}" ]; then
        for pkg in {{ pkgs }}; do
            if [ "$pkg" = "stub" ]; then
                cargo +nightly clippy --target {{ arch }}-unknown-uefi --features uefi -p stub
            else
                cargo clippy --target {{ arch }}-unknown-linux-musl -p "$pkg"
            fi
        done
    else
        cargo clippy --target {{ arch }}-unknown-linux-musl
        cargo +nightly clippy --target {{ arch }}-unknown-uefi --features uefi -p stub
    fi

# Run tests (e.g., just test or just test yuki koci)
[script]
test *pkgs:
    just _test-run "cargo nextest run" "Running tests for" {{ pkgs }}

# Run E2E tests suite (requires: qemu, built artifacts)
[script]
e2e: (build "--release" "muakctl") _ensure-fw
    printf "{{ cyan }}Running E2E tests{{ reset }}\n"
    MUAK_ARTIFACTS={{ artifacts }} MUAK_CLI=$(realpath "{{ release_dir }}/muakctl") cargo nextest run -E 'package(e2e)' --test-threads 3

# Boot the ISO in QEMU using user-mode networking and a persistent NVMe disk
[script]
start: (_require artifacts / "muak-" + arch + ".iso" "just dev") _ensure-fw
    if [ ! -f "/tmp/nvme-disk.img" ]; then
        printf "{{ cyan }}Creating persistent NVMe disk{{ reset }}\n"
        qemu-img create -f raw "/tmp/nvme-disk.img" 5G >/dev/null
    fi

    if [ ! -f "/tmp/OVMF_VARS.fd" ]; then
        printf "{{ cyan }}Creating persistent OVMF vars{{ reset }}\n"
        cp "{{ artifacts }}/OVMF_VARS.fd" "/tmp/OVMF_VARS.fd"
    fi

    printf "{{ cyan }}Starting QEMU{{ reset }}\n"
    printf "{{ green }}Guest install image:{{ reset }} 10.0.2.2:5000/installer:latest\n"
    printf "{{ green }}Guest API:{{ reset }} 127.0.0.1:50051\n"
    printf "{{ green }}Reset VM state:{{ reset }} rm -f /tmp/nvme-disk.img /tmp/OVMF_VARS.fd\n"

    qemu-system-x86_64 \
        -enable-kvm \
        -machine type=q35,accel=kvm \
        -cpu host \
        -m 2G \
        -smp 2 \
        -drive if=pflash,format=raw,readonly=on,file="{{ artifacts }}/OVMF_CODE.secboot.fd" \
        -drive if=pflash,format=raw,file="/tmp/OVMF_VARS.fd" \
        -serial stdio \
        -display none \
        -netdev user,id=net0,hostfwd=tcp:127.0.0.1:50051-:50051 \
        -device virtio-net-pci,netdev=net0 \
        -drive file="{{ artifacts }}/muak-{{ arch }}.iso",format=raw,media=cdrom,if=none,id=cdrom0,readonly=on \
        -device ide-cd,drive=cdrom0,bootindex=2 \
        -drive file="/tmp/nvme-disk.img",format=raw,if=none,id=nvme0 \
        -device nvme,serial=deadbeef,drive=nvme0,bootindex=1

# Run tests with coverage (e.g., just coverage, just coverage --missing, or just coverage yuki)
[arg("missing", long="missing", value="--show-missing-lines")]
[script]
coverage missing="" *pkgs:
    just _test-run "cargo llvm-cov nextest {{ missing }}" "Running tests with coverage for" {{ pkgs }}

# Check kernel config, cmdline & sysctl against KSPP security hardening recommendations
[script]
kspp:
    config="config-{{ oci_arch }}"
    cmdline="cmdline-{{ oci_arch }}.txt"
    sysctl="sysctl-{{ oci_arch }}.conf"
    printf "{{ cyan }}Checking kernel confgi, cmdline & sysctl against KSPP recommendations{{ reset }}\n"
    {{ container_runtime }} run --rm --network=host \
        -v {{ justfile_directory() }}/core/kernel/$config:/config:ro \
        -v {{ justfile_directory() }}/core/kernel/$cmdline:/cmdline:ro \
        -v {{ justfile_directory() }}/core/kernel/$sysctl:/sysctl:ro \
        docker.io/alpine:{{ alpine_version }} sh -c '\
        apk add --no-cache git python3 >/dev/null 2>&1 && \
        git clone --depth 1 --quiet https://github.com/a13xp0p0v/kernel-hardening-checker.git /tmp/khc && \
        /tmp/khc/bin/kernel-hardening-checker -c /config -l /cmdline -s /sysctl'

# ─────────────────────────────────────────────────────────────────────────────
# Utilities
# ─────────────────────────────────────────────────────────────────────────────

# Validate SELinux CIL policy
[script]
policy:
    printf "{{ cyan }}Checking SELinux policy{{ reset }}\n"
    cil_files=$(find {{ justfile_directory() }}/policy \
        {{ justfile_directory() }}/services \
        {{ justfile_directory() }}/core \
        -name "*.cil" | LC_ALL=c sort | sed 's|{{ justfile_directory() }}|/src|g')
    {{ container_runtime }} run --rm \
        -v {{ justfile_directory() }}:/src:ro \
        {{ tools }} \
        /secilc -o /dev/null -f /dev/null ${cil_files}
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
[script]
_ensure-fw: _ensure-artifacts
    if [ "{{ arch }}" != "x86_64" ]; then
        printf "{{ red }}{{ bold }}Error:{{ reset }} QEMU helpers currently support only x86_64\n"
        exit 1
    fi

    if [ ! -f "{{ artifacts }}/OVMF_VARS.fd" ] || [ ! -f "{{ artifacts }}/OVMF_CODE.secboot.fd" ]; then
        printf "{{ cyan }}Fetching OVMF firmware files{{ reset }}\n"
        {{ container_runtime }} run --rm --network=host -v {{ artifacts }}:/out docker.io/alpine:{{ alpine_version }} sh -c '
        set -euo pipefail
        apk add --no-cache wget libarchive-tools >/dev/null 2>&1
        wget -q -O /tmp/edk2.rpm https://kojipkgs.fedoraproject.org/packages/edk2/20251119/10.fc44/noarch/edk2-ovmf-20251119-10.fc44.noarch.rpm
        bsdtar -xf /tmp/edk2.rpm -C /tmp
        cp /tmp/usr/share/edk2/ovmf/OVMF_VARS.fd /out/OVMF_VARS.fd
        cp /tmp/usr/share/edk2/ovmf/OVMF_CODE.secboot.fd /out/OVMF_CODE.secboot.fd'
        printf "{{ green }}OVMF firmware files ready{{ reset }}\n"
    fi

[private]
_ensure-artifacts:
    @mkdir -p {{ artifacts }}

[private]
_require file hint:
    @test -f {{ file }} || { printf "{{ red }}{{ bold }}Error:{{ reset }} {{ file }} not found. Run {{ green }}{{ hint }}{{ reset }} first\n"; exit 1; }

[private]
[script]
_test-run runner label *pkgs:
    if [ -n "{{ pkgs }}" ]; then
        printf "{{ cyan }}{{ label }} {{ pkgs }}{{ reset }}\n"
        pkg_args=""
        for pkg in {{ pkgs }}; do
            pkg_args="$pkg_args -p $pkg"
        done
        {{ runner }} $pkg_args
    else
        {{ runner }} -E 'not package(e2e)'
    fi

[private]
[script]
_podman-push image:
    if [ "{{ push }}" = "true" ] && [ "{{ container_runtime }}" = "podman" ]; then
        podman push "{{ image }}" --tls-verify=false
    fi

[private]
[script]
_oci-build pkg:
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
            {{ build_cmd }} {{ common_args }} --build-arg RUST_VERSION={{ rust_version }} {{ ci_args }} {{ pull_arg }} \
                --tag {{ registry }}/muakctl:{{ tag }} \
                $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/muakctl:latest" || echo "") \
                {{ push_arg }} \
                --file cli/Dockerfile \
                .
            just _podman-push "{{ registry }}/muakctl:{{ tag }}"
            if [ "{{ latest }}" = "true" ]; then
                just _podman-push "{{ registry }}/muakctl:latest"
            fi
            ;;
        tools)
            printf "{{ cyan }}Building tools OCI{{ reset }} (push={{ push }}, latest={{ latest }})\n"
            {{ build_cmd }} {{ common_args }} --build-arg RUST_VERSION={{ rust_version }} {{ ci_args }} {{ pull_arg }} \
                --tag {{ registry }}/tools:{{ tag }} \
                $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/tools:latest" || echo "") \
                {{ push_arg }} \
                --file tools/Dockerfile \
                .
            just _podman-push "{{ registry }}/tools:{{ tag }}"
            if [ "{{ latest }}" = "true" ]; then
                just _podman-push "{{ registry }}/tools:latest"
            fi
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
            {{ build_cmd }} {{ common_args }} --build-arg RUST_VERSION={{ rust_version }} {{ ci_args }} {{ pull_arg }} \
                --tag {{ registry }}/pkgs/{{ pkg }}:{{ tag }} \
                $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/pkgs/{{ pkg }}:latest" || echo "") \
                {{ push_arg }} \
                --file "$dockerfile" \
                .
            just _podman-push "{{ registry }}/pkgs/{{ pkg }}:{{ tag }}"
            if [ "{{ latest }}" = "true" ]; then
                just _podman-push "{{ registry }}/pkgs/{{ pkg }}:latest"
            fi
            ;;
    esac
