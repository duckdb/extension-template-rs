//! A table function with state: `rusty_quack(names)`.
//!
//! [`TableFunctionCallbacks`] is the lifecycle DuckDB runs for a table
//! function. DuckDB calls back in this order:
//!
//! 1. `bind` runs once per call site in a query, reads the constant arguments
//!    and declares the result columns.
//! 2. `init_global_state` runs once per scan and creates state shared by all
//!    workers; here it hands out icons.
//! 3. `init_local_state` runs once per worker; here it accumulates the names
//!    a worker has seen.
//! 4. `exec` runs repeatedly, possibly on several workers, until a call leaves
//!    the output chunk empty.
//!
//! This is the template's original example; see [`crate::sequence`] for a
//! function that emits more than one chunk.

use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

use duckdb_neo::{
    DuckDBType, FromValue, Result,
    bind_arguments::BindArgument,
    connection::Context,
    data_chunk::DataChunkRef,
    error::Error,
    extension::Extension,
    signature::{FunctionDocumentationBuilder, FunctionParameter, SignatureBuilder},
    table_function::{
        BindFunctionHandle, ExecColumnInfo, InitColumnData, TableFunctionBuilder,
        TableFunctionCallbacks,
    },
    types::List,
};

use crate::CATEGORY;

/// The registered implementation; the icons are fixed at registration time.
struct EchoTable {
    icons: Vec<String>,
}

impl TableFunctionCallbacks for EchoTable {
    /// Shared by every worker of one scan; here a counter that picks the next icon.
    type GlobalState = AtomicU64;
    /// Owned by one worker; here the names it has emitted so far, joined with `|`.
    type LocalState = String;
    /// Whatever `bind` computes once per call site; here the remaining names.
    /// A `Mutex` because bind data is shared and `exec` pops from it.
    type BindData = Mutex<Vec<Option<String>>>;

    /// Runs once per scan, after `bind`. Typical work:
    /// - Create state every worker shares, such as a cursor or a counter.
    /// - Return a thread limit as the second value, or `None` for DuckDB's default.
    fn init_global_state(
        &self,
        _bind_data: &Self::BindData,
        _context: &Context,
        _column_data: InitColumnData<'_>,
    ) -> Result<(Option<Self::GlobalState>, Option<usize>)> {
        Ok((Some(0.into()), None))
    }

    /// Runs once per worker, after `init_global_state`. Typical work:
    /// - Create per-worker scratch space, such as buffers or a file handle.
    /// - Return `None` when the worker needs nothing of its own.
    fn init_local_state(
        &self,
        _bind_data: &Self::BindData,
        _global_state: Option<&Self::GlobalState>,
        _context: &Context,
        _column_data: InitColumnData<'_>,
    ) -> Result<Option<Self::LocalState>> {
        Ok(Some("".into()))
    }

    /// Runs once when the query is bound. Typical work:
    /// - Declare the result columns with `add_result_column`, in the order `exec` fills them.
    /// - Validate the argument types and constant values, returning an error for what
    ///   the function cannot handle.
    /// - Return as `BindData` whatever the later callbacks need.
    fn bind(
        &self,
        context: &Context,
        arguments: Vec<BindArgument>,
        bind_handle: BindFunctionHandle<'_>,
    ) -> Result<Self::BindData> {
        bind_handle.add_result_column("name", &String::logical_type(context)?)?;
        bind_handle.add_result_column("icon", &String::logical_type(context)?)?;
        bind_handle.add_result_column("concat_name", &String::logical_type(context)?)?;
        bind_handle.add_result_column("total_index", &u64::logical_type(context)?)?;

        let arg = arguments
            .first()
            .ok_or_else(|| Error::invalid_input("rusty_quack expects one argument"))?;

        // A bind callback reports problems by returning an error; a panic would be an internal error instead.
        if arg.logical_type.id()? != <List<String>>::logical_type(context)?.id()? {
            return Err(Error::invalid_input(
                "rusty_quack expects a VARCHAR[] argument",
            ));
        }

        let items = match &arg.value {
            // `List<T>` wraps `Vec<Option<T>>`; a NULL list binds as an empty one.
            Some(value) => <List<String>>::from_value(value)?.unwrap_or_default().0,
            None => Vec::new(),
        };

        Ok(Mutex::new(items))
    }

    /// Runs until the output chunk stays empty. Typical work:
    /// - Take the output vectors, narrow each to its declared type, and size them.
    /// - Fill rows from the bind data and the states, `None` for NULL.
    /// - Leave the chunk empty (or `set_size(0)`) once there is nothing left.
    ///
    /// Here one call emits every remaining name, so the second call ends the scan.
    fn exec(
        &self,
        bind_data: &Self::BindData,
        global_state: Option<&Self::GlobalState>,
        local_state: Option<&mut Self::LocalState>,
        _context: &Context,
        mut output: DataChunkRef<'_>,
        _column_info: ExecColumnInfo<'_>,
    ) -> Result<()> {
        let [name, icon, concat_name, total_index] = output.get_vector_array_mut([0, 1, 2, 3])?;
        let mut name = name.cast::<String>()?;
        let mut icon = icon.cast::<String>()?;
        let mut concat_name = concat_name.cast::<String>()?;
        let mut total_index = total_index.cast::<u64>()?;

        let bind_data_len = bind_data.lock().unwrap().len();

        name.set_size(bind_data_len)?;
        icon.set_size(bind_data_len)?;
        concat_name.set_size(bind_data_len)?;
        total_index.set_size(bind_data_len)?;

        let mut name_writer = name.writer()?;
        let mut icon_writer = icon.writer()?;
        let mut concat_name_writer = concat_name.writer()?;
        let mut total_index_writer = total_index.writer()?;

        let mut index = 0;
        let local_concat = local_state.unwrap();

        while let Some(name) = bind_data.lock().unwrap().pop() {
            match name {
                None => {
                    name_writer.set(index, None)?;
                    icon_writer.set(index, None)?;
                    concat_name_writer.set(index, None)?;
                    total_index_writer.set(index, None)?;
                }
                Some(name) => {
                    let icon_index = global_state.unwrap().fetch_add(1, Ordering::AcqRel);

                    let icon = &self.icons[icon_index as usize % self.icons.len()];

                    local_concat.push_str(&name);
                    local_concat.push('|');

                    name_writer.set(index, Some(&name))?;
                    icon_writer.set(index, Some(icon))?;
                    concat_name_writer.set(index, Some(local_concat))?;
                    total_index_writer.set(index, Some(index as u64))?;
                }
            }
            index += 1;
        }

        Ok(())
    }
}

pub fn register(extension: &Extension) -> Result<()> {
    TableFunctionBuilder::new(
        "rusty_quack",
        SignatureBuilder::new()
            .param(FunctionParameter::new::<List<String>>("names"))
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description("One row per name, last name first, with a rotating icon and the names seen so far.")
                    .add_example("Four names", "SELECT * FROM rusty_quack(['Jane', 'Doe', 'Loves', 'Ducks'])")
                    .add_category(CATEGORY),
            ),
        EchoTable {
            icons: vec!["🐤".into(), "🦀".into(), "🐥".into()],
        },
    )
    .register(extension)
}
