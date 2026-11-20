//! A custom setting: `SET quack_emojis = ['🐤', '🦀']`.
//!
//! [`CustomOptionCallbacks::update`] validates every `SET` and `RESET`; the
//! value it returns is what DuckDB stores. [`crate::quack`] reads the setting
//! when a query is bound.

use duckdb_neo::{
    DuckDBType, Error, FromValue, Result, ToValue,
    connection::{Context, SettingScope},
    custom_option::{CustomOptionBuilder, CustomOptionCallbacks},
    extension::Extension,
    types::List,
    value::Value,
};

/// The setting's name, as used in `SET` and `current_setting`.
pub const QUACK_EMOJIS: &str = "quack_emojis";

fn default_emojis() -> List<String> {
    List(vec![
        Some("🐤".into()),
        Some("🦀".into()),
        Some("🐥".into()),
    ])
}

struct QuackEmojisOption;

impl CustomOptionCallbacks for QuackEmojisOption {
    fn update(
        &self,
        context: &Context,
        value: Value,
        _scope: SettingScope,
        is_reset: bool,
    ) -> Result<Value> {
        if is_reset {
            return default_emojis().value(context);
        }

        let emojis = <List<String>>::from_value(&value)?
            .ok_or_else(|| Error::api_error("The emojis must not be NULL"))?;

        if emojis.0.is_empty() {
            return Err(Error::api_error("The emojis must not be empty"));
        }
        if emojis.0.iter().any(Option::is_none) {
            return Err(Error::api_error("The emojis must not contain NULL values"));
        }
        Ok(value)
    }
}

pub(crate) fn register(extension: &Extension) -> Result<()> {
    CustomOptionBuilder::new(
        QUACK_EMOJIS,
        "The list of emojis to use for the quack table function",
        <List<String>>::logical_type(extension)?,
        QuackEmojisOption,
    )
    .default_value(default_emojis().value(extension)?)
    .default_scope(SettingScope::Global)
    .register(extension)
}
