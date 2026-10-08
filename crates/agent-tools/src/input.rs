//! Each tool's input, declared once (`runtime.md` § Tool input): the JSON
//! Schema the model receives and the check a call runs come from the same
//! type. An optional field is absent or a value, never null.

use demi_shared_types::{CommandId, ShellId};
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

/// What a call's `description` asks of the model: the title the user sees,
/// which the model writes before the step runs (`runtime.md` § Tool
/// descriptions).
pub(super) const DESCRIPTION: &str = "Short title of what this step does, as a command in the imperative, such as \"Install Chrome for Testing\", \"Start the browser with a blank tab\" or \"Run the type checker\". It is written before the step runs, so never state a result or a state reached (not \"Chrome for Testing installed\"). Do not describe waiting, pausing or tool mechanics; no generic action or bare noun; no scripts, output, protocol state, step numbers, tool names, ids, internal labels or reasons.";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ShellExecInput {
    pub(super) script: String,
    #[schemars(length(min = 1), description = DESCRIPTION)]
    #[expect(
        dead_code,
        reason = "the call's title, which the renderer reads from its input"
    )]
    description: NonEmpty,
    #[serde(default, deserialize_with = "some_numbered")]
    #[schemars(with = "u64")]
    pub(super) shell_id: Option<ShellId>,
    #[schemars(range(min = 1, max = MAX_DELAY_MS))]
    pub(super) timeout_ms: DelayMs,
}

/// What `shell_status` takes, as the model's schema declares it; a call is
/// decoded as [`StatusInput`], which adds the rule the schema cannot state.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StatusFields {
    #[serde(deserialize_with = "numbered")]
    #[schemars(with = "u64")]
    command_id: CommandId,
    #[serde(default, deserialize_with = "unwrap_or_skip::deserialize")]
    #[schemars(with = "String", length(min = 1), description = STDIN)]
    stdin: Option<NonEmpty>,
    #[serde(default, deserialize_with = "unwrap_or_skip::deserialize")]
    #[schemars(with = "DelayMs", range(min = 1, max = MAX_DELAY_MS), description = WATCH)]
    timeout_ms: Option<DelayMs>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String", description = DESCRIPTION)]
    description: Option<String>,
}

/// What `stdin` asks of the model.
const STDIN: &str = "Input to write to the command before looking at it, such as an answer to its prompt; end it with a newline for a line-based prompt.";

/// What `shell_status`'s `timeoutMs` asks of the model.
const WATCH: &str = "How long to wait for the command to end before looking; without it, look at once.";

/// A `shell_status` call: a look at a command, which writes `stdin` first
/// when it has some, and then watches the command up to `timeout_ms`. A
/// call that writes needs a title (`runtime.md` § Tool descriptions).
#[derive(Debug, Deserialize)]
#[serde(try_from = "StatusFields")]
pub(super) struct StatusInput {
    pub(super) command_id: CommandId,
    pub(super) stdin: Option<NonEmpty>,
    pub(super) timeout_ms: Option<DelayMs>,
}

impl TryFrom<StatusFields> for StatusInput {
    type Error = &'static str;

    fn try_from(fields: StatusFields) -> Result<Self, &'static str> {
        let titled = fields
            .description
            .as_deref()
            .is_some_and(|description| !description.is_empty());
        if fields.stdin.is_some() && !titled {
            return Err("description: required when stdin is given");
        }
        Ok(Self {
            command_id: fields.command_id,
            stdin: fields.stdin,
            timeout_ms: fields.timeout_ms,
        })
    }
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
    #[serde(default, deserialize_with = "some_numbers")]
    #[schemars(with = "Vec<u64>", length(min = 1), description = COMMANDS)]
    pub(super) command_ids: Option<Vec<CommandId>>,
}

/// What `yield`'s `commandIds` asks of the model.
const COMMANDS: &str = "Commands to wait for, by commandId: you are woken as soon as the first of them ends, or when durationMs passes.";

/// A command's number as the model writes it.
struct Number(u64);

impl<'de> Deserialize<'de> for Number {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(NumberVisitor).map(Self)
    }
}

