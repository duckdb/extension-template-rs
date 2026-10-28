//! A table function with state: `rusty_quack(names)`.
//!
//! [`TableFunctionCallbacks::bind`] reads a constant `VARCHAR[]` argument and
//! declares the result columns. Global state (one per scan, shared by all
//! workers) hands out icons; local state (one per worker) accumulates the
//! names a worker has seen. This is the template's original example; see
//! [`crate::sequence`] for a function that emits more than one chunk.

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
    table_function::{BindFunctionHandle, ExecColumnInfo, InitColumnData, TableFunctionBuilder, TableFunctionCallbacks},
};

use crate::CATEGORY;

struct EchoTable {
    icons: Vec<String>,
}

impl TableFunctionCallbacks for EchoTable {
    type GlobalState = AtomicU64;
    type LocalState = String;
    type BindData = Mutex<Vec<Option<String>>>;

    fn init_global_state(
        &self,
        _bind_data: &Self::BindData,
        _context: &Context,
        _column_data: InitColumnData<'_>,
    ) -> Result<(Option<Self::GlobalState>, Option<usize>)> {
        Ok((Some(0.into()), None))
    }

    fn init_local_state(
        &self,
        _bind_data: &Self::BindData,
        _global_state: Option<&Self::GlobalState>,
        _context: &Context,
        _column_data: InitColumnData<'_>,
    ) -> Result<Option<Self::LocalState>> {
        Ok(Some("".into()))
    }

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
        if arg.logical_type.id()? != <Vec<String>>::logical_type(context)?.id()? {
            return Err(Error::invalid_input("rusty_quack expects a VARCHAR[] argument"));
        }

        let items = match &arg.value {
            Some(value) => <Vec<Option<String>>>::from_value(value)?.unwrap_or_default(),
            None => Vec::new(),
        };

        Ok(Mutex::new(items))
    }

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
            .param(FunctionParameter::new::<Vec<String>>("names"))
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
