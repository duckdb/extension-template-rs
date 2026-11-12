//! A query pragma: `PRAGMA duck_count('🦆🦆')`.
//!
//! [`PragmaQueryCallbacks::query`] runs before the statement is bound and
//! returns SQL that runs in its place. Here it adds the ducks it was given to a
//! running total kept on the registered implementation, and returns a query
//! that reports the new total.

use std::sync::atomic::{AtomicU64, Ordering};

use duckdb_neo::{
    Result,
    bind_arguments::BindArguments,
    connection::Context,
    error::Error,
    extension::Extension,
    pragma_function::{PragmaFunctionBuilder, PragmaQueryCallbacks},
    signature::{FunctionDocumentationBuilder, FunctionParameter, SignatureBuilder},
};

use crate::CATEGORY;

const DUCK: char = '🦆';

/// The registered implementation; the total is shared by every connection to the database.
struct DuckCount {
    total: AtomicU64,
}

impl PragmaQueryCallbacks for DuckCount {
    /// Runs once per `PRAGMA` statement. Typical work:
    /// - Read the arguments, which are always constants.
    /// - Return the SQL to run instead, or an error to reject the statement.
    fn query(&self, _context: &Context, arguments: BindArguments<'_>) -> Result<String> {
        let ducks = arguments
            .value(0)?
            .map(|value| value.get::<String>())
            .transpose()?
            .flatten()
            .ok_or_else(|| Error::invalid_input("duck_count: the ducks must not be NULL"))?;

        if let Some(other) = ducks.chars().find(|&c| c != DUCK) {
            return Err(Error::invalid_input(format!(
                "duck_count: '{other}' is not a {DUCK}"
            )));
        }

        let count = ducks.chars().count() as u64;
        let total = self.total.fetch_add(count, Ordering::Relaxed) + count;
        Ok(format!("SELECT {total}::UBIGINT AS duck_count"))
    }
}

pub fn register(extension: &Extension) -> Result<()> {
    PragmaFunctionBuilder::query(
        "duck_count",
        SignatureBuilder::new()
            .param(FunctionParameter::new::<String>("ducks"))
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description("Add the 🦆 in ducks to a running total and return the total.")
                    .add_example("Two ducks", "PRAGMA duck_count('🦆🦆')")
                    .add_category(CATEGORY),
            ),
        DuckCount {
            total: AtomicU64::new(0),
        },
    )
    .register(extension)
}
