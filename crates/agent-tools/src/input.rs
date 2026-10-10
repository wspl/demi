//! The `shell` tool's input, declared once (`runtime.md` § Tool input): the
//! JSON Schema the model receives and the check a call runs come from the
//! same type.

use schemars::{JsonSchema, generate::SchemaSettings};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// The least interval a command reports at, in milliseconds
/// (`runtime.md` § Tool input).
pub const INTERVAL_FLOOR_MS: u32 = 15_000;
/// The greatest interval a command reports at, and the longest a `shell`
/// call watches its command: ten minutes.
pub const INTERVAL_CAP_MS: u32 = 600_000;

/// What `intervalMs` asks of the model; its bounds are the product's, which
/// every request states alike (`providers.md` § Prompt cache).
const INTERVAL: &str = "How to watch the command, required: a number of whole milliseconds from 15000 to 600000 (ten minutes) for a command that ends, such as a build or a test suite; the call watches it that long, and if it still runs then, the call returns its commandId and the command reports to you every interval until it ends. Short for a command that should end within minutes, long for one that takes long. A value outside the bounds is taken as the nearest bound. null for a command that runs until it is stopped, such as a dev server or a watcher: the call returns once its output has been quiet for 2 seconds, or after 30 seconds, and the command reports only its end.";

/// What a call's `description` asks of the model: the title the user sees,
/// which the model writes before the step runs (`runtime.md` § Tool
/// descriptions).
pub(super) const DESCRIPTION: &str = "Short title of what this step does, as a command in the imperative, such as \"Install Chrome for Testing\", \"Start the browser with a blank tab\" or \"Run the type checker\". It is written before the step runs, so never state a result or a state reached (not \"Chrome for Testing installed\"). Do not describe waiting, pausing or tool mechanics; no generic action or bare noun; no scripts, output, protocol state, step numbers, tool names, ids, internal labels or reasons.";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ShellInput {
    pub(super) script: String,
    #[schemars(length(min = 1), description = DESCRIPTION)]
    pub(super) description: NonEmpty,
    #[serde(deserialize_with = "interval")]
    #[schemars(description = INTERVAL)]
    pub(super) interval_ms: Interval,
}

// How a call asks its command to be watched: whole milliseconds, or none
// for a resident command. Its field is required, and its schema says `null`
// is a value: a field of an `Option` would be optional to serde and
// schemars alike, and schemars's `required` takes the `null` out. (A doc
// comment would become the field's description in the model's schema.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Interval(pub(super) Option<u64>);

impl JsonSchema for Interval {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Interval".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": ["integer", "null"] })
    }
}

/// Reads an interval as the field holds it; serde reads it only when the
/// field is there, since it deserializes the field itself.
fn interval<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Interval, D::Error> {
    Option::<u64>::deserialize(deserializer).map(Interval)
}

/// A command's interval as a node's shells take it: `asked`, within the
/// bounds from `floor` to [`INTERVAL_CAP_MS`], and the line that says so
/// when it was outside them (`runtime.md` § Tool input), naming it as
/// `field` names it, such as `intervalMs 900000 is above the cap; taken as
/// 600000`. The floor is [`INTERVAL_FLOOR_MS`] in the product; a test
/// configures a shorter one.
pub fn taken_interval(asked: u64, floor: u32, field: &str) -> (u32, Option<String>) {
    if asked > u64::from(INTERVAL_CAP_MS) {
        let note = format!("{field} {asked} is above the cap; taken as {INTERVAL_CAP_MS}");
        return (INTERVAL_CAP_MS, Some(note));
    }
    let asked = u32::try_from(asked).expect("within the cap");
    if asked < floor {
        let note = format!("{field} {asked} is below the floor; taken as {floor}");
        return (floor, Some(note));
    }
    (asked, None)
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
    fn the_schema_requires_a_title_and_an_interval_that_may_be_null_and_nothing_else() {
        let shell = schema::<ShellInput>();
        assert_eq!(shell["additionalProperties"], false);
        assert_eq!(
            shell["required"],
            json!(["script", "description", "intervalMs"])
        );
        let properties = &shell["properties"];
        assert_eq!(properties["intervalMs"]["type"], json!(["integer", "null"]));
        // A value outside the bounds is taken as the nearest one, not
        // refused, so the schema states them in words.
        assert!(properties["intervalMs"].get("maximum").is_none());
        assert!(properties["intervalMs"].get("minimum").is_none());
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
                json!({"script": "true", "intervalMs": 20_000, "description": "Run", "timeoutMs": 3}),
                "unknown field `timeoutMs`",
            ),
            (
                json!({"script": "true", "intervalMs": 20_000.5, "description": "Run"}),
                "intervalMs: invalid type: floating point",
            ),
            (
                json!({"script": "true", "intervalMs": -1, "description": "Run"}),
                "intervalMs: invalid value: integer `-1`",
            ),
            // How the command is watched is the model's to say.
            (
                json!({"script": "true", "description": "Run"}),
                "missing field `intervalMs`",
            ),
            // A step that starts work names it for the user.
            (
                json!({"script": "true", "intervalMs": null}),
                "missing field `description`",
            ),
            (
                json!({"script": "true", "intervalMs": null, "description": ""}),
                "description: must not be empty",
            ),
            (json!("not json"), "invalid type: string"),
        ] {
            let text = refusal(input);
            assert!(text.starts_with("shell input is invalid:\n"), "{text}");
            assert!(text.contains(field), "{text}");
        }
        let resident: ShellInput = parse(
            "shell",
            json!({"script": "npm run dev", "intervalMs": null, "description": "Start the dev server"}),
        )
        .unwrap();
        assert_eq!(resident.interval_ms, Interval(None));
    }

    #[test]
    fn an_interval_outside_the_bounds_is_taken_as_the_nearest_with_a_line_that_says_so() {
        let floor = INTERVAL_FLOOR_MS;
        assert_eq!(taken_interval(300_000, floor, "intervalMs"), (300_000, None));
        assert_eq!(
            taken_interval(900_000, floor, "intervalMs"),
            (
                600_000,
                Some("intervalMs 900000 is above the cap; taken as 600000".to_owned())
            )
        );
        assert_eq!(
            taken_interval(0, floor, "intervalMs"),
            (
                15_000,
                Some("intervalMs 0 is below the floor; taken as 15000".to_owned())
            )
        );
        assert_eq!(taken_interval(u64::MAX, floor, "intervalMs").0, 600_000);
    }
}