/// The commands a non-empty list of numbers names, of an optional field
/// that is absent or a list, never null.
fn some_numbers<'de, D>(deserializer: D) -> Result<Option<Vec<CommandId>>, D::Error>
where
    D: Deserializer<'de>,
{
    let numbers = Vec::<Number>::deserialize(deserializer)?;
    if numbers.is_empty() {
        return Err(de::Error::custom("must not be empty"));
    }
    numbers
        .into_iter()
        .map(|Number(number)| CommandId::try_from(number.to_string()).map_err(de::Error::custom))
        .collect::<Result<_, _>>()
        .map(Some)
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

// Text a call must not leave empty: the title of a step that starts or
// feeds work, and the input `shell_status` writes, since writing nothing is
// a look without it. (A doc comment would become the field's description in
// the model's schema.)
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
    fn a_schema_declares_integer_windows_string_handles_and_nothing_else() {
        let exec = schema::<ShellExecInput>();
        assert_eq!(exec["additionalProperties"], false);
        assert_eq!(
            exec["required"],
            json!(["script", "description", "timeoutMs"])
        );
        let properties = &exec["properties"];
        assert_eq!(properties["timeoutMs"]["type"], "integer");
        assert_eq!(properties["timeoutMs"]["minimum"], 1);
        assert_eq!(properties["timeoutMs"]["maximum"], 600_000);
        assert_eq!(properties["shellId"]["type"], "integer");
        assert_eq!(
            properties["description"],
            json!({"type": "string", "minLength": 1, "description": DESCRIPTION})
        );
        assert!(!exec.contains_key("$schema") && !exec.contains_key("title"));
        // A look at a command that has its title, or a wait, may leave its
        // title out; a look that writes input needs one, which the decode
        // checks.
        let status = schema::<StatusFields>();
        assert_eq!(status["required"], json!(["commandId"]));
        assert_eq!(status["additionalProperties"], false);
        assert_eq!(status["properties"]["stdin"]["type"], "string");
        assert_eq!(status["properties"]["stdin"]["minLength"], 1);
        assert_eq!(status["properties"]["timeoutMs"]["type"], "integer");
        assert_eq!(status["properties"]["timeoutMs"]["maximum"], 600_000);
        let wait = schema::<YieldInput>();
        assert_eq!(wait["required"], json!(["durationMs"]));
        assert_eq!(wait["properties"]["commandIds"]["type"], "array");
        assert_eq!(wait["properties"]["commandIds"]["minItems"], 1);
        assert_eq!(wait["properties"]["commandIds"]["items"]["type"], "integer");
    }

    #[test]
    fn a_refusal_names_the_tool_and_the_offending_field() {
        let refusal = |input: Value| parse::<ShellExecInput>("shell_exec", input).unwrap_err();
        for (input, field) in [
            (
                json!({"script": "true", "timeoutMs": 1, "description": "Run", "shellId": "main"}),
                "shellId: ",
            ),
            (
                json!({"script": "true", "timeoutMs": 1, "description": "Run", "shellId": null}),
                "shellId: ",
            ),
            (
                json!({"script": "true", "timeoutMs": 1, "description": "Run", "maxOutputBytes": 10}),
                "unknown field `maxOutputBytes`",
            ),
            (
                json!({"script": "true", "timeoutMs": 1.5, "description": "Run"}),
                "timeoutMs: invalid type: floating point",
            ),
            (
                json!({"script": "true", "timeoutMs": 0, "description": "Run"}),
                "timeoutMs: 0 is not a whole number of milliseconds from 1 to 600000",
            ),
            (
                json!({"script": "true", "timeoutMs": 600_001, "description": "Run"}),
                "timeoutMs: 600001 is not",
            ),
            // A step that starts work names it for the user.
            (
                json!({"script": "true", "timeoutMs": 1}),
                "missing field `description`",
            ),
            (
                json!({"script": "true", "timeoutMs": 1, "description": ""}),
                "description: must not be empty",
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
        let status = |input: Value| parse::<StatusInput>("shell_status", input);
        let empty = status(json!({"commandId": 7, "description": "Answer the prompt", "stdin": ""}));
        assert!(empty.unwrap_err().contains("stdin: must not be empty"));
        for untitled in [
            json!({"commandId": 7, "stdin": "y\n"}),
            json!({"commandId": 7, "stdin": "y\n", "description": ""}),
        ] {
            let refused = status(untitled).unwrap_err();
            assert!(
                refused.contains("description: required when stdin is given"),
                "{refused}"
            );
        }
        let look = status(json!({"commandId": "7", "timeoutMs": 5_000})).unwrap();
        assert_eq!(
            (look.command_id.as_str(), look.stdin, look.timeout_ms),
            ("7", None, Some(DelayMs(5_000)))
        );
        let wait = |input: Value| parse::<YieldInput>("yield", input);
        let named = wait(json!({"durationMs": 900_000, "commandIds": [17, "18"]}));
        assert!(named.unwrap_err().contains("durationMs: 900000 is not"));
        let named = wait(json!({"durationMs": 60_000, "commandIds": [17, "18"]})).unwrap();
        let named: Vec<&str> = named
            .command_ids
            .iter()
            .flatten()
            .map(CommandId::as_str)
            .collect();
        assert_eq!(named, ["17", "18"]);
        assert!(
            wait(json!({"durationMs": 1, "commandIds": []}))
                .unwrap_err()
                .contains("commandIds: must not be empty")
        );
        let exec: ShellExecInput = parse(
            "shell_exec",
            json!({"script": "ls", "timeoutMs": 600_000, "description": "List the files"}),
        )
        .unwrap();
        assert_eq!(
            (exec.script.as_str(), exec.timeout_ms, exec.shell_id),
            ("ls", DelayMs(600_000), None)
        );
    }
}
