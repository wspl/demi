//! Argv parsing: selecting a command in a tree and reading its input from the
//! command line (`commands.md` § Parse input and render help). clap parses
//! argv with a command built from the selected node's declaration, and
//! reports a usage error in its own words; the `jsonschema` validator checks
//! what clap cannot express and reports in the same shape.

use clap::{
    Arg, ArgAction, ArgMatches, ColorChoice, Command,
    builder::{
        BoolValueParser, PossibleValuesParser, StringValueParser, StyledStr, TypedValueParser,
        ValueParser,
    },
    error::{ContextKind, ContextValue, ErrorKind},
};
use serde_json::{Map, Number, Value};

use crate::{Group, Leaf, Node};

/// A command line or arguments that do not fit the selected command, as clap
/// renders it: the error, Demi's tips, the command's usage and the pointer
/// to `--help`, without a final newline. A caller writes it to stderr and
/// exits 2.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct UsageError(String);

impl From<clap::Error> for UsageError {
    fn from(error: clap::Error) -> Self {
        Self(error.render().to_string().trim_end().to_owned())
    }
}

type Error = UsageError;

/// The option every node takes; anywhere before a standalone `--`, it asks
/// for the node's help.
const HELP: &str = "help";

/// The option a leaf with a JSON output schema takes.
const JSON: &str = "json";

/// Option names a model gives a body that a command reads from stdin, such
/// as `--content` for the blocks of `demi file edit`; on a leaf with a stdin
/// field, such an unknown option is refused with the tip to use stdin.
const BODY_OPTIONS: &[&str] = &[
    "body", "code", "content", "data", "input", "script", "stdin", "text",
];

pub struct Selected<'a> {
    pub node: &'a Node,
    pub path: Vec<String>,
    argument_index: usize,
    /// Whether the command line asks for help.
    help: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct Parsed {
    pub path: Vec<String>,
    pub values: Map<String, Value>,
    pub json: bool,
    pub help: bool,
}

impl Node {
    /// Selects the command `argv` names below this root. `argv` excludes the
    /// root executable's name. `--help` anywhere before a standalone `--`
    /// selects the deepest node named before it.
    pub fn select(&self, argv: &[String]) -> Result<Selected<'_>, Error> {
        let help = asks_for_help(argv);
        let mut node = self;
        let mut path = vec![self.name().to_owned()];
        let mut index = 0;
        while let Node::Group(group) = node {
            let Some(token) = argv.get(index) else {
                break;
            };
            match group.subcommands.iter().find(|child| child.name() == token) {
                Some(child) => node = child,
                None if help => break,
                None => return Err(group.refusal(&path.join(" "), &argv[index..])),
            }
            path.push(node.name().to_owned());
            index += 1;
        }
        Ok(Selected {
            node,
            path,
            argument_index: index,
            help,
        })
    }
}

/// Whether `argv` holds `--help` before a standalone `--`, after which
/// tokens are data.
fn asks_for_help(argv: &[String]) -> bool {
    argv.iter()
        .take_while(|token| *token != "--")
        .any(|token| token == "--help")
}

impl<B> Group<B> {
    /// Why `argv`, which names none of the group's subcommands, is refused:
    /// clap's error for an unknown subcommand, with a near name it suggests.
    fn refusal(&self, path: &str, argv: &[String]) -> Error {
        let command = self.subcommands.iter().fold(
            base(path, format!("{path} <command>")),
            |command, child| command.subcommand(Command::new(child.name().to_owned())),
        );
        match command.try_get_matches_from(argv) {
            Err(error) => error.into(),
            // Unreachable: the first token names no subcommand, which clap
            // refuses as well; said in clap's words all the same.
            Ok(_) => base(path, format!("{path} <command>"))
                .error(
                    ErrorKind::InvalidSubcommand,
                    format!("unrecognized subcommand '{}'", argv[0]),
                )
                .into(),
        }
    }
}

/// A clap command named `path` with Demi's conventions: no colour, no
/// version, `--help` as the only help, and `usage` as its usage line.
fn base(path: &str, usage: String) -> Command {
    Command::new(path.to_owned())
        .no_binary_name(true)
        .color(ColorChoice::Never)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .override_usage(usage)
        .arg(Arg::new(HELP).long(HELP).action(ArgAction::Help))
}

