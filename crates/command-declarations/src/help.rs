//! Help rendering: the text `--help` prints for a group or a command, and
//! the model's capability index.

use serde_json::Value;

use crate::{Group, Leaf, Node, StdinRead, parse::placeholder};

/// The paragraph the capability index opens with: what every command does
/// unless its own help says otherwise.
pub const HELP_DEFAULTS: &str = "Commands print data on stdout, a list as a table and one object as Key: value lines, as JSON with --json where offered, and errors on stderr; a failure exits 1, a usage error 2. --help works at every level. Stdin bodies come by heredoc or pipe. A value starting with -- goes as --name=value, or after --. `demi file view` shows you an image, a video or a PDF.";

/// The paragraph after [`HELP_DEFAULTS`] that introduces the groups'
/// entries.
pub const INDEX_OPENER: &str = "Demi's own capabilities are `demi` commands you run in the shell. Each group below says what it is for; read its `--help` before you first use it, and an operation's `--help` for its arguments.";

/// The model's capability index: the defaults and the opener, then each
/// group's section, sorted by the group's path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityIndex {
    /// The groups, sorted by their paths.
    pub groups: Vec<IndexGroup>,
}

/// One command group's section of the capability index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexGroup {
    /// The group's command line, such as `demi browser`.
    pub path: String,
    /// Its section: its path, its entry, its operations' names and where its
    /// details are.
    pub section: String,
}

impl CapabilityIndex {
    /// Whether it lists no group.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The index as the system prompt holds it; nothing when it lists no
    /// group.
    pub fn text(&self) -> String {
        if self.groups.is_empty() {
            return String::new();
        }
        let mut sections = vec![HELP_DEFAULTS, INDEX_OPENER];
        sections.extend(self.groups.iter().map(|group| group.section.as_str()));
        sections.join("\n\n")
    }
}

