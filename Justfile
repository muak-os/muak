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

alpine_version := "3.24"
rust_version := `grep -oP 'channel\s*=\s*"\K[^"]+' rust-toolchain.toml`
registry := env_var_or_default("REGISTRY", "ghcr.io/muak-os")
tag := env_var_or_default("TAG", "latest")
tools := env_var_or_default("TOOLS", "ghcr.io/muak-os/tools:" + tag)
push := env_var_or_default("PUSH", "false")
latest := env_var_or_default("LATEST", "false")
ci_args := env_var_or_default("CI_ARGS", "")
kernel_signing := env_var_or_default("KERNEL_SIGNING", "")
signature := env_var_or_default("SIGNATURE", "signature.key")
out := `test -f .git && realpath -m "$(git rev-parse --git-common-dir)/../_out" || realpath -m _out`

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
push_cmd := if push == "true" {
    if container_runtime == "podman" { "podman push --tls-verify=false" } else { "docker push" }
} else {
    "true"
}
provenance_arg := if container_runtime == "podman" { "" } else { "--provenance=false" }
common_args := "--platform=linux/" + oci_arch + " --progress=" + env_var_or_default("PROGRESS", "auto") + " --build-arg SOURCE_DATE_EPOCH=" + env_var_or_default("SOURCE_DATE_EPOCH", "0") + " --build-arg ALPINE_VERSION=" + alpine_version + " " + provenance_arg

# Colors

bold := '\e[1m'
cyan := '\e[36m'
green := '\e[32m'
red := '\e[31m'
reset := '\e[0m'

# ─────────────────────────────────────────────────────────────────────────────
# Main Recipes
# ─────────────────────────────────────────────────────────────────────────────

# Full local development build (build → installer → sign → uki + iso)
dev: (build "--release" "") installer sign (artifacts "iso")

# Build kernel image (use `just extract --image ...` to extract artifacts locally)
kernel:
    @printf "{{ cyan }}Building kernel{{ reset }}\n"
    {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ kernel_signing }} {{ pull_arg }} \
        --tag {{ registry }}/kernel:{{ tag }} \
        --file core/kernel/Dockerfile \
        .
    {{ push_cmd }} "{{ registry }}/kernel:{{ tag }}"

# Build Rust packages with cargo (e.g., just build, just build --release, just build granola, just build --release granola)
[arg("release", long="release", value="--release")]
[script]
build release="" *pkgs:
    printf "{{ cyan }}Building Rust packages{{ reset }}\n"
    if [ -n "{{ pkgs }}" ]; then
        for pkg in {{ pkgs }}; do
            if [ "$pkg" = "stub" ]; then
                cargo +nightly-2026-07-31 build {{ release }} --target {{ arch }}-unknown-uefi --features uefi -p stub
            else
                cargo build {{ release }} --target {{ arch }}-unknown-linux-musl -p "$pkg"
            fi
        done
    else
        cargo build {{ release }} --target {{ arch }}-unknown-linux-musl
        cargo +nightly-2026-07-31 build {{ release }} --target {{ arch }}-unknown-uefi --features uefi -p stub
    fi

