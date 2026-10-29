//! `rusty_quack`: a tour of what a Rust extension can register through DuckDB's C API v2.
//!
//! Each module shows one feature of [`duckdb_neo`] and registers it on the
//! [`Extension`] handed to the entrypoint:
//!
//! | Module | Feature | Try it |
//! |---|---|---|
//! | [`scalar`] | scalar function with per-query init state | `SELECT rusty_echo('Jane')` |
//! | [`quack`] | table function with bind data, global and local state | `FROM rusty_quack(['a', 'b'])` |
//! | [`aggregate`] | aggregate function with a full state lifecycle | `SELECT rusty_longest(name) FROM t` |
//! | [`sequence`] | chunked table function with projection and filter pushdown, cardinality and progress | `FROM rusty_sequence(10000) WHERE i < 10` |
//! | [`quack_type`] | custom type and a cast into it | `SELECT 3::QUACK` |

use duckdb_neo::{Result, extension::Extension};

pub mod aggregate;
pub mod quack;
pub mod quack_type;
pub mod scalar;
pub mod sequence;

/// The category every function of this extension is documented under in `duckdb_functions()`.
pub const CATEGORY: &str = "rusty_quack";

/// Register everything the extension provides.
pub fn register(extension: &Extension) -> Result<()> {
    scalar::register(extension)?;
    quack::register(extension)?;
    aggregate::register(extension)?;
    sequence::register(extension)?;
    quack_type::register(extension)?;
    Ok(())
}
