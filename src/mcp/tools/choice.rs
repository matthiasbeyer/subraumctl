//! Reuse CLI enum names in both JSON schemas and argument validation.

use std::borrow::Cow;

use clap::ValueEnum;
use rmcp::schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer};
use serde_json::json;

pub(super) struct Choice<T>(pub T);

impl<'de, T: ValueEnum> Deserialize<'de> for Choice<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        T::from_str(&name, false)
            .map(Self)
            .map_err(|_| serde::de::Error::custom("unknown choice"))
    }
}

impl<T: ValueEnum + 'static> JsonSchema for Choice<T> {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        std::any::type_name::<T>().into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let names: Vec<_> = T::value_variants()
            .iter()
            .filter_map(|value| {
                value
                    .to_possible_value()
                    .map(|value| value.get_name().to_string())
            })
            .collect();
        json!({"type": "string", "enum": names})
            .try_into()
            .expect("valid enum schema")
    }
}
