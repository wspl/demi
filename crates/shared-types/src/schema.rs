//! A marker that makes a type's JSON Schema say what serde does.

use std::{borrow::Cow, marker::PhantomData};

use schemars::{JsonSchema, Schema, SchemaGenerator};

/// The schema of a nullable field, which is always written and holds `T` or
/// `null`. serde refuses its absence (`Option::deserialize`), but schemars
/// treats every `Option` as one it may omit, so such a field declares
/// `#[schemars(with = "Nullable<T>")]`, which is required and admits `null`.
/// No value of this type exists.
pub struct Nullable<T>(PhantomData<T>);

impl<T: JsonSchema> JsonSchema for Nullable<T> {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        <Option<T>>::schema_name()
    }

    fn schema_id() -> Cow<'static, str> {
        <Option<T>>::schema_id()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        <Option<T>>::json_schema(generator)
    }
}
