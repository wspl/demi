use super::{Leaf, Node};
use serde_json::Value;

impl Node {
    pub fn help(&self, path: &str) -> String {
        let mut lines = vec![format!("{path}: {}", self.summary())];
        if let Some(leaf) = self.leaf() {
            lines.extend([String::new(), "Usage:".into(), String::new()]);
            let mut arguments = leaf.positionals().clone().unwrap_or_default();
            if let Some(properties) = leaf.properties() {
                arguments.extend(
                    properties
                        .keys()
                        .filter(|field| source(leaf, field) == Source::Option)
                        .cloned(),
                );
            }
            if let Some(field) = leaf.rest_field() {
                arguments.push(field.clone());
            }
            let mut arguments: Vec<String> = arguments
                .iter()
                .map(|field| {
                    let schema = &leaf.properties().expect("validated field schemas")[field];
                    let syntax = syntax(field, schema, source(leaf, field));
                    if leaf.required(field) {
                        syntax
                    } else {
                        format!("[{syntax}]")
                    }
                })
                .collect();
            if leaf.json_output().is_some() {
                let index = arguments.len() - usize::from(leaf.rest_field().is_some());
                arguments.insert(index, "[--json]".into());
            }
            let invocation = std::iter::once(path.to_owned())
                .chain(arguments)
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(field) = leaf.stdin_field() {
                lines.extend([
                    format!("  {invocation} <<'EOF'"),
                    format!("  <{field}>"),
                    "  EOF".into(),
                ]);
            } else {
                lines.push(format!("  {invocation}"));
            }
            if let Some(success) = leaf
                .success_output()
                .as_ref()
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("    Success output: {success}"));
            } else if leaf.json_output().is_some() {
                lines.push("    Success output: raw text by default; machine-readable JSON when --json is passed".into());
            }
            if let Some(failure) = leaf
                .failure_output()
                .as_ref()
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("    Failure output: {failure}"));
            }
            if let Some(properties) = leaf.properties() {
                let fields: Vec<_> = properties
                    .iter()
                    .filter(|(field, _)| leaf.stdin_field().as_ref() != Some(*field))
                    .collect();
                if !fields.is_empty() {
                    lines.push("    Parameters:".into());
                    for (field, schema) in fields {
                        let source = source(leaf, field);
                        let required = if leaf.required(field) {
                            "required"
                        } else {
                            "optional"
                        };
                        let repeatable = if source == Source::Option
                            && schema.get("type").and_then(Value::as_str) == Some("array")
                        {
                            ", repeatable"
                        } else {
                            ""
                        };
                        lines.push(format!(
                            "      {} ({required}{repeatable}){}",
                            syntax(field, schema, source),
                            description(schema)
                        ));
                    }
                }
                if let Some(field) = leaf.stdin_field() {
                    lines.push(format!(
                        "    Stdin body: {field}{}",
                        description(&properties[field])
                    ));
                }
            }
            if leaf.json_output().is_some() {
                lines.push("    --json: emits machine-readable JSON for this command".into());
            }
        }
        let mut blocks = Vec::new();
        if let Node::Variant0(group) = self {
            lines.extend([String::new(), "Subcommands:".into()]);
            for child in &group.subcommands {
                lines.push(format!("  {path} {} — {}", child.name(), child.summary()));
            }
            blocks.push(lines.join("\n"));
            for child in &group.subcommands {
                blocks.push(child.help(&format!("{path} {}", child.name())));
            }
        } else {
            blocks.push(lines.join("\n"));
        }
        blocks.join("\n\n")
    }
}

#[derive(Eq, PartialEq)]
enum Source {
    Positional,
    Stdin,
    Rest,
    Option,
}

fn source(leaf: &Leaf, field: &str) -> Source {
    if leaf.stdin_field().as_deref() == Some(field) {
        Source::Stdin
    } else if leaf.rest_field().as_deref() == Some(field) {
        Source::Rest
    } else if leaf
        .positionals()
        .as_ref()
        .is_some_and(|fields| fields.iter().any(|name| name == field))
    {
        Source::Positional
    } else {
        Source::Option
    }
}

fn syntax(field: &str, schema: &Value, source: Source) -> String {
    match source {
        Source::Positional => format!("<{field}>"),
        Source::Rest => format!("-- <{field}>..."),
        _ if schema.get("type").and_then(Value::as_str) == Some("boolean") => {
            format!("--{field} [true|false]")
        }
        _ => {
            let label = schema
                .get("enum")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .map(|value| {
                            value
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| value.to_string())
                        })
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .unwrap_or_else(|| field.to_owned());
            format!("--{field} <{label}>")
        }
    }
}

fn description(schema: &Value) -> String {
    schema
        .get("description")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(|value| format!(" - {value}"))
        .unwrap_or_default()
}
