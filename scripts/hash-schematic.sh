#!/bin/bash
set -euo pipefail

if [ $# -eq 0 ]; then
    echo ""
    exit 0
fi

EXTENSIONS="$@"

SORTED_EXTENSIONS=$(echo "$EXTENSIONS" | tr ' ' '\n' | sort | tr '\n' ',' | sed 's/,$//')

echo -n "$SORTED_EXTENSIONS" | sha256sum | awk '{print $1}' | cut -c1-16
