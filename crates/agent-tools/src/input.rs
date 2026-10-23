//! The `shell` tool's input, declared once (`crates/agent-tools/src/lib.rs`): the
//! JSON Schema the model receives and the check a call runs come from the
//! same type.

use schemars::{JsonSchema, generate::SchemaSettings};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// What `service` tells the model.
const SERVICE: &str = "A command that runs until stopped, such as a development server.";

/// What a call's `description` asks of the model: the title the user sees,
/// which the model writes before the step runs.
pub(super) const DESCRIPTION: &str = "The step's title the user sees, such as \"Run the type checker\".";

// `description` comes first: a model writes a call's fields in the schema's
// order, so the title the page shows arrives before a long script.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ShellInput {
    #[schemars(length(min = 1), description = DESCRIPTION)]
    pub(super) description: NonEmpty,
    #[schemars(description = "The script.")]
    pub(super) script: String,
    #[serde(default)]
    #[schemars(description = SERVICE)]
    pub(super) service: bool,
}

// Text a call must not leave empty: the title of a step. (A doc comment
// would become the field's description in the model's schema.)
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(try_from = "String")]
#[schemars(inline)]
pub(super) struct NonEmpty(pub(super) String);

impl TryFrom<String> for NonEmpty {
    type Error = &'static str;

    fn try_from(text: String) -> Result<Self, &'static str> {
        if text.is_empty() {
            return Err("must not be empty");
        }
        Ok(Self(text))
    }
}

/// The model-facing JSON Schema of `T`. A provider embeds it in its own
/// request, where a dialect declaration has no meaning and the Rust type's
/// name is no title, so neither is written.
pub(super) fn schema<T: JsonSchema>() -> Map<String, Value> {
    let schema = SchemaSettings::draft2020_12()
        .with(|settings| settings.meta_schema = None)
        .into_generator()
        .into_root_schema_for::<T>();
    let Value::Object(mut object) = schema.to_value() else {
        unreachable!("a struct's schema is an object")
    };
    object.remove("title");
    object
}

/// Decodes and checks a call's input: every rule is in the type, so the one
/// decode is the whole check. The refusal names the tool and the offending
/// field, so the model can correct the call.
pub(super) fn parse<T: DeserializeOwned>(tool: &str, input: Value) -> Result<T, String> {
    serde_path_to_error::deserialize(input).map_err(|error| {
        let path = error.path().to_string();
        let inner = error.into_inner();
        match path.as_str() {
            "." => format!("{tool} input is invalid:\n{inner}"),
            _ => format!("{tool} input is invalid:\n{path}: {inner}"),
        }
    })
}
