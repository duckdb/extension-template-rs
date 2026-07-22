use duckdb::{
    Connection, Result,
    core::{DataChunkHandle, Inserter, LogicalTypeHandle, LogicalTypeId},
    duckdb_entrypoint_c_api,
    ffi::duckdb_string_t,
    types::DuckString,
    vscalar::{ScalarFunctionSignature, VScalar},
    vtab::{BindInfo, InitInfo, TableFunctionInfo, VTab, arrow::WritableVector},
};
use std::{
    error::Error,
    ffi::CString,
    sync::atomic::{AtomicBool, Ordering},
};

struct EchoScalar;

impl VScalar for EchoScalar {
    type State = ();

    fn invoke(
        _state: &Self::State,
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let input_vec = input.flat_vector(0);
        let values = unsafe { input_vec.as_slice_with_len::<duckdb_string_t>(input.len()) };
        let mut output = output.flat_vector();

        for (i, value) in values.iter().enumerate() {
            if input_vec.row_is_null(i as u64) {
                output.set_null(i);
                continue;
            }

            let mut value = *value;
            let s = DuckString::new(&mut value).as_str();
            output.insert(i, format!("🐤 {s} 🦀 {s}").as_str());
        }
        Ok(())
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        vec![ScalarFunctionSignature::exact(
            vec![LogicalTypeId::Varchar.into()],
            LogicalTypeId::Varchar.into(),
        )]
    }
}

#[repr(C)]
struct HelloBindData {
    name: String,
}

#[repr(C)]
struct HelloInitData {
    done: AtomicBool,
}

struct HelloVTab;

impl VTab for HelloVTab {
    type InitData = HelloInitData;
    type BindData = HelloBindData;

    fn bind(bind: &BindInfo) -> Result<Self::BindData, Box<dyn std::error::Error>> {
        bind.add_result_column("column0", LogicalTypeHandle::from(LogicalTypeId::Varchar));
        let name = bind.get_parameter(0).to_string();
        Ok(HelloBindData { name })
    }

    fn init(_: &InitInfo) -> Result<Self::InitData, Box<dyn std::error::Error>> {
        Ok(HelloInitData {
            done: AtomicBool::new(false),
        })
    }

    fn func(
        func: &TableFunctionInfo<Self>,
        output: &mut DataChunkHandle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let init_data = func.get_init_data();
        let bind_data = func.get_bind_data();
        if init_data.done.swap(true, Ordering::Relaxed) {
            output.set_len(0);
        } else {
            let vector = output.flat_vector(0);
            let result = CString::new(format!("Rusty Quack {} 🐥", bind_data.name))?;
            vector.insert(0, result);
            output.set_len(1);
        }
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
    }
}

#[duckdb_entrypoint_c_api]
pub unsafe fn extension_entrypoint(con: Connection) -> Result<(), Box<dyn Error>> {
    con.register_scalar_function::<EchoScalar>("rusty_echo")?;
    con.register_table_function::<HelloVTab>("rusty_quack")?;

    Ok(())
}
