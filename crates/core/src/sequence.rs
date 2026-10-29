//! A chunked table function with optimizer hooks: `rusty_sequence(count, step := 1)`.
//!
//! DuckDB calls [`TableFunctionCallbacks::exec`] repeatedly, possibly from
//! several workers at once, until a call produces zero rows. A cursor in the
//! global state hands each call its next slice. On top of that this module
//! opts into:
//!
//! - projection pushdown: `exec` only fills the columns the query uses
//!   ([`ExecColumnInfo`] maps output positions to declared columns);
//! - filter pushdown: `i < constant` is claimed in
//!   [`TableFilterPushdownCallbacks::pushdown_filter`] by inspecting the bound
//!   [`Expression`](duckdb_neo::expression::Expression), and enforced in `exec` so DuckDB drops its own filter;
//! - an exact cardinality and a progress fraction.

use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

use duckdb_neo::{
    DuckDBType, Result,
    bind_arguments::BindArgument,
    connection::Context,
    data_chunk::DataChunkRef,
    error::Error,
    expression::ExpressionType,
    extension::Extension,
    signature::{FunctionDocumentationBuilder, FunctionParameter, SignatureBuilder},
    table_function::{
        BindFunctionHandle, ExecColumnInfo, InitColumnData, PushdownData, TableFilterPushdownCallbacks,
        TableFunctionBuilder, TableFunctionCallbacks,
    },
};

use crate::CATEGORY;

/// Declared column positions, as given to `add_result_column`.
const COLUMN_I: usize = 0;
const COLUMN_LABEL: usize = 1;

/// The registered implementation; it has no fields, so all state lives in bind data and global state.
struct Sequence;

/// What `bind` computes once per call site; every later callback receives it.
struct SequenceBind {
    count: u64,
    step: i64,
    /// An accepted `i < limit` filter; rows at or above it are never emitted.
    limit: Mutex<Option<i64>>,
}

impl SequenceBind {
    /// `bind` checked that the last row's value fits in an `i64`, so no row can overflow here.
    fn value_at(&self, row: u64) -> i64 {
        row as i64 * self.step
    }

    fn passes(&self, value: i64, limit: Option<i64>) -> bool {
        limit.is_none_or(|limit| value < limit)
    }
}

/// Read argument `index` as `T`, treating SQL NULL as `None`.
fn argument<T: duckdb_neo::FromValue>(arguments: &[BindArgument], index: usize) -> Result<Option<T>> {
    match arguments.get(index).and_then(|argument| argument.value.as_ref()) {
        Some(value) => value.get::<T>(),
        None => Ok(None),
    }
}

impl TableFunctionCallbacks for Sequence {
    type BindData = SequenceBind;
    /// The next row to hand out, shared by every worker of the scan.
    type GlobalState = AtomicU64;
    /// Nothing per worker: every call claims its slice from the global cursor.
    type LocalState = ();

    /// Runs once when the query is bound. Typical work:
    /// - Read and validate the constant arguments; reject what the function cannot handle.
    /// - Declare the result columns with `add_result_column`, in the order the column
    ///   constants above use.
    /// - Tell the optimizer what is known up front, such as an exact cardinality.
    fn bind(
        &self,
        context: &Context,
        arguments: Vec<BindArgument>,
        bind_handle: BindFunctionHandle<'_>,
    ) -> Result<Self::BindData> {
        let count = match argument::<i64>(&arguments, 0)? {
            None => 0,
            Some(count) if count < 0 => return Err(Error::invalid_input("rusty_sequence: count must not be negative")),
            Some(count) => count as u64,
        };
        let step = argument::<i64>(&arguments, 1)?.unwrap_or(1);
        if step == 0 {
            return Err(Error::invalid_input("rusty_sequence: step must not be zero"));
        }
        // Every value is `row * step` with `row < count`, so the last row bounds them all.
        if count > 0 && (count - 1).checked_mul(step.unsigned_abs()).is_none_or(|last| last > i64::MAX as u64) {
            return Err(Error::invalid_input("rusty_sequence: count * step does not fit in a BIGINT"));
        }

        bind_handle.add_result_column("i", &i64::logical_type(context)?)?;
        bind_handle.add_result_column("label", &String::logical_type(context)?)?;
        bind_handle.set_cardinality(count as usize, true)?;

        Ok(SequenceBind {
            count,
            step,
            limit: Mutex::new(None),
        })
    }

    /// Runs once per scan. Typical work:
    /// - Create the state every worker shares; here the cursor, an atomic so workers
    ///   can claim slices without a lock.
    /// - Return a thread limit as the second value, or `None` for DuckDB's default.
    fn init_global_state(
        &self,
        _bind_data: &Self::BindData,
        _context: &Context,
        _column_data: InitColumnData<'_>,
    ) -> Result<(Option<Self::GlobalState>, Option<usize>)> {
        Ok((Some(AtomicU64::new(0)), None))
    }

    /// Runs once per worker. This function needs no per-worker state, so it returns `None`.
    fn init_local_state(
        &self,
        _bind_data: &Self::BindData,
        _global_state: Option<&Self::GlobalState>,
        _context: &Context,
        _column_data: InitColumnData<'_>,
    ) -> Result<Option<Self::LocalState>> {
        Ok(None)
    }

