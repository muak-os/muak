#!/usr/bin/env bash
#
# Check the network crate layering rules described in docs/network-architecture.md.
#
# Only crate manifests and source imports are inspected: no cargo invocation and
# no network access.

set -euo pipefail

if ! command -v rg > /dev/null 2>&1; then
    printf 'check-network-layers: ripgrep (rg) is required to inspect source imports\n' >&2
    exit 1
fi

network_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/libs/network
failed=0

sans_io_crates=(netmodel ip-method ip-static dhcp slaac iface net-supervisor net-runtime net-loop)
sans_io_forbidden=(
    # Async runtime and platform APIs.
    tokio rustix libc rtnetlink
    # rtnetlink protocol stack.
    netlink netlink-monitor netlink-proto netlink-sys
    netlink-packet-core netlink-packet-route
    # Legacy workspace crates that pull tokio or netlink internally.
    netdev linkctl dhcp-client packet tap
)

verbs_crates=(netlink-linux)
sync_adapter_crates=(packet-v4-linux icmpv6-linux tap-linux)
async_backend=net-runtime-linux
async_backend_forbidden=(tokio netlink-proto netlink-sys)

# Linux specific crates: no layer above the backend may depend on them.
linux_crates=("${verbs_crates[@]}" "${sync_adapter_crates[@]}" "$async_backend")

emit() {
    printf 'check-network-layers: %s\n' "$1" >&2
    failed=1
}

manifest() {
    printf '%s/%s/Cargo.toml' "$network_dir" "$1"
}

crate_exists() {
    [ -f "$(manifest "$1")" ]
}

# Reports whether the crate manifest declares the given package name as a
# dependency, including `package = "..."` renames and quoted keys.
depends_on() {
    awk -v want="$2" '
        BEGIN {
            quote = sprintf("%c", 39)
            found = 0
        }
        function unquote(value) {
            gsub(/"/, "", value)
            gsub(quote, "", value)
            return value
        }
        function package_of(line, value) {
            if (line !~ /package[[:space:]]*=/) return ""
            value = line
            sub(/^.*package[[:space:]]*=[[:space:]]*/, "", value)
            sub(/^[[:space:]]*/, "", value)
            while (substr(value, 1, 1) == "\"" || substr(value, 1, 1) == quote) value = substr(value, 2)
            sub(/[^A-Za-z0-9_-].*$/, "", value)
            return value
        }
        /^[[:space:]]*\[/ {
            section = $0
            gsub(/[][[:space:]]/, "", section)
            kind = "none"
            if (section ~ /(^|\.)(dependencies|dev-dependencies|build-dependencies)$/) {
                kind = "keys"
            } else if (section ~ /(^|\.)(dependencies|dev-dependencies|build-dependencies)\./) {
                name = section
                sub(/^.*(dependencies|dev-dependencies|build-dependencies)\./, "", name)
                if (unquote(name) == want) found = 1
                kind = "props"
            }
            continuation = 0
            next
        }
        kind == "props" {
            if (package_of($0) == want) found = 1
            next
        }
        kind != "keys" { next }
        /^[[:space:]]/ {
            if (continuation && package_of($0) == want) found = 1
            if ($0 ~ /\}/) continuation = 0
            next
        }
        /^#/ { next }
        {
            line = $0
            sub(/#.*/, "", line)
            if (line !~ /=/) next
            name = line
            sub(/=.*/, "", name)
            gsub(/[[:space:]]/, "", name)
            if (unquote(name) == want) found = 1
            if (package_of(line) == want) found = 1
            continuation = (line ~ /\{/ && line !~ /\}/) ? 1 : 0
        }
        END { if (!found) exit 1 }
    ' "$(manifest "$1")"
}

import_pattern() {
    printf '(^|[^A-Za-z0-9_])(use|extern[[:space:]]+crate)[[:space:]]+%s([^A-Za-z0-9_]|$)' "$1"
}

matching_source() {
    local crate=$1
    local pattern=$2
    local dir="$network_dir/$crate/src"

    [ -d "$dir" ] || return 0
    rg --no-heading --line-number --glob '*.rs' -- "$pattern" "$dir" 2>/dev/null || true
}

forbid_crates() {
    local rule=$1
    local crate=$2
    shift 2

    if ! crate_exists "$crate"; then
        emit "$rule: crate '$crate' no longer exists"
        return 0
    fi

    local name
    local ident
    local hits
    for name in "$@"; do
        if depends_on "$crate" "$name"; then
            emit "$rule: crate '$crate' depends on '$name'"
        fi

        ident=${name//-/_}
        hits=$(matching_source "$crate" "$(import_pattern "$ident")")
        if [ -n "$hits" ]; then
            while IFS= read -r hit; do
                emit "$rule: crate '$crate' imports '${ident}': ${hit#"$network_dir/"}"
            done <<< "$hits"
        fi
    done
}

forbid_identifier() {
    local rule=$1
    local crate=$2
    local identifier=$3

    if ! crate_exists "$crate"; then
        emit "$rule: crate '$crate' no longer exists"
        return 0
    fi

    local hits
    hits=$(matching_source "$crate" "(^|[^A-Za-z0-9_])$identifier([^A-Za-z0-9_]|$)")
    if [ -n "$hits" ]; then
        while IFS= read -r hit; do
            emit "$rule: crate '$crate' uses '$identifier': ${hit#"$network_dir/"}"
        done <<< "$hits"
    fi
}

network_crates() {
    local dir
    for dir in "$network_dir"/*/; do
        [ -f "$dir/Cargo.toml" ] || continue
        basename "$dir"
    done
}

check_sans_io() {
    local crate
    local forbidden=("${sans_io_forbidden[@]}" "${verbs_crates[@]}" "${sync_adapter_crates[@]}")
    for crate in "${sans_io_crates[@]}"; do
        forbid_crates 'sans-io layer' "$crate" "${forbidden[@]}"
    done
}

check_netlink_verbs() {
    forbid_crates 'netlink verbs' netlink-linux \
        netlink-proto netlink-sys tokio rustix libc rtnetlink
}

check_sync_adapters() {
    local crate
    for crate in "${sync_adapter_crates[@]}"; do
        forbid_crates 'sync adapter' "$crate" tokio
    done
}

check_runtime_ports() {
    forbid_crates 'runtime ports' net-runtime "${linux_crates[@]}"
}

check_driver_loop() {
    forbid_crates 'driver loop' net-loop "${linux_crates[@]}" tokio
}

check_async_backend() {
    local crate
    for crate in $(network_crates); do
        [ "$crate" = "$async_backend" ] && continue
        forbid_crates 'async backend' "$crate" "${async_backend_forbidden[@]}"
        forbid_identifier 'async backend' "$crate" AsyncFd
    done
}

check_sans_io
check_netlink_verbs
check_sync_adapters
check_runtime_ports
check_driver_loop
check_async_backend

if [ "$failed" -ne 0 ]; then
    printf 'check-network-layers: layering rules violated\n' >&2
    exit 1
fi

printf 'check-network-layers: network crate layering is valid\n'
