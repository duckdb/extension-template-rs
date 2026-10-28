# DuckDB Rust extension template
This is an **experimental** template for Rust based extensions based on the C Extension API of DuckDB. The goal is to
turn this eventually into a stable basis for pure-Rust DuckDB extensions that can be submitted to the Community extensions
repository

Features:
- No DuckDB build required
- No C++ or C code required
- CI/CD chain preconfigured
- (Coming soon) Works with community extensions

## Cloning

Clone the repo with submodules

```shell
git clone --recurse-submodules <repo>
```

## Dependencies
In principle, these extensions can be compiled with the Rust toolchain alone. However, this template relies on some additional
tooling to make life a little easier and to be able to share CI/CD infrastructure with extension templates for other languages:

- Python3
- Python3-venv
- [Make](https://www.gnu.org/software/make)
- Git

Installing these dependencies will vary per platform:
- For Linux, these come generally pre-installed or are available through the distro-specific package manager.
- For MacOS, [homebrew](https://formulae.brew.sh/).
- For Windows, [chocolatey](https://community.chocolatey.org/).

## Project layout

The Cargo workspace contains three crates:

- `crates/core` (`rusty_quack_core`): the shared function implementations and registration.
- `crates/native` (`rusty_quack`): a native `cdylib` with the DuckDB extension entrypoint.
- `crates/wasm` (`rusty_quack_wasm`): a Wasm `staticlib` with the same entrypoint.

Both entrypoint crates depend on `rusty_quack_core`. Each feature lives in its own module under `crates/core/src/`,
and `crates/core/src/lib.rs` registers them; see [What the extension registers](#what-the-extension-registers).
Dependency versions and release settings are shared in the root `Cargo.toml`.

## Building
After installing the dependencies, building is a two-step process. Firstly run:
```shell
make configure
```
This will ensure a Python venv is set up with DuckDB and DuckDB's test runner installed. Additionally, depending on configuration,
DuckDB will be used to determine the correct platform for which you are compiling.

Then, to build the extension run:
```shell
make debug
```
This delegates the build process to cargo, which will produce a shared library in `target/debug/<shared_lib_name>`. After this step,
a script is run to transform the shared library into a loadable extension by appending a binary footer. The resulting extension is written
to the `build/debug` directory.

To create optimized release binaries, simply run `make release` instead.

### WASM

To build for WASM targets, please run `make wasm_eh`. This will create `./build/wasm_eh/release/rusty_quack.duckdb_extension.wasm`, that can then be used inside nodejs or an browser.
### Running the extension
To run the extension code, start `duckdb` with `-unsigned` flag. This will allow you to load the local extension file.

```sh
duckdb -unsigned -cmd "LOAD '$PWD/build/debug/extension/rusty_quack/rusty_quack.duckdb_extension';"
```

The extension must be loaded into the exact DuckDB version it was built for (`TARGET_DUCKDB_VERSION` in the Makefile).

### What the extension registers

The template is a small tour of what a Rust extension can register through DuckDB's C API v2. Every function is
documented, so `SELECT * FROM duckdb_functions() WHERE categories = ['rusty_quack']` lists them with descriptions and
examples.

| Module (`crates/core/src/`) | Feature of `duckdb-neo` | Try it |
|---|---|---|
| `scalar.rs` | scalar function with per-query init state | `SELECT rusty_echo('Jane')` |
| `quack.rs` | table function with bind data, global and local state | `SELECT * FROM rusty_quack(['Jane', 'Doe'])` |
| `aggregate.rs` | aggregate function: init, update, combine, finalize; order-independent | `SELECT rusty_longest(name) FROM t` |
| `sequence.rs` | chunked table function with projection and filter pushdown, cardinality and progress | `SELECT i FROM rusty_sequence(10000) WHERE i < 10` |
| `quack_type.rs` | custom type and a cast into it, with `CAST` and `TRY_CAST` semantics | `SELECT 3::QUACK, TRY_CAST(11 AS QUACK)` |

```sql
SELECT rusty_echo('Jane');
```

```
┌─────────────────────┐
│ rusty_echo('Jane')  │
│       varchar       │
├─────────────────────┤
│ 🐤 Jane 🦀 Jane     │
└─────────────────────┘
```

```sql
SELECT * FROM rusty_quack(['Jane', 'Doe', 'Loves', 'Ducks']);
```

```
┌─────────┬─────────┬───────────────────────┬─────────────┐
│  name   │  icon   │      concat_name      │ total_index │
│ varchar │ varchar │        varchar        │   uint64    │
├─────────┼─────────┼───────────────────────┼─────────────┤
│ Ducks   │ 🐤      │ Ducks|                │           0 │
│ Loves   │ 🦀      │ Ducks|Loves|          │           1 │
│ Doe     │ 🐥      │ Ducks|Loves|Doe|      │           2 │
│ Jane    │ 🐤      │ Ducks|Loves|Doe|Jane| │           3 │
└─────────┴─────────┴───────────────────────┴─────────────┘
```

`EXPLAIN` shows the optimizer hooks at work: a filter on `i < constant` disappears from the plan because
`rusty_sequence` claims it, and the scan lists only the projected columns.

```sql
EXPLAIN SELECT i FROM rusty_sequence(10000) WHERE i < 5;
```

```
╭─ Rusty Sequence ─────────╮
│ Function: RUSTY_SEQUENCE │
│ Projections: i           │
│ ~10,000 rows             │
╰──────────────────────────╯
```

## Testing
This extension uses the DuckDB Python client for testing. This should be automatically installed in the `make configure` step.
The tests themselves are written in the SQLLogicTest format, just like most of DuckDB's tests. A sample test can be found in
`test/sql/<extension_name>.test`. To run the tests using the *debug* build:

```shell
make test_debug
```

or for the *release* build:
```shell
make test_release
```

### Version switching
Testing with different DuckDB versions is really simple:

First, run
```
make clean_all
```
to ensure the previous `make configure` step is deleted.

Then, run
```
DUCKDB_TEST_VERSION=v1.3.2 make configure
```
to select a different duckdb version to test with

Finally, build and test with
```
make debug
make test_debug
```

### Known issues
This is a bit of a footgun, but the extensions produced by this template may (or may not) be broken on windows on python3.11
with the following error on extension load:
```shell
IO Error: Extension '<name>.duckdb_extension' could not be loaded: The specified module could not be found
```
This was resolved by using python 3.12