# Build installer image (default uses local binaries, --prod pulls from registry)
[arg("prod", long="prod", value="true")]
[script]
installer prod="false":
    printf "{{ cyan }}Building installer{{ reset }}\n"
    pkgs="granola provisiond modd networkd apid vmd timed consoled init"
    if [ "{{ prod }}" = "false" ]; then
        test -f {{ out }}/vmlinuz || { printf "{{ red }}{{ bold }}Error:{{ reset }} {{ out }}/vmlinuz not found. Run {{ green }}just kernel{{ reset }} and extract\n"; exit 1; }
        pkg_args=(
            --build-context pkg-kernel={{ out }}
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
    {{ build_cmd }} {{ common_args }} {{ ci_args }} {{ pull_arg }} \
        --build-context services=. \
        --build-arg TOOLS={{ tools }} \
        "${pkg_args[@]}" \
        --tag {{ registry }}/installer:{{ tag }} \
        --file Dockerfile \
        .
    {{ push_cmd }} "{{ registry }}/installer:{{ tag }}"
    printf "{{ green }}Installer image built: {{ registry }}/installer:{{ tag }}{{ reset }}\n"

# Extract an OCI image filesystem to local artifacts
[arg("image", long="image")]
[script]
extract image: _ensure-out
    printf "{{ cyan }}Extracting assets from {{ image }}{{ reset }}\n"
    cid=$({{ container_runtime }} create "{{ image }}")
    {{ container_runtime }} export "$cid" | tar -x -C {{ out }}
    {{ container_runtime }} rm "$cid" >/dev/null
    printf "{{ green }}Assets extracted to {{ out }}/{{ reset }}\n"

# Sign an OCI image in the registry (default to installer image)
[arg("image", long="image")]
sign image=(registry + "/installer:" + tag):
    @printf "{{ cyan }}Signing OCI image {{ image }}{{ reset }}\n"
    {{ container_runtime }} run --rm --network=host \
        -v "{{ absolute_path(signature) }}:/key:ro" \
        {{ tools }} \
        /koci sign \
            --image "{{ image }}" \
            --key /key

# Build boot artifacts (e.g., just artifact uki iso, just artifact raw)
[script]
artifacts *types:
    if [ -z "{{ types }}" ]; then
        printf "{{ red }}{{ bold }}Error:{{ reset }} No artifacts specified. Usage: just artifact <type> [type...]\n"
        exit 1
    fi
    printf "{{ cyan }}Building artifacts: {{ types }}{{ reset }}\n"
    mkdir -p {{ out }}
    printf '[kernel]\nsource = "muak-os/kernel"\n\n[customization]\nextensions = []\n' > "{{ out }}/profile.toml"
    {{ container_runtime }} run --rm --network host \
        -e MUAK_KOCI_CACHE=/out/.cache \
        -v "{{ out }}:/out" \
        {{ tools }} \
        /wizard build \
            --profile /out/profile.toml \
            --artifacts {{ types }} \
            --version {{ tag }} \
            --registry {{ registry }} \
            --arch {{ oci_arch }} \
            --platform metal \
            -o /out

# Build OCI images (e.g., just oci granola kernel installer cli)
[script]
oci *pkgs:
    pkgs="{{ pkgs }}"
    if [ -z "$pkgs" ]; then
        printf "{{ red }}{{ bold }}Error:{{ reset }} No packages specified. Usage: just oci <pkg1> [pkg2...]\n"
        exit 1
    fi
    for pkg in $pkgs; do
        case "$pkg" in
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
                    --file cli/Dockerfile \
                    .
                {{ push_cmd }} "{{ registry }}/muakctl:{{ tag }}"
                if [ "{{ latest }}" = "true" ]; then
                    {{ push_cmd }} "{{ registry }}/muakctl:latest"
                fi
                ;;
            tools)
                printf "{{ cyan }}Building tools OCI{{ reset }} (push={{ push }}, latest={{ latest }})\n"
                {{ build_cmd }} {{ common_args }} --build-arg RUST_VERSION={{ rust_version }} {{ ci_args }} {{ pull_arg }} \
                    --tag {{ registry }}/tools:{{ tag }} \
                    $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/tools:latest" || echo "") \
                    --file tools/Dockerfile \
                    .
                {{ push_cmd }} "{{ registry }}/tools:{{ tag }}"
                if [ "{{ latest }}" = "true" ]; then
                    {{ push_cmd }} "{{ registry }}/tools:latest"
                fi
                ;;
            *)
                dockerfile=""
                for dir in core services tools pkgs; do
                    if [ -f "$dir/$pkg/Dockerfile" ]; then
                        dockerfile="$dir/$pkg/Dockerfile"
                        break
                    fi
                done
                if [ -z "$dockerfile" ]; then
                    printf "{{ red }}{{ bold }}Error:{{ reset }} Dockerfile for $pkg not found in core/, services/, tools/, or pkgs/\n"
                    exit 1
                fi
                printf "{{ cyan }}Building OCI:{{ reset }} $pkg (push={{ push }}, latest={{ latest }})\n"
                {{ build_cmd }} {{ common_args }} --build-arg RUST_VERSION={{ rust_version }} {{ ci_args }} {{ pull_arg }} \
                    --tag {{ registry }}/pkgs/$pkg:{{ tag }} \
                    $([ "{{ latest }}" = "true" ] && echo "--tag {{ registry }}/pkgs/$pkg:latest" || echo "") \
                    --file "$dockerfile" \
                    .
                {{ push_cmd }} "{{ registry }}/pkgs/$pkg:{{ tag }}"
                if [ "{{ latest }}" = "true" ]; then
                    {{ push_cmd }} "{{ registry }}/pkgs/$pkg:latest"
                fi
                ;;
        esac
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
                cargo +nightly-2026-07-31 clippy --all-targets --features uefi -p stub
            else
                cargo clippy --all-targets -p "$pkg"
            fi
        done
    else
        cargo clippy --all-targets
        cargo +nightly-2026-07-31 clippy --all-targets --features uefi -p stub
    fi