impl Selected<'_> {
    /// Parses argv without reading stdin. Help therefore never consumes a body.
    pub fn parse(&self, argv: &[String]) -> Result<Parsed, Error> {
        let mut result = Parsed {
            path: self.path.clone(),
            values: Map::new(),
            json: false,
            help: true,
        };
        let Some(leaf) = self.node.leaf().filter(|_| !self.help) else {
            return Ok(result);
        };
        let path = self.path.join(" ");
        let command = leaf.command(&path);
        let tokens = leaf
            .prepared(&path, &argv[self.argument_index..])
            .map_err(|error| leaf.refusal(&path, error))?;
        let matches = match command.try_get_matches_from(tokens) {
            Ok(matches) => matches,
            Err(error) if error.kind() == ErrorKind::DisplayHelp => return Ok(result),
            Err(error) => return Err(leaf.refusal(&path, error)),
        };
        result.help = false;
        result.json = leaf.json_output().is_some() && matches.get_flag(JSON);
        if let Some(properties) = leaf.properties() {
            for (field, schema) in properties {
                if leaf.stdin_field.as_ref() == Some(field) {
                    continue;
                }
                let value = value(&matches, field, schema).or_else(|| {
                    leaf.positional_option(field)
                        .then(|| value(&matches, &option_id(field), schema))
                        .flatten()
                });
                if let Some(value) = value {
                    result.values.insert(field.clone(), value);
                }
            }
        }
        Ok(result)
    }
}

/// The value clap read for `field`, as its schema declares it.
fn value(matches: &ArgMatches, field: &str, schema: &Value) -> Option<Value> {
    let mut values = matches.get_many::<Value>(field)?.cloned();
    if schema_type(schema) == Some("array") {
        Some(Value::Array(values.collect()))
    } else {
        values.next()
    }
}

impl Parsed {
    /// Adds the body read from stdin and checks the whole input. A missing
    /// value stays missing: validation never fills one in.
    pub fn validate(mut self, leaf: &Leaf, stdin: Option<String>) -> Result<Self, Error> {
        if self.help {
            return Ok(self);
        }
        let path = self.path.join(" ");
        // The dispatcher reads stdin for the field the leaf targets, and
        // only for it.
        match (leaf.stdin_target(&self.values), stdin) {
            (Some(field), Some(body)) => {
                self.values.insert(field.to_owned(), Value::String(body));
            }
            (None, None) => {}
            (Some(_), None) => return Err(leaf.refuse(&path, "stdin was not read")),
            (None, Some(_)) => {
                return Err(leaf.refuse(&path, "stdin was read for a command that takes none"));
            }
        }
        leaf.check_arguments(&path, &self.values)?;
        Ok(self)
    }
}

impl<B> Leaf<B> {
    /// Checks a command's arguments against its input, as the runner does
    /// after parsing argv and the backend does with an `rpc` call's
    /// arguments, which arrive as JSON: `"7"` for a number is refused. The
    /// first failure is reported, in clap's shape with the usage of the
    /// command `path` names.
    pub fn check_arguments(&self, path: &str, arguments: &Map<String, Value>) -> Result<(), Error> {
        match &self.input {
            Some(schema) => schema.check(&Value::Object(arguments.clone())),
            None if arguments.is_empty() => Ok(()),
            None => Err("the command takes no arguments".to_owned()),
        }
        .map_err(|failure| self.refuse(path, failure))
    }

    /// A refusal of the command `path` names that clap could not raise, in
    /// clap's shape: `error: "path" is longer than 4096 characters`, its
    /// usage and the pointer to `--help`.
    fn refuse(&self, path: &str, message: impl std::fmt::Display) -> Error {
        self.command(path)
            .error(ErrorKind::ValueValidation, message)
            .into()
    }

