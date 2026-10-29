//! A custom type and a cast into it: `3::QUACK`.
//!
//! Two registrations work together:
//!
//! 1. [`CustomType`] registers `QUACK` as a named alias of `VARCHAR`, so it can
//!    be used in `CAST` and column definitions.
//! 2. [`CastFunctionBuilder`] adds the `INTEGER -> QUACK` conversion, with
//!    [`CastFunctionCallbacks::exec`] as the only callback; DuckDB runs it once
//!    per input chunk. [`CastMode`] tells it whether a failing row must raise an
//!    error (`CAST`) or become NULL (`TRY_CAST`).

use duckdb_neo::{
    DuckDBType, Result,
    cast::{CastFunctionBuilder, CastFunctionCallbacks, CastMode},
    connection::Context,
    custom_type::CustomType,
    error::Error,
    extension::Extension,
    vector::Vector,
};

/// The type's name in SQL.
pub const TYPE_NAME: &str = "QUACK";

/// The largest integer the cast accepts.
const MAX_QUACKS: i32 = 10;

/// The registered implementation of the `INTEGER -> QUACK` cast.
struct IntegerToQuack;

impl CastFunctionCallbacks for IntegerToQuack {
    /// The Rust type of the source vector; it fixes the source logical type (`INTEGER`).
    type InputType = i32;
    /// The Rust type of the target vector; `QUACK` is stored as `VARCHAR`, so `String`.
    type OutputType = String;

    /// Runs once per input chunk. Typical work:
    /// - Size the output to the input, then read and write row by row, `None` for NULL.
    /// - On a row that cannot convert, consult `mode`: return an error for `CAST`,
    ///   write `None` for `TRY_CAST`.
    fn exec(
        &self,
        _context: &Context,
        mode: CastMode,
        input: Vector<'_, Self::InputType>,
        mut output: Vector<'_, Self::OutputType>,
    ) -> Result<()> {
        output.set_size(input.len())?;
        let reader = input.reader()?;
        let mut writer = output.writer()?;

        for (row, value) in reader.iter().enumerate() {
            match value {
                None => writer.set(row, None)?,
                Some(&count) if (0..=MAX_QUACKS).contains(&count) => {
                    writer.set(row, Some(&"quack".repeat(count as usize)))?
                }
                Some(count) => match mode {
                    CastMode::Try => writer.set(row, None)?,
                    CastMode::Normal => {
                        return Err(Error::invalid_input(format!(
                            "cannot cast {count} to {TYPE_NAME}: the value must be between 0 and {MAX_QUACKS}"
                        )));
                    }
                },
            }
        }
        Ok(())
    }
}

pub fn register(extension: &Extension) -> Result<()> {
    let base = String::logical_type(extension)?;
    CustomType::new(TYPE_NAME, base.clone()).register(extension)?;

    // The cast targets the aliased type, so `typeof(3::QUACK)` reports QUACK rather than VARCHAR.
    let quack = base.with_alias(extension, TYPE_NAME)?;
    CastFunctionBuilder::new(i32::logical_type(extension)?, quack, IntegerToQuack).register(extension)
}
