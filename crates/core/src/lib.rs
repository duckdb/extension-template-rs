use std::{
    any::Any,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use duckdb_neo::{
    DuckDBType, FromValue, Result,
    extension::Extension,
    scalar::{ScalarCallbacks, ScalarFunctionBuilder, ScalarInitCallbacks},
    signature::{FunctionDocumentationBuilder, FunctionParameter, SignatureBuilder},
    table_function::{TableFunctionBuilder, TableFunctionCallbacks},
    types::List,
};

struct EchoScalar {
    prefix: String,
}

impl ScalarInitCallbacks for EchoScalar {
    fn init(
        &self,
        _bind_data: Option<&Self::BindData>,
        _context: &duckdb_neo::connection::Context,
    ) -> Result<Self::InitData> {
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
        _context: &duckdb_neo::connection::Context,
        vectors: &mut duckdb_neo::data_chunk::VectorCollection,
        output: duckdb_neo::vector::Vector<'_, duckdb_neo::vector::Unknown>,
    ) -> Result<()> {
        // Type case since we can overwrite the output type at bind time.
        let mut output = output.cast::<String>()?;
        output.set_size(vectors.row_count()?)?;

        let prefix = &self.prefix;
        let postfix = init_data.unwrap();

        let vector = vectors.get_vector_at::<String>(0)?;
        let reader = vector.reader()?;
        let mut writer = output.writer()?;

        for (i, row) in reader.iter().enumerate() {
            match row {
                Some(row) => writer.set(i, Some(&format!("{prefix} {row} {postfix} {row}"))),
                None => writer.set(i, None),
            }?
        }
        Ok(())
    }
}

struct EchoTable {
    icons: Vec<String>,
}

impl TableFunctionCallbacks for EchoTable {
    type GlobalState = AtomicU64;
    type LocalState = String;
    type BindData = Mutex<List<String>>;

    fn init_global_state(
        &self,
        _bind_data: &Self::BindData,
        _context: &duckdb_neo::connection::Context,
        _column_data: duckdb_neo::table_function::InitColumnData<'_>,
    ) -> Result<(Option<Self::GlobalState>, Option<usize>)> {
        Ok((Some(0.into()), None))
    }

    fn init_local_state(
        &self,
        _bind_data: &Self::BindData,
        _global_state: Option<&Self::GlobalState>,
        _context: &duckdb_neo::connection::Context,
        _column_data: duckdb_neo::table_function::InitColumnData<'_>,
    ) -> Result<Option<Self::LocalState>> {
        Ok(Some("".into()))
    }

    fn bind(
        &self,
        context: &duckdb_neo::connection::Context,
        arguments: Vec<duckdb_neo::bind_arguments::BindArgument>,
        bind_handle: duckdb_neo::table_function::BindFunctionHandle<'_>,
    ) -> Result<Self::BindData> {
        bind_handle.add_result_column("name", &String::logical_type(context)?)?;
        bind_handle.add_result_column("icon", &String::logical_type(context)?)?;
        bind_handle.add_result_column("concat_name", &String::logical_type(context)?)?;
        bind_handle.add_result_column("total_index", &u64::logical_type(context)?)?;

        let arg = arguments.first().unwrap();

        let value = arg.value.as_ref();

        assert_eq!(
            arg.logical_type.type_id(),
            <List<String>>::logical_type(&context)?.type_id()
        );

        let items = <List<String>>::from_value(value.unwrap())?.unwrap_or_default();

        Ok(Mutex::new(items))
    }

    fn exec(
        &self,
        bind_data: &Self::BindData,
        global_state: Option<&Self::GlobalState>,
        local_state: Option<&mut Self::LocalState>,
        _context: &duckdb_neo::connection::Context,
        mut output: duckdb_neo::data_chunk::DataChunkRef<'_>,
        _column_info: duckdb_neo::table_function::ExecColumnInfo<'_>,
    ) -> Result<()> {
        let [name, icon, concat_name, total_index] = output.get_vector_array_mut([0, 1, 2, 3])?;
        let mut name = name.cast::<String>()?;
        let mut icon = icon.cast::<String>()?;
        let mut concat_name = concat_name.cast::<String>()?;
        let mut total_index = total_index.cast::<u64>()?;

        let bind_data_len = bind_data.lock().unwrap().0.len();

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

        while let Some(name) = bind_data.lock().unwrap().0.pop() {
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
    ScalarFunctionBuilder::new(
        "rusty_echo",
        SignatureBuilder::new()
            .param(FunctionParameter::new::<String>("input"))
            .return_type(String::logical_type(extension)?)
            .documentation(
                FunctionDocumentationBuilder::new()
                    .add_category("example")
                    .add_example("echo", "echo('hello')")
                    .description("Echo back the input you provided, but with a rusty twist!"),
            ),
        EchoScalar {
            prefix: "🐤".to_string(),
        },
    )
    .with_init()
    .register(extension)?;

    TableFunctionBuilder::new(
        "rusty_quack",
        SignatureBuilder::new().param(FunctionParameter::new::<List<String>>("names")),
        EchoTable {
            icons: vec!["🐤".into(), "🦀".into(), "🐥".into()],
        },
    )
    .register(extension)?;

    Ok(())
}
