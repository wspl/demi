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

/// A command line or arguments that do not fit the selected command, in
/// clap's words on two lines, without a final newline: the error with its
/// tips joined to it, and the command's usage with the pointer to `--help`
/// (`commands.md` § Parse input and render help). A caller writes it to
/// stderr and exits 2.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct UsageError(String);

impl From<clap::Error> for UsageError {
    fn from(error: clap::Error) -> Self {
        Self(two_lines(&error.render().to_string()))
    }
}

/// clap's rendering of an error, which spreads it over paragraphs, on two
/// lines, so that `head -3` or `tail -3` keeps the error: the error, its
/// continuation lines (the missing arguments, the possible values) and each
/// `tip:` joined into the first, and the usage with `; more with --help`.
/// clap offers no formatter that keeps its words but not its layout, so the
/// layout of its `RichFormatter` is read back.
fn two_lines(rendered: &str) -> String {
    let rendered = rendered.trim_end();
    let (rendered, help) = match rendered.strip_suffix(HELP_POINTER) {
        Some(rest) => (rest.trim_end(), true),
        None => (rendered, false),
    };
    let (error, usage) = match rendered.rsplit_once("\n\n") {
        Some((error, usage)) if usage.starts_with("Usage:") => (error, Some(usage)),
        _ => (rendered, None),
    };
    let mut message = String::new();
    let mut tips = Vec::new();
    for line in error.lines().filter(|line| !line.trim().is_empty()) {
        if let Some(tip) = line.trim_start().strip_prefix("tip: ") {
            tips.push(tip);
        } else if message.is_empty() {
            message.push_str(line.trim_end());
        } else {
            message.push(' ');
            message.push_str(line.trim());
        }
    }
    for tip in tips {
        message.push_str("; ");
        message.push_str(tip);
    }
    let mut usage = usage
        .map(|usage| usage.lines().map(str::trim).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    if help {
        usage.push_str(if usage.is_empty() { "more with --help" } else { "; more with --help" });
    }
    if usage.is_empty() {
        message
    } else {
        format!("{message}\n{usage}")
    }
}

/// The paragraph clap ends an error with, `--help` being the help option.
const HELP_POINTER: &str = "For more information, try '--help'.";

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

/// A command line as clap reads it, and what it gives: its positional
/// tokens in order and the names of the options it gives.
struct Line {
    tokens: Vec<String>,
    positionals: Vec<String>,
    options: Vec<String>,
}

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
        let line = leaf
            .prepared(&path, &argv[self.argument_index..])
            .map_err(|error| leaf.refusal(&path, error))?;
        let passed_over = leaf.passed_over(&line);
        let command = leaf.command(&path, passed_over);
        let matches = match command.try_get_matches_from(line.tokens) {
            Ok(matches) => matches,
            Err(error) if error.kind() == ErrorKind::DisplayHelp => return Ok(result),
            Err(error) => return Err(leaf.refusal(&path, error)),
        };
        result.help = false;
        result.json = leaf.json_output().is_some() && matches.get_flag(JSON);
        if let Some(properties) = leaf.properties() {
            for (field, schema) in properties {
                // clap has an argument of the field's own name unless the
                // field is the stdin field alone or a positional passed over.
                let own = passed_over != Some(field.as_str())
                    && (leaf.stdin_field.as_ref() != Some(field) || leaf.is_positional(field));
                let value = own
                    .then(|| value(&matches, field, schema))
                    .flatten()
                    .or_else(|| {
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
        self.command(path, None)
            .error(ErrorKind::ValueValidation, message)
            .into()
    }

    /// The clap command of this leaf, named `path`: its positionals in
    /// order, a trailing array taking every token left, but the one the
    /// command line `passed_over`, which keeps only its option form; its
    /// options, a boolean one a flag that takes `=false`; its rest field
    /// after `--`; and `--json` when it declares JSON output. Each value
    /// converts to the JSON its field's schema declares.
    fn command(&self, path: &str, passed_over: Option<&str>) -> Command {
        let mut command = base(path, self.usage(path)).allow_missing_positional(self.missing_positional());
        if self.json_output().is_some() {
            command = command.arg(Arg::new(JSON).long(JSON).action(ArgAction::SetTrue));
        }
        let Some(properties) = self.properties() else {
            return command;
        };
        let positionals: Vec<&String> = self
            .positionals
            .iter()
            .flatten()
            .filter(|field| passed_over != Some(field.as_str()))
            .collect();
        for (field, schema) in properties {
            let required = self.required_on_line(field);
            let arg = Arg::new(field.clone())
                .value_name(placeholder(field, schema))
                .required(required);
            let array = schema_type(schema) == Some("array");
            let action = if array {
                ArgAction::Append
            } else {
                ArgAction::Set
            };
            // The option form of a positional that `positionalOptions` names.
            let option_form = || {
                Arg::new(option_id(field))
                    .long(field.clone())
                    .value_name(placeholder(field, schema))
                    .action(action.clone())
                    .value_parser(parser(item(schema)))
            };
            let arg = if let Some(index) = positionals.iter().position(|name| *name == field) {
                let arg = arg
                    .index(index + 1)
                    .action(action.clone())
                    .value_parser(parser(item(schema)));
                let arg = if array { arg.num_args(1..) } else { arg };
                if !self.positional_option(field) {
                    arg
                } else {
                    // Its option form, never beside the positional.
                    command = command.arg(option_form().conflicts_with(field.clone()));
                    if required {
                        arg.required(false).required_unless_present(option_id(field))
                    } else {
                        arg
                    }
                }
            } else if passed_over == Some(field.as_str()) {
                if self.positional_option(field) {
                    command = command.arg(option_form());
                }
                continue;
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
    /// rather than taken as the next positional. It also names the
    /// positional tokens and the options the line gives.
    fn prepared(&self, path: &str, argv: &[String]) -> Result<Line, clap::Error> {
        let mut line = Line {
            tokens: Vec::with_capacity(argv.len()),
            positionals: Vec::new(),
            options: Vec::new(),
        };
        let mut index = 0;
        while let Some(token) = argv.get(index) {
            index += 1;
            if token == "--" {
                line.tokens.extend(argv[index - 1..].iter().cloned());
                if self.rest_field.is_none() {
                    line.positionals.extend(argv[index..].iter().cloned());
                }
                break;
            }
            let named = token
                .strip_prefix("--")
                .map(|name| name.split_once('=').map_or(name, |(name, _)| name))
                .filter(|name| self.takes_option(name));
            if let Some(name) = named {
                line.options.push(name.to_owned());
            } else if token == "-" || !token.starts_with('-') {
                line.positionals.push(token.clone());
            }
            let option = named.filter(|_| !token.contains('='));
            let (Some(name), Some(next)) = (option, argv.get(index)) else {
                line.tokens.push(token.clone());
                continue;
            };
            let schema = &self.properties().expect("an option has a schema")[name];
            if schema_type(schema) == Some("boolean") {
                if next == "true" || next == "false" {
                    let mut error = clap::Error::new(ErrorKind::UnknownArgument)
                        .with_cmd(&self.command(path, None));
                    error.insert(ContextKind::InvalidArg, ContextValue::String(next.clone()));
                    error.insert(
                        ContextKind::Suggested,
                        ContextValue::StyledStrs(vec![StyledStr::from(format!(
                            "--{name} alone is true; write --{name}=false for false"
                        ))]),
                    );
                    return Err(error);
                }
                line.tokens.push(token.clone());
            } else if next.starts_with('-') && !next.starts_with("--") {
                line.tokens.push(format!("--{name}={next}"));
                index += 1;
            } else {
                line.tokens.push(token.clone());
                if !next.starts_with("--") {
                    line.tokens.push(next.clone());
                    index += 1;
                }
            }
        }
        Ok(line)
    }

    /// The optional positional the command line passes over, so that its
    /// token fills the optional last positional after it: when its option
    /// form is given, or when the last positional token stands in its place
    /// and is no value of its field (`commands.md` § Parse input and render
    /// help). `browser eval t1 e21` gives the ref and `browser eval t1
    /// 'document.title'` the expression.
    fn passed_over(&self, line: &Line) -> Option<&str> {
        let positionals = self.positionals.as_deref()?;
        let [.., before, last] = positionals else {
            return None;
        };
        if self.required_on_line(before) || self.required_on_line(last) {
            return None;
        }
        if line.options.contains(before) {
            return Some(before);
        }
        let index = positionals.len() - 2;
        let token = line.positionals.get(index).filter(|_| line.positionals.len() == index + 1)?;
        let schema = &self.properties()?[before.as_str()];
        (!jsonschema::is_valid(schema, &Value::String(token.clone()))).then_some(before.as_str())
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
            return Some(if self.is_positional(field) {
                format!(
                    "\"{path}\" takes <{field}> as its last positional argument or from stdin; remove --{name}"
                )
            } else {
                format!(
                    "\"{path}\" reads {field} only from stdin; remove --{name} and use a quoted heredoc"
                )
            });
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
