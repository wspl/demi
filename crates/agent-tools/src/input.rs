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
    /// Was `background` until 0.1.61; transcripts written before keep that
    /// name, so a replayed call still reads.
    #[serde(default, alias = "background")]
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_schema_requires_a_title_first_and_offers_service_and_nothing_else() {
        let shell = schema::<ShellInput>();
        assert_eq!(shell["additionalProperties"], false);
        assert_eq!(shell["required"], json!(["description", "script"]));
        let properties = &shell["properties"];
        // The title leads: serde_json keeps the order (`preserve_order`), so
        // the schema a provider serializes lists it first too.
        let order: Vec<&String> = properties.as_object().unwrap().keys().collect();
        assert_eq!(order, ["description", "script", "service"]);
        assert_eq!(properties["service"]["type"], "boolean");
        assert_eq!(
            properties["description"],
            json!({"type": "string", "minLength": 1, "description": DESCRIPTION})
        );
        assert!(!shell.contains_key("$schema") && !shell.contains_key("title"));
    }

    #[test]
    fn a_refusal_names_the_tool_and_the_offending_field() {
        let refusal = |input: Value| parse::<ShellInput>("shell", input).unwrap_err();
        for (input, field) in [
            (
                json!({"script": "true", "description": "Run", "intervalMs": 20_000}),
                "unknown field `intervalMs`",
            ),
            (
                json!({"script": "true", "description": "Run", "service": "yes"}),
                "service: invalid type: string",
            ),
            // A step that starts work names it for the user.
            (json!({"script": "true"}), "missing field `description`"),
            (
                json!({"script": "true", "description": ""}),
                "description: must not be empty",
            ),
            (json!("not json"), "invalid type: string"),
        ] {
            let text = refusal(input);
            assert!(text.starts_with("shell input is invalid:\n"), "{text}");
            assert!(text.contains(field), "{text}");
        }
        let plain: ShellInput = parse("shell", json!({"script": "true", "description": "Run"})).unwrap();
        assert!(!plain.service);
        let server: ShellInput = parse(
            "shell",
            json!({"script": "npm run dev", "service": true, "description": "Start the dev server"}),
        )
        .unwrap();
        assert!(server.service);
    }
}