/// The model's capability index of `groups`, each a top-level group with
/// its path and its entry: the defaults and the opener, then each group,
/// sorted by its path, with its entry, its operations' names and where its
/// details are.
pub fn render_index<'a, B: 'a>(
    groups: impl IntoIterator<Item = (Vec<&'a str>, &'a str, &'a Group<B>)>,
) -> CapabilityIndex {
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by(|(left, _, _), (right, _, _)| left.cmp(right));
    let groups = groups
        .into_iter()
        .map(|(path, entry, group)| {
            let path = path.join(" ");
            let mut operations = Vec::new();
            for child in &group.subcommands {
                operation_names(child, &mut Vec::new(), &mut operations);
            }
            let section = format!(
                "{path}\n{entry}\nOperations: {}\nDetails: {path} --help; one operation: {path} <operation> --help",
                operations.join(", ")
            );
            IndexGroup { path, section }
        })
        .collect();
    CapabilityIndex { groups }
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

impl<B> Leaf<B> {
    /// The usage line of this leaf, named by the command line `path`: its
    /// positionals, its options, `--json` and its rest field, an optional
    /// one in brackets. Help, the capability index and a usage error show
    /// it alike.
    pub fn usage(&self, path: &str) -> String {
        self.usage_of(path, true)
    }

    /// The usage line, with the stdin field's positional form or without
    /// it, as the line that a stdin body follows shows it.
    fn usage_of(&self, path: &str, stdin_positional: bool) -> String {
        let mut arguments: Vec<String> = self
            .positionals
            .iter()
            .flatten()
            .filter(|field| stdin_positional || self.stdin_field.as_ref() != Some(*field))
            .cloned()
            .collect();
        if let Some(properties) = self.properties() {
            arguments.extend(
                properties
                    .keys()
                    .filter(|field| source(self, field) == Source::Option)
                    .cloned(),
            );
        }
        if let Some(field) = &self.rest_field {
            arguments.push(field.clone());
        }
        let mut arguments: Vec<String> = arguments
            .iter()
            .map(|field| {
                let schema = &self.properties().expect("validated field schemas")[field];
                let syntax = syntax(field, schema, source(self, field));
                if self.required_on_line(field) {
                    syntax
                } else {
                    format!("[{syntax}]")
                }
            })
            .collect();
        if self.json_output().is_some() {
            let index = arguments.len() - usize::from(self.rest_field.is_some());
            arguments.insert(index, "[--json]".into());
        }
        std::iter::once(path.to_owned())
            .chain(arguments)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl<B> Node<B> {
    /// The help of this node, as command-line programs print it
    /// (`crates/shared-types/src/output.rs`): `Usage:` first, then what it does, then a
    /// group's commands or a command's arguments, options and stdin; `path`
    /// is the command line that names this node.
    pub fn help(&self, path: &str) -> String {
        let mut sections = Vec::new();
        match self {
            Node::Group(group) => {
                sections.push(format!("Usage: {path} <command>"));
                sections.push(group.summary.clone());
                if let Some(details) = &group.details {
                    sections.push(details.clone());
                }
                let commands: Vec<(String, String)> = group
                    .subcommands
                    .iter()
                    .map(|child| (child.name().to_owned(), child.summary().to_owned()))
                    .collect();
                sections.push(listing("Commands:", &commands));
            }
            Node::Leaf(leaf) => {
                let mut usage = Vec::new();
                if let Some(field) = &leaf.stdin_field {
                    // A stdin field that is also a positional has both forms.
                    if leaf.is_positional(field) {
                        usage.push(leaf.usage(path));
                    }
                    usage.extend([
                        format!("{} <<'EOF'", leaf.usage_of(path, false)),
                        format!("<{field}>"),
                        "EOF".into(),
                    ]);
                } else {
                    usage.push(leaf.usage(path));
                }
                sections.push(format!("Usage: {}", usage.join("\n       ")));
                sections.push(leaf.summary.clone());
                let mut arguments = Vec::new();
                let mut options = Vec::new();
                let mut stdin = Vec::new();
                for (field, schema) in leaf.properties().into_iter().flatten() {
                    let source = source(leaf, field);
                    let mut notes = Vec::new();
                    if leaf.stdin_field.as_ref() == Some(field) {
                        match &leaf.stdin_read {
                            None if leaf.is_positional(field) => {
                                notes.push(format!("not read when given as <{field}>"));
                            }
                            None => {}
                            Some(read) => {
                                let words = match read {
                                    StdinRead::Unless(_) => "not read with",
                                    StdinRead::With(_) => "read only with",
                                };
                                let options: Vec<String> =
                                    read.options().iter().map(|option| format!("--{option}")).collect();
                                notes.push(format!("{words} {}", options.join(" or ")));
                            }
                        }
                        stdin.push((format!("<{field}>"), with_notes(schema, notes)));
                        if !leaf.is_positional(field) {
                            continue;
                        }
                        notes = Vec::new();
                    }
                    if source != Source::Option {
                        notes.push(
                            if leaf.required_on_line(field) { "required" } else { "optional" }.to_owned(),
                        );
                    }
                    if matches!(source, Source::Option | Source::Positional)
                        && schema.get("type").and_then(Value::as_str) == Some("array")
                    {
                        notes.push("repeatable".into());
                    }
                    if leaf.positional_option(field) {
                        notes.push(format!("or --{field} <{}>", placeholder(field, schema)));
                    }
                    if leaf.required(field) && source == Source::Option {
                        notes.push("required".into());
                    }
                    let entry = (syntax(field, schema, source), with_notes(schema, notes));
                    match source {
                        Source::Positional | Source::Rest => {
                            // In the order the usage line shows them: the
                            // positionals, then the rest field.
                            let order = leaf
                                .positionals
                                .iter()
                                .flatten()
                                .position(|positional| positional == field)
                                .unwrap_or(usize::MAX);
                            arguments.push((order, entry));
                        }
                        Source::Option | Source::Stdin => options.push(entry),
                    }
                }
                if leaf.json_output().is_some() {
                    options.push(("--json".into(), "Print JSON instead of text".into()));
                }
                arguments.sort_by_key(|(order, _)| *order);
                let arguments: Vec<_> = arguments.into_iter().map(|(_, entry)| entry).collect();
                for (title, entries) in [("Arguments:", &arguments), ("Options:", &options), ("Stdin:", &stdin)] {
                    if !entries.is_empty() {
                        sections.push(listing(title, entries));
                    }
                }
                let mut notes = Vec::new();
                if let Some(success) = leaf.success_output.as_ref().filter(|value| !value.is_empty()) {
                    notes.push(format!("Output: {success}"));
                }
                if let Some(failure) = leaf.failure_output.as_ref().filter(|value| !value.is_empty()) {
                    notes.push(format!("Failure: {failure}"));
                }
                if let Some(permission) = &leaf.permission {
                    notes.push(format!("Permission: needs the user's permission ({permission}) in each conversation; without it, the command asks the user and waits for the answer."));
                }
                if leaf.media {
                    notes.push("Media: shows you images, videos and PDFs, attached to the result that reports the command's end, whatever its stdout is.".into());
                }
                if !notes.is_empty() {
                    sections.push(notes.join("\n"));
                }
            }
        }
        sections.join("\n\n")
    }
}

/// A titled list of names and what each is, the names aligned in a column,
/// as `Commands:`, `Arguments:` and `Options:` print.
fn listing(title: &str, entries: &[(String, String)]) -> String {
    let width = entries.iter().map(|(name, _)| name.chars().count()).max().unwrap_or(0);
    let mut lines = vec![title.to_owned()];
    for (name, meaning) in entries {
        let line = format!("  {name:width$}  {meaning}");
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}

/// A field's description followed by its notes in parentheses, such as
/// `File path (required)`.
fn with_notes(schema: &Value, notes: Vec<String>) -> String {
    let description = schema
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .trim_end_matches('.');
    match (description.is_empty(), notes.is_empty()) {
        (_, true) => description.to_owned(),
        (true, false) => format!("({})", notes.join(", ")),
        (false, false) => format!("{description} ({})", notes.join(", ")),
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Source {
    Positional,
    Stdin,
    Rest,
    Option,
}

/// Where a field's value comes from on the command line; a stdin field that
/// is also a positional shows as the positional.
fn source<B>(leaf: &Leaf<B>, field: &str) -> Source {
    if leaf.is_positional(field) {
        Source::Positional
    } else if leaf.stdin_field.as_deref() == Some(field) {
        Source::Stdin
    } else if leaf.rest_field.as_deref() == Some(field) {
        Source::Rest
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
        _ if schema.get("type").and_then(Value::as_str) == Some("boolean") => format!("--{field}"),
        _ => format!("--{field} <{}>", placeholder(field, schema)),
    }
}
