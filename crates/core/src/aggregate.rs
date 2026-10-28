//! An aggregate function: `rusty_longest(input)`.
//!
//! [`AggregateCallbacks`] is the full lifecycle DuckDB runs for an aggregate.
//! DuckDB owns the memory for one state per group (for `GROUP BY`) or per
//! partition (for window functions) and calls back in this order.
//!
//! 1. `bind` runs once per call site in a query and validates the arguments.
//! 2. `init` runs once per group and fills a fresh state.
//! 3. `update` runs once per input chunk and folds rows into their group's state.
//! 4. `combine` runs when several threads built partial states for the same
//!    groups and merges them.
//! 5. `finalize` runs once per group and turns a state into a result value.
//!
//! Two callbacks keep their defaults here: `size` reports how many bytes a
//! state needs (the size of `StateItem`), and `destroy` lets you clean up
//! states yourself instead of letting Rust drop them.
//!
//! Because ties go to the lexicographically smaller string, the result does
//! not depend on input order, which the properties tell the optimizer.

use duckdb_neo::{
    DuckDBType, Result,
    aggregate::{AggregateCallbacks, AggregateFunctionBuilder, ReturnTypeHandle, States},
    bind_arguments::BindArguments,
    connection::Context,
    data_chunk::VectorCollection,
    enums::AggregateFunctionProperty,
    extension::Extension,
    signature::{FunctionDocumentationBuilder, FunctionParameter, SignatureBuilder},
    vector::{Unknown, Vector},
};

use crate::CATEGORY;

struct Longest;

/// Replace `state` with `candidate` when it is longer, or equally long and lexicographically smaller.
fn keep_longer(state: &mut Option<String>, candidate: &str) {
    let replace = match state {
        None => true,
        Some(current) => {
            let (candidate_len, current_len) = (candidate.chars().count(), current.chars().count());
            candidate_len > current_len || (candidate_len == current_len && candidate < current.as_str())
        }
    };
    if replace {
        *state = Some(candidate.to_owned());
    }
}

impl AggregateCallbacks for Longest {
    /// Whatever `bind` computes once per call site; every later callback receives it.
    type BindData = ();
    /// The per-group accumulator; here the longest string seen so far, `None` until a non-NULL value arrives.
    type StateItem = Option<String>;

    /// Runs once when the query is bound. Typical work:
    /// - Inspect the argument types or constant arguments and reject what the function cannot handle.
    /// - Pick the return type with `result_type` when it depends on the arguments.
    /// - Return as `BindData` whatever every later callback needs, such as parsed options or a lookup table.
    fn bind(
        &self,
        _context: &Context,
        _arguments: BindArguments<'_>,
        _result_type: ReturnTypeHandle<'_>,
    ) -> Result<Self::BindData> {
        Ok(())
    }

    /// Runs once per group to create its empty state. Typical work:
    /// - Return the identity of the aggregation, such as zero counts, `None`, or an empty list.
    ///
    /// It cannot fail, so validation belongs in `bind`.
    fn init(&self, _bind_data: Option<&Self::BindData>) -> Self::StateItem {
        None
    }

    /// Runs once per input chunk. Typical work:
    /// - Read one vector per argument from `data`.
    /// - Skip NULL rows unless the aggregate counts them.
    /// - Fold each row into `states[row]`, the state of the group that row belongs to.
    ///
    /// Several rows of a chunk usually share one state.
    fn update(
        &self,
        _bind_data: Option<&Self::BindData>,
        data: &mut VectorCollection,
        mut states: States<'_, Self::StateItem>,
    ) -> Result<()> {
        let input = data.get_vector_at::<String>(0)?;
        for (row, value) in input.reader()?.iter().enumerate() {
            if let Some(value) = value {
                keep_longer(&mut states[row], value);
            }
        }
        Ok(())
    }

    /// Runs when parallel threads each built a partial state for the same group. Typical work:
    /// - Merge `source[i]` into `target[i]` the way `update` merges a row. Add counts, keep the
    ///   maximum, or append lists.
    ///
    /// The merge must give the same result as if all rows had gone through one `update`.
    fn combine(
        &self,
        _bind_data: Option<&Self::BindData>,
        source: &States<'_, Self::StateItem>,
        mut target: States<'_, Self::StateItem>,
    ) -> Result<()> {
        for index in 0..target.len() {
            if let Some(candidate) = &source[index] {
                keep_longer(&mut target[index], candidate);
            }
        }
        Ok(())
    }

    /// Runs once per group at the end. Typical work:
    /// - Compute the final value from each state, such as a division, a median, or the kept value.
    /// - Write it to row `result_offset + i` of `result`, with `None` for groups that saw no input.
    ///
    /// The offset is non-zero when DuckDB finalizes a batch of groups into the middle of a larger vector.
    fn finalize(
        &self,
        _bind_data: Option<&Self::BindData>,
        states: &[&Self::StateItem],
        result: Vector<'_, Unknown>,
        result_offset: usize,
    ) -> Result<()> {
        let mut result = result.cast::<String>()?;
        let mut writer = result.writer()?;
        for (index, state) in states.iter().enumerate() {
            writer.set(result_offset + index, state.as_deref())?;
        }
        Ok(())
    }
}

pub fn register(extension: &Extension) -> Result<()> {
    AggregateFunctionBuilder::new(
        "rusty_longest",
        SignatureBuilder::new()
            .param(FunctionParameter::new::<String>("input"))
            .return_type(String::logical_type(extension)?)
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description("The longest non-NULL string in the group; ties go to the lexicographically smaller one.")
                    .add_example("Longest name", "SELECT rusty_longest(name) FROM (VALUES ('a'), ('ccc'), ('bb')) t(name)")
                    .add_category(CATEGORY),
            ),
        Longest,
    )
    // Both false: the result is the same for any row order and with or without DISTINCT,
    // so the optimizer may skip sorting and de-duplicating the input.
    .set_property(AggregateFunctionProperty::AggregateOrderDependent(false))
    .set_property(AggregateFunctionProperty::AggregateDistinctDependent(false))
    .register(extension)
}