# Run tests (e.g., just test or just test yuki koci)
[script]
test *pkgs:
    just _test-run "cargo nextest run" "Running tests for" {{ pkgs }}

# Run tests with coverage (e.g., just coverage, just coverage --missing, or just coverage yuki)
[arg("missing", long="missing", value="--show-missing-lines")]
[script]
coverage missing="" *pkgs:
    cargo llvm-cov clean --workspace
    just _test-run "cargo llvm-cov nextest {{ missing }}" "Running tests with coverage for" {{ pkgs }}

# Run E2E tests suite (requires: qemu, built artifacts)
[script]
e2e: (build "--release" "muakctl") _ensure-fw
    printf "{{ cyan }}Running E2E tests{{ reset }}\n"
    MUAK_ARTIFACTS={{ out }} MUAK_CLI=$(realpath "{{ release_dir }}/muakctl") cargo nextest run -E 'package(e2e)' --test-threads 3

# Boot the ISO in QEMU using user-mode networking and a persistent NVMe disk
[arg("clean", long="reset", value="true")]
[script]
start clean="false": (_require out / "muak.iso" "just dev") _ensure-fw
    if [ "{{ clean }}" = "true" ]; then
        printf "{{ cyan }}Resetting VM state{{ reset }}\n"
        rm -f "/tmp/nvme-disk.img" "/tmp/OVMF_VARS.fd"
    fi

    if [ ! -f "/tmp/nvme-disk.img" ]; then
        printf "{{ cyan }}Creating persistent NVMe disk{{ reset }}\n"
        qemu-img create -f raw "/tmp/nvme-disk.img" 5G >/dev/null
    fi

    if [ ! -f "/tmp/OVMF_VARS.fd" ]; then
        printf "{{ cyan }}Creating persistent OVMF vars{{ reset }}\n"
        cp "{{ out }}/OVMF_VARS.fd" "/tmp/OVMF_VARS.fd"
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
        -drive if=pflash,format=raw,readonly=on,file="{{ out }}/OVMF_CODE.secboot.fd" \
        -drive if=pflash,format=raw,file="/tmp/OVMF_VARS.fd" \
        -serial stdio \
        -display none \
        -netdev user,id=net0,hostfwd=tcp:127.0.0.1:50051-:50051 \
        -device virtio-net-pci,netdev=net0 \
        -drive file="{{ out }}/muak.iso",format=raw,media=cdrom,if=none,id=cdrom0,readonly=on \
        -device ide-cd,drive=cdrom0,bootindex=2 \
        -drive file="/tmp/nvme-disk.img",format=raw,if=none,id=nvme0 \
        -device nvme,serial=deadbeef,drive=nvme0,bootindex=1

