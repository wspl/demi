//! Query parameters. Queries are read by the backend alone and are not part of
//! the generated TypeScript; the integration API's document describes those
//! it shares with the web app.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, de};

/// A boolean query parameter: spelled exactly `true` or `false`, and false
/// when omitted. Anything else, such as `1`, `TRUE` or an empty value, is
/// refused rather than read as one of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StrictBool(pub bool);

impl<'de> Deserialize<'de> for StrictBool {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = std::borrow::Cow::<'de, str>::deserialize(deserializer)?;
        match text.as_ref() {
            "true" => Ok(Self(true)),
            "false" => Ok(Self(false)),
            other => Err(de::Error::custom(format!(
                "must be true or false, not {other:?}"
            ))),
        }
    }
}

impl JsonSchema for StrictBool {
    fn schema_name() -> Cow<'static, str> {
        "StrictBool".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "boolean", "default": false })
    }
}

/// `?refresh=true|false`.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Refresh {
    #[serde(default)]
    pub refresh: StrictBool,
}

/// `?q=<query>&entry=<id>` of `GET /models/search`: the words to match,
/// and the entry whose vendor fills the matches it lists.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelSearch {
    pub q: String,
    #[serde(default)]
    pub entry: Option<crate::ids::ProviderId>,
}
