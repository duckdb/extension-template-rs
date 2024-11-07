.PHONY: clean clean_all

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

# TODO: these values are currently duplicated in lib.rs. Fetching them from the env variables in the proc macro should fix this
EXTENSION_NAME=rusty_quack
MINIMUM_DUCKDB_VERSION=v0.0.1

all: configure debug

# Include makefiles from DuckDB
include duckdb_extension_c_api.Makefile
include duckdb_extension_rs.Makefile

configure: venv platform extension_version

debug: build_extension_library_debug build_extension_with_metadata_debug
release: build_extension_library_release build_extension_with_metadata_release

test: test_debug
test_debug: test_extension_debug
test_release: test_extension_release

clean: clean_build clean_rust
clean_all: clean_configure clean_build clean_rust