    /// The clap command of this leaf, named `path`: its positionals in
    /// order, a trailing array taking every token left; its options, a
    /// boolean one a flag that takes `=false`; its rest field after `--`;
    /// and `--json` when it declares JSON output. Each value converts to the
    /// JSON its field's schema declares.
    fn command(&self, path: &str) -> Command {
        let mut command = base(path, self.usage(path)).allow_missing_positional(self.missing_positional());
        if self.json_output().is_some() {
            command = command.arg(Arg::new(JSON).long(JSON).action(ArgAction::SetTrue));
        }
        let Some(properties) = self.properties() else {
            return command;
        };
        let positionals = self.positionals.as_deref().unwrap_or_default();
        for (field, schema) in properties {
            let required = self.required(field);
            let arg = Arg::new(field.clone())
                .value_name(placeholder(field, schema))
                .required(required);
            let array = schema_type(schema) == Some("array");
            let arg = if let Some(index) = positionals.iter().position(|name| name == field) {
                let action = if array {
                    ArgAction::Append
                } else {
                    ArgAction::Set
                };
                let arg = arg
                    .index(index + 1)
                    .action(action.clone())
                    .value_parser(parser(item(schema)));
                let arg = if array { arg.num_args(1..) } else { arg };
                if !self.positional_option(field) {
                    arg
                } else {
                    // Its option form, never beside the positional.
                    let option = option_id(field);
                    command = command.arg(
                        Arg::new(option.clone())
                            .long(field.clone())
                            .value_name(placeholder(field, schema))
                            .action(action)
                            .conflicts_with(field.clone())
                            .value_parser(parser(item(schema))),
                    );
                    if required {
                        arg.required(false).required_unless_present(option)
                    } else {
                        arg
                    }
                }
            } else if self.rest_field.as_ref() == Some(field) {
                arg.index(positionals.len() + 1)
                    .num_args(1..)
                    .last(true)
                    .action(ArgAction::Append)
                    .value_parser(parser(item(schema)))
            } else if self.stdin_field.as_ref() == Some(field) {
                continue;
            } else if array {
                arg.long(field.clone())
                    .action(ArgAction::Append)
                    .value_parser(parser(item(schema)))
            } else if schema_type(schema) == Some("boolean") {
                arg.long(field.clone())
                    .value_name("true|false")
                    .num_args(0..=1)
                    .require_equals(true)
                    .default_missing_value("true")
                    .action(ArgAction::Set)
                    .value_parser(parser(schema))
            } else {
                arg.long(field.clone())
                    .action(ArgAction::Set)
                    .value_parser(parser(schema))
            };
            command = command.arg(arg);
        }
        command
    }

    /// The command line clap reads: the same tokens, but a value option
    /// whose value begins with a single `-`, such as `--dy -300`, joined
    /// into `--dy=-300`. clap takes such a value only with
    /// `allow_hyphen_values`, which also takes a following `--option` as the
    /// value, where a value beginning with `--` must be written
    /// `--name=value`. A `true` or `false` after a boolean flag is refused
    /// rather than taken as the next positional.
    fn prepared(&self, path: &str, argv: &[String]) -> Result<Vec<String>, clap::Error> {
        let mut tokens = Vec::with_capacity(argv.len());
        let mut index = 0;
        while let Some(token) = argv.get(index) {
            index += 1;
            if token == "--" {
                tokens.extend(argv[index - 1..].iter().cloned());
                break;
            }
            let option = token
                .strip_prefix("--")
                .filter(|name| !name.contains('='))
                .filter(|name| self.takes_option(name));
            let (Some(name), Some(next)) = (option, argv.get(index)) else {
                tokens.push(token.clone());
                continue;
            };
            let schema = &self.properties().expect("an option has a schema")[name];
            if schema_type(schema) == Some("boolean") {
                if next == "true" || next == "false" {
                    let mut error = clap::Error::new(ErrorKind::UnknownArgument)
                        .with_cmd(&self.command(path));
                    error.insert(ContextKind::InvalidArg, ContextValue::String(next.clone()));
                    error.insert(
                        ContextKind::Suggested,
                        ContextValue::StyledStrs(vec![StyledStr::from(format!(
                            "--{name} alone is true; write --{name}=false for false"
                        ))]),
                    );
                    return Err(error);
                }
                tokens.push(token.clone());
            } else if next.starts_with('-') && !next.starts_with("--") {
                tokens.push(format!("--{name}={next}"));
                index += 1;
            } else {
                tokens.push(token.clone());
                if !next.starts_with("--") {
                    tokens.push(next.clone());
                    index += 1;
                }
            }
        }
        Ok(tokens)
    }

