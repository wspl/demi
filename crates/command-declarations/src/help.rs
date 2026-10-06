//! Help rendering: the text `--help` prints for a group or a command, and
//! the model's capability index (`commands.md` § Help, `system-prompt.md`
//! § Capability index).

use serde_json::Value;

use crate::{Group, Leaf, Node};

/// The paragraph the capability index opens with: what every command does
/// unless its own help says otherwise (`commands.md` § Help).
pub const HELP_DEFAULTS: &str = "Unless a command states otherwise: success prints raw text on stdout, failure writes an error message to stderr and exits non-zero. Pass --help at any level to print a command's documentation. Usage uses <placeholders> for values and [brackets] for optional arguments. Quote values containing spaces. Stdin bodies use a quoted heredoc, pipe, or input redirection; they have no command-line option. Use --name=value for option values beginning with --, and -- before positional values beginning with --. A command marked as returning media attaches its images and videos to the result when its stdout is the job's output, and otherwise writes a single one's bytes as its stdout.";

/// The paragraph after [`HELP_DEFAULTS`] that introduces the groups'
/// entries (`system-prompt.md` § Capability index).
pub const INDEX_OPENER: &str = "Demi's own capabilities are `demi` commands you run in the shell. Each group below says what it is for; read its `--help` before you first use it, and an operation's `--help` for its arguments.";

/// The model's capability index of `groups`, each a top-level group with
/// its path and its entry: the defaults and the opener, then each group,
/// sorted by its path, with its entry, its operations' names and where its
/// details are. Nothing when there is no group.
pub fn render_index<'a, B: 'a>(
    groups: impl IntoIterator<Item = (Vec<&'a str>, &'a str, &'a Group<B>)>,
) -> String {
    let mut groups: Vec<_> = groups.into_iter().collect();
    if groups.is_empty() {
        return String::new();
    }
    groups.sort_by(|(left, _, _), (right, _, _)| left.cmp(right));
    let mut sections = vec![HELP_DEFAULTS.to_owned(), INDEX_OPENER.to_owned()];
    for (path, entry, group) in groups {
        let path = path.join(" ");
        let mut operations = Vec::new();
        for child in &group.subcommands {
            operation_names(child, &mut Vec::new(), &mut operations);
        }
        sections.push(format!(
            "{path}\n{entry}\nOperations: {}\nDetails: {path} --help; one operation: {path} <operation> --help",
            operations.join(", ")
        ));
    }
    sections.join("\n\n")
}

/// The path of each leaf at or below `node` from the group whose operations
/// are listed, such as `content fetch`; `path` names the subgroups between.
fn operation_names<'a, B>(node: &'a Node<B>, path: &mut Vec<&'a str>, names: &mut Vec<String>) {
    path.push(node.name());
    match node {
        Node::Leaf(_) => names.push(path.join(" ")),
        Node::Group(group) => {
            for child in &group.subcommands {
                operation_names(child, path, names);
            }
        }
    }
    path.pop();
}

impl<B> Node<B> {
    /// The help of this node: a group's lists its subcommands with their
    /// summaries, and a command's gives its full usage; `path` is the
    /// command line that names this node.
    pub fn help(&self, path: &str) -> String {
        let mut lines = vec![format!("{path}: {}", self.summary())];
        if let Some(leaf) = self.leaf() {
            lines.extend([String::new(), "Usage:".into(), String::new()]);
            let mut arguments = leaf.positionals.clone().unwrap_or_default();
            if let Some(properties) = leaf.properties() {
                arguments.extend(
                    properties
                        .keys()
                        .filter(|field| source(leaf, field) == Source::Option)
                        .cloned(),
                );
            }
            if let Some(field) = &leaf.rest_field {
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
                let index = arguments.len() - usize::from(leaf.rest_field.is_some());
                arguments.insert(index, "[--json]".into());
            }
            let invocation = std::iter::once(path.to_owned())
                .chain(arguments)
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(field) = &leaf.stdin_field {
                lines.extend([
                    format!("  {invocation} <<'EOF'"),
                    format!("  <{field}>"),
                    "  EOF".into(),
                ]);
            } else {
                lines.push(format!("  {invocation}"));
            }
            if let Some(success) = leaf
                .success_output
                .as_ref()
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("    Success output: {success}"));
            } else if leaf.json_output().is_some() {
                lines.push("    Success output: raw text by default; machine-readable JSON when --json is passed".into());
            }
            if let Some(failure) = leaf
                .failure_output
                .as_ref()
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("    Failure output: {failure}"));
            }
            if let Some(permission) = &leaf.permission {
                lines.push(format!("    Permission: needs the user's permission ({permission}) in each conversation; without it, the command fails at once and the user is asked."));
            }
            if let Some(properties) = leaf.properties() {
                let fields: Vec<_> = properties
                    .iter()
                    .filter(|(field, _)| leaf.stdin_field.as_ref() != Some(*field))
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
                        let repeatable = if matches!(source, Source::Option | Source::Positional)
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
                if let Some(field) = &leaf.stdin_field {
                    lines.push(format!(
                        "    Stdin body: {field}{}",
                        description(&properties[field])
                    ));
                }
            }
            if leaf.json_output().is_some() {
                lines.push("    --json: emits machine-readable JSON for this command".into());
            }
            if leaf.media {
                lines.push("    Returns media: images and videos, attached to the result when stdout is the job's output; otherwise a single one's bytes are stdout".into());
            }
        }
        if let Node::Group(group) = self {
            lines.extend([String::new(), "Subcommands:".into()]);
            for child in &group.subcommands {
                lines.push(format!("  {path} {} — {}", child.name(), child.summary()));
            }
        }
        lines.join("\n")
    }
}

#[derive(Eq, PartialEq)]
enum Source {
    Positional,
    Stdin,
    Rest,
    Option,
}

fn source<B>(leaf: &Leaf<B>, field: &str) -> Source {
    if leaf.stdin_field.as_deref() == Some(field) {
        Source::Stdin
    } else if leaf.rest_field.as_deref() == Some(field) {
        Source::Rest
    } else if leaf
        .positionals
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
        Source::Positional if schema.get("type").and_then(Value::as_str) == Some("array") => {
            format!("<{field}>...")
        }
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
