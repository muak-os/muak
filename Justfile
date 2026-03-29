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
tools := env_var_or_default("TOOLS", "ghcr.io/muak-os/tools:latest")
push := env_var_or_default("PUSH", "false")
latest := env_var_or_default("LATEST", "false")
ci_args := env_var_or_default("CI_ARGS", "")
kernel_signing := env_var_or_default("KERNEL_SIGNING", "")
signature:= env_var_or_default("SIGNATURE", "signature.key")
artifacts := `test -f .git && realpath -m "$(git rev-parse --git-common-dir)/../_out" || realpath -m _out`

# Architecture - override with ARCH=aarch64 for arm build

arch := env_var_or_default("ARCH", "x86_64")
container_arch := if arch == "aarch64" { "arm64" } else { "amd64" }
release_dir := "target" / (arch + "-unknown-linux-musl") / "release"

# Container runtime

container_runtime := `command -v docker >/dev/null 2>&1 && echo docker || echo podman`
build_cmd := if container_runtime == "podman" { "podman build" } else { "docker buildx build" }
pull_arg := if container_runtime == "podman" { "--pull=never" } else { "" }
push_arg := if container_runtime == "podman" { "" } else { "--push=" + push }
platform := "linux/" + container_arch
progress := env_var_or_default("PROGRESS", "auto")
source_date_epoch := `git log -1 --pretty=%ct`
tag := env_var_or_default("TAG", "latest")
provenance_arg := if container_runtime == "podman" { "" } else { "--provenance=false" }
common_args := "--platform=" + platform + " --progress=" + progress + " --build-arg SOURCE_DATE_EPOCH=" + source_date_epoch + " --build-arg RUST_VERSION=" + rust_version + " --build-arg ALPINE_VERSION=" + alpine_version + " " + provenance_arg

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
        /imager sign \
            --image "{{ image }}" \
            --key /key

# Build UKI (Unified Kernel Image)
[script]
uki: _ensure-artifacts (_require artifacts / "stub.efi" "just installer") (_require artifacts / "vmlinuz" "just kernel") (_require artifacts / "initramfs.img" "just installer")
    printf "{{ cyan }}Building UKI for {{ arch }}{{ reset }}\n"
    { tr -d '\n' < core/kernel/cmdline-{{ container_arch }}.txt; printf ' muak.mode=live'; } > {{ artifacts }}/cmdline.txt
    {{ container_runtime }} run --rm \
        -v "{{ artifacts }}:/out" \
        {{ tools }} \
        /yuki \
            --stub /out/stub.efi \
            --linux /out/vmlinuz \
            --initrd /out/initramfs.img \
            --cmdline /out/cmdline.txt \
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
            --label MUAK \
            --arch {{ arch }}
    @printf "{{ green }}ISO built:{{ reset }} {{ artifacts }}/muak-{{ arch }}.iso\n"

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

# Run tests (e.g., just test or just test yuki imager)
[script]
test *pkgs:
    just _test-run "cargo nextest run" "Running tests for" {{ pkgs }}

# Run E2E tests suite (requires: qemu, built artifacts)
[script]
e2e: (build "--release" "muakctl")
    printf "{{ cyan }}Running E2E tests{{ reset }}\n"
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
    MUAK_ARTIFACTS={{ artifacts }} MUAK_CLI=$(realpath "{{ release_dir }}/muakctl") cargo nextest run -E 'package(e2e)' --test-threads 3

# Run tests with coverage (e.g., just coverage or just coverage yuki)
[script]
coverage *pkgs:
    just _test-run "cargo llvm-cov nextest" "Running tests with coverage for" {{ pkgs }}

# Check kernel config against KSPP security hardening recommendations
[script]
kspp:
    config="config-{{ container_arch }}"
    printf "{{ cyan }}Checking kernel config ($config) against KSPP recommendations{{ reset }}\n"
    {{ container_runtime }} run --rm --network=host \
        -v {{ justfile_directory() }}/core/kernel/$config:/config:ro \
        docker.io/alpine:{{ alpine_version }} sh -c '\
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
            {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} \
                --tag {{ registry }}/muakctl:{{ tag }} \
                $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/muakctl:latest" || echo "") \
                {{ push_arg }} \
                --file cli/Dockerfile \
                .
            ;;
        tools)
            printf "{{ cyan }}Building tools OCI{{ reset }} (push={{ push }}, latest={{ latest }})\n"
            {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} \
                --tag {{ registry }}/tools:{{ tag }} \
                $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/tools:latest" || echo "") \
                {{ push_arg }} \
                --file tools/Dockerfile \
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
            just _podman-push "{{ registry }}/pkgs/{{ pkg }}:{{ tag }}"
            if [ "{{ latest }}" = "true" ]; then
                just _podman-push "{{ registry }}/pkgs/{{ pkg }}:latest"
            fi
            ;;
    esac