    /// Runs whenever DuckDB asks how far the scan is, only when registered with
    /// `with_progress`. Typical work:
    /// - Compare the shared cursor with the total, and return a fraction from `0.0` to `1.0`.
    /// - Return `None` when progress cannot be estimated.
    fn progress(
        &self,
        bind_data: &Self::BindData,
        global_state: Option<&Self::GlobalState>,
        _context: &Context,
    ) -> Result<Option<f64>> {
        let Some(cursor) = global_state else {
            return Ok(None);
        };
        if bind_data.count == 0 {
            return Ok(Some(1.0));
        }
        let done = cursor.load(Ordering::Relaxed).min(bind_data.count);
        Ok(Some(done as f64 / bind_data.count as f64))
    }

    /// Runs until the output chunk stays empty, possibly on several workers at once.
    /// Typical work:
    /// - Claim the next slice of work from the global state.
    /// - Apply the filters accepted in `pushdown_filter`, so no emitted row violates them.
    /// - Fill only the columns in `columns`: with projection pushdown, output position `i`
    ///   maps to declared column `columns.get_column_index(i)`.
    /// - Leave the chunk empty once there is nothing left; an empty chunk ends the scan,
    ///   so a slice that filters down to nothing must not be returned as-is.
    fn exec(
        &self,
        bind_data: &Self::BindData,
        global_state: Option<&Self::GlobalState>,
        _local_state: Option<&mut Self::LocalState>,
        _context: &Context,
        mut output: DataChunkRef<'_>,
        columns: ExecColumnInfo<'_>,
    ) -> Result<()> {
        let cursor = global_state.ok_or_else(|| Error::internal("rusty_sequence: missing global state"))?;
        let capacity = output.capacity()? as u64;
        let limit = *bind_data.limit.lock().unwrap();

        // Claim slices until one yields rows, because an empty chunk ends the scan.
        let rows: Vec<u64> = loop {
            let start = cursor.fetch_add(capacity, Ordering::SeqCst);
            if start >= bind_data.count {
                break Vec::new();
            }
            let end = (start + capacity).min(bind_data.count);
            let rows: Vec<u64> = (start..end)
                .filter(|&row| bind_data.passes(bind_data.value_at(row), limit))
                .collect();
            if !rows.is_empty() {
                break rows;
            }
        };

        for position in 0..columns.count()? {
            match columns.get_column_index(position)? {
                COLUMN_I => {
                    let mut vector = output.get_vector_at_mut::<i64>(position)?;
                    vector.set_size(rows.len())?;
                    let mut writer = vector.writer()?;
                    for (index, &row) in rows.iter().enumerate() {
                        writer.set(index, Some(bind_data.value_at(row)))?;
                    }
                }
                COLUMN_LABEL => {
                    let mut vector = output.get_vector_at_mut::<String>(position)?;
                    vector.set_size(rows.len())?;
                    let mut writer = vector.writer()?;
                    for (index, &row) in rows.iter().enumerate() {
                        writer.set(index, Some(&format!("row {}", bind_data.value_at(row))))?;
                    }
                }
                other => return Err(Error::internal(format!("rusty_sequence: unknown column {other}"))),
            }
        }
        Ok(())
    }
}

impl TableFilterPushdownCallbacks for Sequence {
    /// Runs once per scan, between `bind` and `init_global_state`, only when registered with
    /// `with_filter_pushdown`. Typical work:
    /// - Walk the filter expressions and inspect their shape (comparison, column reference,
    ///   constant).
    /// - Record the ones the function will enforce in the bind data (hence the `Mutex`),
    ///   and call `accept_pushdown` so DuckDB drops its own copy.
    /// - Leave everything else unclaimed; DuckDB still applies it after the scan.
    ///
    /// Here only `i < constant` is claimed.
    fn pushdown_filter(
        &self,
        bind_data: &Self::BindData,
        _context: &Context,
        pushdown_data: PushdownData<'_>,
    ) -> Result<()> {
        for index in 0..pushdown_data.filter_count()? {
            let filter = pushdown_data.filter(index)?;
            if filter.expression_type()? != ExpressionType::CompareLessThan || filter.child_count()? != 2 {
                continue;
            }
            let (column, constant) = (filter.child(0)?, filter.child(1)?);
            if column.expression_type()? != ExpressionType::BoundColumnRef
                || constant.expression_type()? != ExpressionType::ValueConstant
            {
                continue;
            }
            // The column reference indexes the scan's projected columns.
            if pushdown_data.get_column_index(column.reference_index()?)? != COLUMN_I {
                continue;
            }
            let Some(limit) = constant.get_constant_value()?.get::<i64>()? else {
                continue;
            };
            let mut accepted = bind_data.limit.lock().unwrap();
            *accepted = Some(accepted.map_or(limit, |current| current.min(limit)));
            pushdown_data.accept_pushdown(index)?;
        }
        Ok(())
    }
}

pub fn register(extension: &Extension) -> Result<()> {
    TableFunctionBuilder::new(
        "rusty_sequence",
        SignatureBuilder::new()
            .param(FunctionParameter::new::<i64>("count"))
            .param(FunctionParameter::with_default("step", 1_i64))
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description("count rows of i = 0, step, 2*step, ... with a label; filters on i < constant are pushed into the scan.")
                    .add_example("First ten rows", "SELECT i FROM rusty_sequence(10000) WHERE i < 10")
                    .add_example("With a step", "SELECT * FROM rusty_sequence(3, step := 10)")
                    .add_category(CATEGORY),
            ),
        Sequence,
    )
    .with_projection_pushdown(true)
    .with_filter_pushdown()
    .with_progress()
    .register(extension)
}