# Profile a Rust binary with perf and render a CPU flamegraph
# (e.g., just flame wizard build --artifacts iso --version latest --arch amd64 --platform metal)
# Output always goes to {{ out }}; any user-supplied -o/--output-dir is ignored.
[script]
flame pkg *args: _ensure-out
    flame=$(command -v cargo-flamegraph || echo "$HOME/.cargo/bin/cargo-flamegraph")
    [ -x "$flame" ] || { printf "{{ red }}Missing flamegraph. Run: cargo install flamegraph{{ reset }}\n"; exit 1; }
    if [ "$(cat /proc/sys/kernel/perf_event_paranoid)" -gt 1 ]; then
        printf "{{ red }}perf_event_paranoid too high; run once: sudo sysctl -w kernel.perf_event_paranoid=1{{ reset }}\n"
        exit 1
    fi
    if [ "$(cat /proc/sys/kernel/kptr_restrict)" -gt 0 ]; then
        printf "{{ red }}kptr_restrict=$(cat /proc/sys/kernel/kptr_restrict): kernel frames will show as [unknown]; run once as root: sysctl -w kernel.kptr_restrict=0{{ reset }}\n"
    fi
    args=""
    skip_o=false
    for a in {{ args }}; do
        if [ "$skip_o" = "true" ]; then skip_o=false; continue; fi
        case "$a" in
            -o|--output-dir) skip_o=true ;;
            -o?*|--output-dir=*) ;;
            *) args="$args $a" ;;
        esac
    done
    RUSTFLAGS="-C force-frame-pointers=yes -Clink-arg=-Wl,--no-rosegment" \
        "$flame" flamegraph --profile profiling --no-inline -c "record -F 997 --call-graph dwarf -o {{ out }}/flame.data" -o {{ out }}/flamegraph.svg -p {{ pkg }} -- $args -o {{ out }}
    printf "{{ green }}Flamegraph written to {{ out }}/flamegraph.svg{{ reset }}\n"

# Check kernel config, cmdline & sysctl against KSPP security hardening recommendations
[script]
kspp:
    config="config-{{ oci_arch }}"
    cmdline="cmdline-{{ oci_arch }}.txt"
    sysctl="sysctl-{{ oci_arch }}.conf"
    printf "{{ cyan }}Checking kernel config, cmdline & sysctl against KSPP recommendations{{ reset }}\n"
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
    rm -rf {{ out }}
    @printf "{{ green }}Clean complete{{ reset }}\n"

# ─────────────────────────────────────────────────────────────────────────────
# Private Helpers
# ─────────────────────────────────────────────────────────────────────────────

[private]
[script]
_ensure-fw: _ensure-out
    if [ "{{ arch }}" != "x86_64" ]; then
        printf "{{ red }}{{ bold }}Error:{{ reset }} QEMU helpers currently support only x86_64\n"
        exit 1
    fi

    if [ ! -f "{{ out }}/OVMF_VARS.fd" ] || [ ! -f "{{ out }}/OVMF_CODE.secboot.fd" ]; then
        printf "{{ cyan }}Fetching OVMF firmware files{{ reset }}\n"
        {{ container_runtime }} run --rm --network=host -v {{ out }}:/out docker.io/alpine:{{ alpine_version }} sh -c '
        set -euo pipefail
        apk add --no-cache wget libarchive-tools >/dev/null 2>&1
        wget -q -O /tmp/edk2.rpm https://kojipkgs.fedoraproject.org/packages/edk2/20251119/10.fc44/noarch/edk2-ovmf-20251119-10.fc44.noarch.rpm
        bsdtar -xf /tmp/edk2.rpm -C /tmp
        cp /tmp/usr/share/edk2/ovmf/OVMF_VARS.fd /out/OVMF_VARS.fd
        cp /tmp/usr/share/edk2/ovmf/OVMF_CODE.secboot.fd /out/OVMF_CODE.secboot.fd'
        printf "{{ green }}OVMF firmware files ready{{ reset }}\n"
    fi

[private]
_ensure-out:
    @mkdir -p {{ out }}

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

