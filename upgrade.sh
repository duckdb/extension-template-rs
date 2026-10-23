#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "Usage: $0 NEO_VERSION MACROS_VERSION (e.g. $0 0.20000.0 1.20000.0)" >&2
    exit 1
}

VERSION_PATTERN='^(0|[1-9][0-9]*)\.([1-9][0-9]{4,})\.(0|[1-9][0-9]*)$'
if [ "$#" -ne 2 ] || [[ ! "$1" =~ $VERSION_PATTERN ]] || [[ ! "$2" =~ $VERSION_PATTERN ]]; then
    usage
fi

NEO_VERSION=$1
MACROS_VERSION=$2
IFS=. read -r _ ENCODED_VERSION _ <<<"$NEO_VERSION"
IFS=. read -r _ MACROS_ENCODED_VERSION _ <<<"$MACROS_VERSION"
if [[ "$ENCODED_VERSION" != "$MACROS_ENCODED_VERSION" ]] || ((ENCODED_VERSION < 20000)); then
    echo "Both crates must target the same DuckDB version (2.0.0 or newer)." >&2
    usage
fi
printf -v API_VERSION '%d.%d.%d' \
    "$((ENCODED_VERSION / 10000))" "$((ENCODED_VERSION / 100 % 100))" "$((ENCODED_VERSION % 100))"
DUCKDB_VERSION=v$API_VERSION

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
sed_inplace -E "/^duckdb-neo[[:space:]]*=/s!version = \"[^\"]+\"!version = \"~$NEO_VERSION\"!" Cargo.toml
sed_inplace -E "/^duckdb-loadable-macros[[:space:]]*=/s!version = \"[^\"]+\"!version = \"~$MACROS_VERSION\"!" Cargo.toml
sed_inplace -E "s!api_version = \"[^\"]+\"!api_version = \"$API_VERSION\"!" \
    crates/native/src/lib.rs crates/wasm/src/lib.rs

echo "Updated duckdb-neo to $NEO_VERSION and duckdb-loadable-macros to $MACROS_VERSION / DuckDB $DUCKDB_VERSION"
echo "Note: run cargo update to refresh Cargo.lock; dependencies were not resolved."
echo "Note: update the extension-ci-tools workflow ref, ci_tools_version, and submodule manually."
