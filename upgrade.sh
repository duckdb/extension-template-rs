#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "Usage: $0 OLD_VERSION NEW_VERSION (e.g. $0 v2.0.0 v2.0.1)" >&2
    exit 1
fi

OLD_VERSION=${1#v}
NEW_VERSION=${2#v}
for VERSION in "$OLD_VERSION" "$NEW_VERSION"; do
    if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "Invalid DuckDB version: $VERSION (expected major.minor.patch)" >&2
        exit 1
    fi
done

if [ "$OLD_VERSION" = "$NEW_VERSION" ]; then
    echo "Already at DuckDB v$NEW_VERSION"
    exit 0
fi

cd "$(dirname "$0")"

sed_inplace() {
    if sed --version 2>/dev/null | grep -q GNU; then
        sed -i "$@"
    else
        sed -i '' "$@"
    fi
}

crate_version() {
    local MAJOR MINOR PATCH
    IFS=. read -r MAJOR MINOR PATCH <<<"$1"
    printf '1.%d%02d%02d.0' "$((10#$MAJOR))" "$((10#$MINOR))" "$((10#$PATCH))"
}

OLD_CRATE_VERSION=$(crate_version "$OLD_VERSION")
NEW_CRATE_VERSION=$(crate_version "$NEW_VERSION")
if ! grep -Fxq "TARGET_DUCKDB_VERSION=v$OLD_VERSION" Makefile ||
    ! grep -Fq "version = \"~$OLD_CRATE_VERSION\"" Cargo.toml; then
    echo "OLD_VERSION does not match the current Makefile and Cargo.toml" >&2
    exit 1
fi

# Resolve the release tag before changing local version references.
git submodule update --init extension-ci-tools
git -C extension-ci-tools fetch origin "refs/tags/v$NEW_VERSION:refs/tags/v$NEW_VERSION"
git -C extension-ci-tools checkout --detach "v$NEW_VERSION"

sed_inplace "s!v${OLD_VERSION//./\\.}\$!v$NEW_VERSION!" \
    Makefile .github/workflows/MainDistributionPipeline.yml
sed_inplace "s!\"~${OLD_CRATE_VERSION//./\\.}\"!\"~$NEW_CRATE_VERSION\"!" Cargo.toml
cargo update -p duckdb

echo "Updated DuckDB v$OLD_VERSION -> v$NEW_VERSION (duckdb-rs $NEW_CRATE_VERSION)"
