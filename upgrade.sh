#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ] || [[ ! "$1" =~ ^[0-9]+\.[1-9][0-9]{4,}\.[0-9]+$ ]]; then
    echo "Usage: $0 NEW_CARGO_VERSION (e.g. $0 1.10506.1)" >&2
    exit 1
fi

CARGO_VERSION=$1
IFS=. read -r _ ENCODED_VERSION _ <<<"$CARGO_VERSION"
printf -v DUCKDB_VERSION 'v%d.%d.%d' \
    "$((ENCODED_VERSION / 10000))" "$((ENCODED_VERSION / 100 % 100))" "$((ENCODED_VERSION % 100))"

cd "$(dirname "$0")"

sed_inplace() {
    if sed --version 2>/dev/null | grep -q GNU; then
        sed -i "$@"
    else
        sed -i '' "$@"
    fi
}

sed_inplace -E "s!^TARGET_DUCKDB_VERSION=.*!TARGET_DUCKDB_VERSION=$DUCKDB_VERSION!" Makefile
sed_inplace -E "/duckdb_version:/s!v[0-9]+\.[0-9]+\.[0-9]+!$DUCKDB_VERSION!g" \
    .github/workflows/MainDistributionPipeline.yml
sed_inplace -E "/^duckdb[[:space:]]*=/s!version = \"[^\"]+\"!version = \"~$CARGO_VERSION\"!" Cargo.toml

echo "Updated to duckdb-rs $CARGO_VERSION / DuckDB $DUCKDB_VERSION"
echo "Note: update the extension-ci-tools workflow ref, ci_tools_version, and submodule manually."
