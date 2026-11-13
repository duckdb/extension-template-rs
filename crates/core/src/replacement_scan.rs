//! Replacement scans: `SELECT * FROM 'ducks.quackfile'` and `SELECT * FROM quack`.
//!
//! DuckDB consults [`ReplacementScanCallbacks::scan`] while binding, for every
//! table name the catalog cannot resolve. A quoted path counts as a name. Two
//! scans are registered, each claiming its names and leaving the rest for the
//! next scan, or the usual "table does not exist" error:
//!
//! - Names ending in `.quackfile` become a call to the [`READ_FUNCTION`] table
//!   function, the path as its argument.
//! - The name `quack` becomes the subquery [`QUACK_QUERY`]; a real table named
//!   `quack` still wins, since the catalog is checked first.
//!
//! [`READ_FUNCTION`] reads the [`crate::copy`] format without a target table:
//!
//! 1. `bind` reads the first line to count the columns, all VARCHAR and named
//!    `column0`, `column1`, ... like `read_csv` without a header.
//! 2. `init_global_state` opens the file again and limits the scan to one thread,
//!    since lines must be read in order.
//! 3. `exec` decodes the next rows until the file ends.

use std::{num::NonZeroUsize, sync::Mutex};

use duckdb_neo::{
    DuckDBType, Result, ToValue,
    bind_arguments::BindArgument,
    connection::Context,
    data_chunk::DataChunkRef,
    error::Error,
    extension::Extension,
    qualified_name::QualifiedName,
    replacement_scan::{
        ReplacementHandle, ReplacementScanBuilder, ReplacementScanCallbacks, ReplacementType,
    },
    signature::{FunctionDocumentationBuilder, FunctionParameter, SignatureBuilder},
    table_function::{
        BindFunctionHandle, ExecColumnInfo, InitColumnData, TableFunctionBuilder,
        TableFunctionCallbacks,
    },
};

use crate::{
    CATEGORY,
    copy::{DELIMITER, FORMAT_NAME, QuackReader},
};

/// The table function the replacement scan calls.
pub const READ_FUNCTION: &str = "read_quackfile";

/// What `FROM quack` reads instead.
pub const QUACK_QUERY: &str = "SELECT 'quack' AS quack";

struct QuackScan;

impl ReplacementScanCallbacks for QuackScan {
    fn scan(
        &self,
        _context: &Context,
        name: &QualifiedName,
        handle: ReplacementHandle<'_>,
    ) -> Result<()> {
        let name = name.get_view()?;
        if name.catalog.is_none()
            && name.schema.is_none()
            && name.table.eq_ignore_ascii_case("quack")
        {
            handle.set_reference(ReplacementType::Subquery(QUACK_QUERY.into()))?;
        }
        Ok(())
    }
}

struct QuackfileScan;

impl ReplacementScanCallbacks for QuackfileScan {
    /// Runs while binding, for each name the catalog cannot resolve; returning without
    /// `set_reference` declines it. Arguments can only follow a table function reference.
    fn scan(
        &self,
        context: &Context,
        name: &QualifiedName,
        handle: ReplacementHandle<'_>,
    ) -> Result<()> {
        let name = name.get_view()?;
        let suffix = format!(".{FORMAT_NAME}");
        if name.catalog.is_some()
            || name.schema.is_some()
            || !name.table.to_ascii_lowercase().ends_with(&suffix)
        {
            return Ok(());
        }
        handle.set_reference(ReplacementType::Table(READ_FUNCTION.try_into()?))?;
        handle.add_parameter(name.table.value(context)?)
    }
}

struct ReadQuackfile;

struct ReadQuackfileBind {
    path: String,
    columns: usize,
}

impl TableFunctionCallbacks for ReadQuackfile {
    type BindData = ReadQuackfileBind;
    /// Shared by the scan's single worker; here the open file.
    type GlobalState = Mutex<QuackReader>;
    type LocalState = ();

    /// Runs once when the query is bound. The columns must be known here, so the
    /// first line is read now and the file read again from the start in `init_global_state`.
    fn bind(
        &self,
        context: &Context,
        arguments: Vec<BindArgument>,
        bind_handle: BindFunctionHandle<'_>,
    ) -> Result<Self::BindData> {
        let path = arguments
            .first()
            .and_then(|argument| argument.value.as_ref())
            .map(|value| value.get::<String>())
            .transpose()?
            .flatten()
            .ok_or_else(|| {
                Error::invalid_input(format!("{READ_FUNCTION}: path must not be NULL"))
            })?;

        // An empty file has one column and no rows, as with `read_csv`.
        let columns = QuackReader::open(context, &path)?
            .next_line()?
            .map_or(1, |line| line.split(DELIMITER).count());

        let width = (columns - 1).to_string().len();
        for column in 0..columns {
            bind_handle.add_result_column(
                &format!("column{column:0width$}"),
                &String::logical_type(context)?,
            )?;
        }
        Ok(ReadQuackfileBind { path, columns })
    }

    fn init_global_state(
        &self,
        context: &Context,
        bind_data: &Self::BindData,
        _column_data: InitColumnData<'_>,
    ) -> Result<Option<Self::GlobalState>> {
        let reader = QuackReader::open(context, &bind_data.path)?;
        Ok(Some(Mutex::new(reader)))
    }

    fn max_threads(
        &self,
        _context: &Context,
        _bind_data: &Self::BindData,
        _global_state: Option<&Self::GlobalState>,
        _column_data: InitColumnData<'_>,
    ) -> Result<Option<NonZeroUsize>> {
        Ok(NonZeroUsize::new(1))
    }

    /// Runs until the output chunk stays empty. Every column is VARCHAR, so the
    /// decoded values are written as they are, without the cast `COPY ... FROM` needs.
    fn exec(
        &self,
        _context: &Context,
        bind_data: &Self::BindData,
        _local_state: Option<&mut Self::LocalState>,
        global_state: Option<&Self::GlobalState>,
        mut output: DataChunkRef<'_>,
        _column_info: ExecColumnInfo<'_>,
    ) -> Result<()> {
        let rows = global_state
            .ok_or_else(|| Error::internal(format!("{READ_FUNCTION}: global state missing")))?
            .lock()
            .unwrap()
            .next_rows(output.capacity()?, bind_data.columns)?;

        for column in 0..bind_data.columns {
            let mut vector = output.get_vector_at_mut::<String>(column)?;
            vector.set_size(rows.len())?;
            let mut writer = vector.writer()?;
            for (row, fields) in rows.iter().enumerate() {
                writer.set_option(row, fields[column].as_deref())?;
            }
        }
        Ok(())
    }
}

pub(crate) fn register(extension: &Extension) -> Result<()> {
    TableFunctionBuilder::new(
        READ_FUNCTION,
        SignatureBuilder::new()
            .param(FunctionParameter::new::<String>("path"))
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description("Read a quackfile as VARCHAR columns column0, column1, ...; FROM 'path.quackfile' calls it too.")
                    .add_example("By name", "SELECT * FROM read_quackfile('ducks.quackfile')")
                    .add_example("By path", "SELECT * FROM 'ducks.quackfile'")
                    .add_category(CATEGORY),
            ),
        ReadQuackfile,
    )
    .register(extension)?;
    ReplacementScanBuilder::new(QuackfileScan).register(extension)?;
    ReplacementScanBuilder::new(QuackScan).register(extension)
}
