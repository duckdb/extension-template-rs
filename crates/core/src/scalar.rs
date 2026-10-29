//! A scalar function: `rusty_echo(input)`.
//!
//! [`ScalarCallbacks`] is the lifecycle DuckDB runs for a scalar function.
//! DuckDB calls back in this order:
//!
//! 1. `bind` (kept at its default here) runs once per call site in a query.
//! 2. `init` runs once per query and worker, before any `exec`, when the
//!    function is registered with `with_init`.
//! 3. `exec` runs once per input chunk and fills the output vector.
//!
//! The documentation builder makes the function discoverable in
//! `duckdb_functions()`.

use duckdb_neo::{
    DuckDBType, Result,
    connection::Context,
    data_chunk::VectorCollection,
    error::Error,
    extension::Extension,
    scalar::{ScalarCallbacks, ScalarFunctionBuilder, ScalarInitCallbacks},
    signature::{FunctionDocumentationBuilder, FunctionParameter, SignatureBuilder},
    vector::{Unknown, Vector},
};

use crate::CATEGORY;

/// Echoes its input twice: `<prefix> <input> <postfix> <input>`.
///
/// The struct itself is the registered implementation, so fields set at
/// registration time (the prefix) are available in every callback.
struct EchoScalar {
    prefix: String,
}

impl ScalarInitCallbacks for EchoScalar {
    /// Runs once per query and worker, before the first `exec`. Typical work:
    /// - Build state that is expensive to create and reusable across chunks, such as
    ///   a compiled regex, a lookup table, or a buffer.
    /// - Read settings from `context` that stay fixed for the query.
    ///
    /// The state is handed to every `exec` as `init_data`; here it is the postfix.
    fn init(
        &self,
        _bind_data: Option<&Self::BindData>,
        _context: &Context,
    ) -> Result<Self::InitData> {
        Ok("🦀".into())
    }
}

impl ScalarCallbacks for EchoScalar {
    /// Whatever `bind` computes once per call site; this function keeps the default `bind`, so nothing.
    type BindData = ();
    /// The per-query, per-worker state created by `init`; here the postfix.
    type InitData = String;

    /// Runs once per input chunk. Typical work:
    /// - Narrow `output` to the declared return type with `cast`.
    /// - Size the output to the input row count.
    /// - Read one vector per argument from `vectors`, and write one result per row,
    ///   `None` for NULL.
    ///
    /// `exec` may run on several workers at once, so it must not keep state in `self`.
    fn exec(
        &self,
        _bind_data: Option<&Self::BindData>,
        init_data: Option<&mut Self::InitData>,
        _context: &Context,
        vectors: &mut VectorCollection,
        output: Vector<'_, Unknown>,
    ) -> Result<()> {
        // The output vector arrives untyped; narrow it to the declared return type.
        let mut output = output.cast::<String>()?;
        output.set_size(vectors.row_count()?)?;

        let prefix = &self.prefix;
        let postfix = init_data.ok_or_else(|| Error::internal("rusty_echo: init state missing"))?;

        let input = vectors.get_vector_at::<String>(0)?;
        let reader = input.reader()?;
        let mut writer = output.writer()?;

        for (row, value) in reader.iter().enumerate() {
            match value {
                Some(value) => {
                    writer.set(row, Some(&format!("{prefix} {value} {postfix} {value}")))?
                }
                None => writer.set(row, None)?,
            }
        }
        Ok(())
    }
}

pub fn register(extension: &Extension) -> Result<()> {
    ScalarFunctionBuilder::new(
        "rusty_echo",
        SignatureBuilder::new()
            .param(FunctionParameter::new::<String>("input"))
            .return_type(String::logical_type(extension)?)
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description("Echo back the input you provided, but with a rusty twist!")
                    .add_example("Echo a name", "SELECT rusty_echo('Jane')")
                    .add_category(CATEGORY),
            ),
        EchoScalar {
            prefix: "🐤".to_string(),
        },
    )
    .with_init()
    .register(extension)
}
