//! Each tool's input, declared once (`runtime.md` § Tool input): the JSON
//! Schema the model receives and the check a call runs come from the same
//! type. An optional field is absent or a value, never null.

use demi_core::{CommandId, ShellId};
use schemars::{JsonSchema, generate::SchemaSettings};
use serde::de::{self, DeserializeOwned, Unexpected, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use serde_with::rust::unwrap_or_skip;

/// Reads a command's or a shell's number as the model writes it: an
/// integer, or its digits as a string (`runtime.md` § Identifiers the model
/// sees). serde_with's `PickFirst` does this behind features the workspace
/// leaves off, which would rebuild every crate that uses it.
struct NumberVisitor;

impl Visitor<'_> for NumberVisitor {
    type Value = u64;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a number such as 17")
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<u64, E> {
        Ok(value)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<u64, E> {
        value
            .parse()
            .map_err(|_| E::invalid_value(Unexpected::Str(value), &self))
    }
}

/// The identity a number names.
fn numbered<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: TryFrom<String>,
    T::Error: std::fmt::Display,
{
    let number = deserializer.deserialize_any(NumberVisitor)?;
    T::try_from(number.to_string()).map_err(de::Error::custom)
}

/// The identity a number names, of an optional field that is absent or a
/// number, never null.
fn some_numbered<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: TryFrom<String>,
    T::Error: std::fmt::Display,
{
    numbered(deserializer).map(Some)
}

/// The longest window a shell tool watches, and the longest yield.
const MAX_DELAY_MS: u32 = 600_000;

/// What a call's `description` asks of the model.
const DESCRIPTION: &str = "Concise title for the concrete user-visible state or result to make visible or confirm. Do not describe waiting, pausing, tool mechanics, generic actions, object labels, steps, tool names, ids, internals, or reasons.";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ShellExecInput {
    pub(super) script: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String", description = DESCRIPTION)]
    #[expect(
        dead_code,
        reason = "the call's title, which the renderer reads from its input"
    )]
    description: Option<String>,
    #[serde(default, deserialize_with = "some_numbered")]
    #[schemars(with = "u64")]
    pub(super) shell_id: Option<ShellId>,
    #[schemars(range(min = 1, max = MAX_DELAY_MS))]
    pub(super) timeout_ms: DelayMs,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CommandInput {
    #[serde(deserialize_with = "numbered")]
    #[schemars(with = "u64")]
    pub(super) command_id: CommandId,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String", description = DESCRIPTION)]
    #[expect(
        dead_code,
        reason = "the call's title, which the renderer reads from its input"
    )]
    description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ShellWriteInput {
    #[serde(deserialize_with = "numbered")]
    #[schemars(with = "u64")]
    pub(super) command_id: CommandId,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String", description = DESCRIPTION)]
    #[expect(
        dead_code,
        reason = "the call's title, which the renderer reads from its input"
    )]
    description: Option<String>,
    #[schemars(length(min = 1))]
    pub(super) stdin: Stdin,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct YieldInput {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String", description = DESCRIPTION)]
    #[expect(
        dead_code,
        reason = "the call's title, which the renderer reads from its input"
    )]
    description: Option<String>,
    #[schemars(range(min = 1, max = MAX_DELAY_MS))]
    pub(super) duration_ms: DelayMs,
}

// Whole milliseconds from 1 to 600,000: the window a shell tool watches, or
// the wait of a yield. A fraction is refused, not rounded. (A doc comment
// would become the field's description in the model's schema.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(try_from = "u32")]
#[schemars(inline)]
pub(super) struct DelayMs(pub(super) u32);

impl TryFrom<u32> for DelayMs {
    type Error = String;

    fn try_from(milliseconds: u32) -> Result<Self, String> {
        if (1..=MAX_DELAY_MS).contains(&milliseconds) {
            Ok(Self(milliseconds))
        } else {
            Err(format!(
                "{milliseconds} is not a whole number of milliseconds from 1 to {MAX_DELAY_MS}"
            ))
        }
    }
}

// What `shell_write` writes: not empty, since polling is another tool's.
// (A doc comment would become the field's description in the model's
// schema.)
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(try_from = "String")]
#[schemars(inline)]
pub(super) struct Stdin(pub(super) String);

impl TryFrom<String> for Stdin {
    type Error = &'static str;

    fn try_from(stdin: String) -> Result<Self, &'static str> {
        if stdin.is_empty() {
            return Err("must not be empty; use shell_status to poll");
        }
        Ok(Self(stdin))
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
    fn a_schema_declares_integer_windows_string_handles_and_nothing_else() {
        let exec = schema::<ShellExecInput>();
        assert_eq!(exec["additionalProperties"], false);
        assert_eq!(exec["required"], json!(["script", "timeoutMs"]));
        let properties = &exec["properties"];
        assert_eq!(properties["timeoutMs"]["type"], "integer");
        assert_eq!(properties["timeoutMs"]["minimum"], 1);
        assert_eq!(properties["timeoutMs"]["maximum"], 600_000);
        assert_eq!(properties["shellId"]["type"], "integer");
        assert_eq!(
            properties["description"],
            json!({"type": "string", "description": DESCRIPTION})
        );
        assert!(!exec.contains_key("$schema") && !exec.contains_key("title"));
        assert_eq!(
            schema::<ShellWriteInput>()["properties"]["stdin"]["minLength"],
            1
        );
    }

    #[test]
    fn a_refusal_names_the_tool_and_the_offending_field() {
        let refusal = |input: Value| parse::<ShellExecInput>("shell_exec", input).unwrap_err();
        for (input, field) in [
            (
                json!({"script": "true", "timeoutMs": 1, "shellId": "main"}),
                "shellId: ",
            ),
            (
                json!({"script": "true", "timeoutMs": 1, "shellId": null}),
                "shellId: ",
            ),
            (
                json!({"script": "true", "timeoutMs": 1, "maxOutputBytes": 10}),
                "unknown field `maxOutputBytes`",
            ),
            (
                json!({"script": "true", "timeoutMs": 1.5}),
                "timeoutMs: invalid type: floating point",
            ),
            (
                json!({"script": "true", "timeoutMs": 0}),
                "timeoutMs: 0 is not a whole number of milliseconds from 1 to 600000",
            ),
            (
                json!({"script": "true", "timeoutMs": 600_001}),
                "timeoutMs: 600001 is not",
            ),
            (
                json!({"script": "true", "timeoutMs": 1, "description": null}),
                "description: invalid type: null",
            ),
            (json!("not json"), "invalid type: string"),
        ] {
            let text = refusal(input);
            assert!(text.starts_with("shell_exec input is invalid:\n"), "{text}");
            assert!(text.contains(field), "{text}");
        }
        let empty = parse::<ShellWriteInput>("shell_write", json!({"commandId": 7, "stdin": ""}));
        assert!(
            empty
                .unwrap_err()
                .contains("stdin: must not be empty; use shell_status to poll")
        );
        let exec: ShellExecInput = parse(
            "shell_exec",
            json!({"script": "ls", "timeoutMs": 600_000, "description": "Files"}),
        )
        .unwrap();
        assert_eq!(
            (exec.script.as_str(), exec.timeout_ms, exec.shell_id),
            ("ls", DelayMs(600_000), None)
        );
    }
}
