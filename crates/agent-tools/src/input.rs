//! The `shell` tool's input, declared once (`crates/agent-tools/src/lib.rs`): the
//! JSON Schema the model receives and the check a call runs come from the
//! same type.

use schemars::{JsonSchema, generate::SchemaSettings};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::time::Duration;

/// What `background` tells the model.
const BACKGROUND: &str = "true for a command that runs until stopped, such as a development server or a watcher: the call returns once its output has been quiet for 2 seconds, and only its exit wakes you.";

/// What a call's `description` asks of the model: the title the user sees,
/// which the model writes before the step runs.
pub(super) const DESCRIPTION: &str = "The step's title the user sees, an imperative written before it runs, such as \"Run the type checker\".";

// `description` comes first: a model writes a call's fields in the schema's
// order, so the title the page shows arrives before a long script.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ShellInput {
    #[schemars(length(min = 1), description = DESCRIPTION)]
    pub(super) description: NonEmpty,
    pub(super) script: String,
    #[serde(default)]
    #[schemars(description = BACKGROUND)]
    pub(super) background: bool,
}

/// How long a script's first command sleeps, when it is a `sleep`: the sum of
/// its arguments, each a number with an optional `s`, `m`, `h` or `d` suffix,
/// up to the first `;`, `&&`, `||`, `|` or line break. A `sleep` that a single
/// `&` sends to the background holds nothing.
pub(super) fn leading_sleep(script: &str) -> Option<Duration> {
    let script = script.trim_start();
    let end = script.find([';', '&', '|', '\n']).unwrap_or(script.len());
    if script[end..].starts_with('&') && !script[end..].starts_with("&&") {
        return None;
    }
    let first = &script[..end];
    let mut words = first.split_whitespace();
    if words.next()? != "sleep" {
        return None;
    }
    let mut seconds = 0.0_f64;
    for word in words {
        let (number, unit) = match word.char_indices().last() {
            Some((at, 's')) => (&word[..at], 1.0),
            Some((at, 'm')) => (&word[..at], 60.0),
            Some((at, 'h')) => (&word[..at], 3_600.0),
            Some((at, 'd')) => (&word[..at], 86_400.0),
            _ => (word, 1.0),
        };
        seconds += number.parse::<f64>().ok().filter(|number| *number >= 0.0)? * unit;
    }
    Some(Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX))
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
    fn the_schema_requires_a_title_first_and_offers_background_and_nothing_else() {
        let shell = schema::<ShellInput>();
        assert_eq!(shell["additionalProperties"], false);
        assert_eq!(shell["required"], json!(["description", "script"]));
        let properties = &shell["properties"];
        // The title leads: serde_json keeps the order (`preserve_order`), so
        // the schema a provider serializes lists it first too.
        let order: Vec<&String> = properties.as_object().unwrap().keys().collect();
        assert_eq!(order, ["description", "script", "background"]);
        assert_eq!(properties["background"]["type"], "boolean");
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
                json!({"script": "true", "description": "Run", "background": "yes"}),
                "background: invalid type: string",
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
        assert!(!plain.background);
        let server: ShellInput = parse(
            "shell",
            json!({"script": "npm run dev", "background": true, "description": "Start the dev server"}),
        )
        .unwrap();
        assert!(server.background);
    }

    #[test]
    fn a_script_that_starts_with_a_sleep_says_how_long_it_sleeps() {
        let secs = |script: &str| leading_sleep(script).map(|sleep| sleep.as_secs_f64());
        assert_eq!(secs("sleep 45"), Some(45.0));
        assert_eq!(secs("  sleep 1m 30s && make"), Some(90.0));
        assert_eq!(secs("sleep 0.5; make"), Some(0.5));
        assert_eq!(secs("sleep 2h\nmake"), Some(7_200.0));
        // Only a script's first command counts, and only a `sleep`.
        assert_eq!(secs("make && sleep 60"), None);
        assert_eq!(secs("sleeper 60"), None);
        assert_eq!(secs("sleep forever"), None);
        // One a single `&` sends to the background holds nothing.
        assert_eq!(secs("sleep 60 & echo started"), None);
        assert_eq!(secs("sleep 60 && echo done"), Some(60.0));
        assert_eq!(secs("echo sleep 60"), None);
    }
}