    /// A clap error as the caller reads it: with Demi's tip for an option
    /// that names a body read from stdin, a positional or the rest field,
    /// and with the command's usage line, which some of clap's errors leave
    /// out.
    fn refusal(&self, path: &str, mut error: clap::Error) -> Error {
        if error.kind() == ErrorKind::UnknownArgument
            && let Some(ContextValue::String(argument)) = error.get(ContextKind::InvalidArg)
            && let Some(name) = argument.strip_prefix("--")
            && let Some(tip) = self.tip(path, name)
        {
            error.remove(ContextKind::SuggestedArg);
            error.insert(
                ContextKind::Suggested,
                ContextValue::StyledStrs(vec![StyledStr::from(tip)]),
            );
        }
        if error.get(ContextKind::Usage).is_none() {
            error.insert(
                ContextKind::Usage,
                ContextValue::StyledStr(StyledStr::from(format!("Usage: {}", self.usage(path)))),
            );
        }
        error.into()
    }

    /// Whether `--name` is one of the leaf's options: a named option, or a
    /// positional that `positionalOptions` lets take one.
    fn takes_option(&self, name: &str) -> bool {
        crate::is_option(self, name) || self.positional_option(name)
    }

    /// Whether the positional `field` may also be given as `--field`.
    pub(crate) fn positional_option(&self, field: &str) -> bool {
        self.positional_options
            .iter()
            .flatten()
            .any(|name| name == field)
    }

    /// Demi's tip for the unknown option `--name`, when it names how the
    /// command takes the value instead.
    fn tip(&self, path: &str, name: &str) -> Option<String> {
        if let Some(field) = &self.stdin_field
            && (field == name || BODY_OPTIONS.contains(&name))
        {
            return Some(format!(
                "\"{path}\" reads {field} only from stdin; remove --{name} and use a quoted heredoc"
            ));
        }
        if self.rest_field.as_deref() == Some(name) {
            return Some(format!(
                "\"{path}\" takes {name} after --; remove --{name} and write them after --"
            ));
        }
        self.positionals
            .iter()
            .flatten()
            .any(|field| field == name)
            .then(|| {
                format!("\"{path}\" takes <{name}> as a positional argument; remove --{name} and give the value alone")
            })
    }
}

/// The clap id of the option form of the positional `field`.
fn option_id(field: &str) -> String {
    format!("--{field}")
}

/// The schema of one value of a field: its items' for an array.
fn item(schema: &Value) -> &Value {
    if schema_type(schema) == Some("array") {
        &schema["items"]
    } else {
        schema
    }
}

/// How clap converts one argv token of a field with `schema`: into the JSON
/// value of its type, a string of its enum, or text.
fn parser(schema: &Value) -> ValueParser {
    match (schema_type(schema), schema.get("enum").and_then(Value::as_array)) {
        (Some("string"), Some(choices)) => PossibleValuesParser::new(
            choices
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        )
        .map(Value::String)
        .into(),
        (Some("integer"), _) => clap::value_parser!(i64).map(Value::from).into(),
        // clap has no typed parser for `f64` to map from.
        (Some("number"), _) => StringValueParser::new().try_map(number).into(),
        (Some("boolean"), _) => BoolValueParser::new().map(Value::Bool).into(),
        _ => StringValueParser::new().map(Value::String).into(),
    }
}

/// The JSON number `text` spells, when finite: an integer when it is one
/// JavaScript holds exactly, so `2` stays `2` rather than `2.0`.
fn number(text: String) -> Result<Value, String> {
    let value: f64 = text.parse().map_err(|error| format!("{error}"))?;
    if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0 {
        Ok(Value::from(value as i64))
    } else {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| "not a finite number".to_owned())
    }
}

pub(crate) fn schema_type(schema: &Value) -> Option<&str> {
    schema.get("type").and_then(Value::as_str)
}

/// The placeholder of a field's value: its enum's choices, or its name.
pub(crate) fn placeholder(field: &str, schema: &Value) -> String {
    item(schema)
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
        .unwrap_or_else(|| field.to_owned())
}
