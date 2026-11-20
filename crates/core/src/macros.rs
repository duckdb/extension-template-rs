//! A table macro: `SELECT * FROM glaze()`.
//!
//! A macro is SQL that DuckDB expands where it is called, so it needs no
//! callbacks. [`MacroBody::Table`] makes it callable like a table function;
//! [`MacroBody::Scalar`] would make it an expression instead. Macros can only
//! be registered by an extension; plain SQL uses `CREATE MACRO`.

use duckdb_neo::{
    Result,
    extension::Extension,
    macro_function::{MacroBody, MacroFunctionBuilder},
    signature::{FunctionDocumentationBuilder, SignatureBuilder},
};

use crate::CATEGORY;

pub fn register(extension: &Extension) -> Result<()> {
    MacroFunctionBuilder::new(
        "glaze",
        SignatureBuilder::new().documentation(
            FunctionDocumentationBuilder::new()
                .description("A table macro with one row expressing how we feel about Rust.")
                .add_example("Glaze", "SELECT * FROM glaze()")
                .add_category(CATEGORY),
        ),
        MacroBody::Table("SELECT 'Rust is awesome!' AS glaze".into()),
    )
    .register(extension)
}
