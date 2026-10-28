//! A scalar function: `rusty_echo(input)`.
//!
//! [`ScalarCallbacks::exec`] turns an input chunk into an output vector.
//! [`ScalarInitCallbacks::init`] runs once per query and worker before any
//! `exec` and can prepare state for it. The documentation builder makes the
//! function discoverable in `duckdb_functions()`.

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
struct EchoScalar {
    prefix: String,
}

impl ScalarInitCallbacks for EchoScalar {
    /// The postfix is prepared once per query and worker.
    fn init(&self, _bind_data: Option<&Self::BindData>, _context: &Context) -> Result<Self::InitData> {
        Ok("🦀".into())
    }
}

impl ScalarCallbacks for EchoScalar {
    type BindData = ();
    type InitData = String;

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
                Some(value) => writer.set(row, Some(&format!("{prefix} {value} {postfix} {value}")))?,
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
