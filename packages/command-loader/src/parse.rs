use crate::{Error, Leaf, Manifest, Node};
use serde_json::{Map, Number, Value};

pub struct Selected<'a> {
    pub node: &'a Node,
    pub path: Vec<String>,
    argument_index: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct Parsed {
    pub path: Vec<String>,
    pub values: Map<String, Value>,
    pub json: bool,
    pub help: bool,
}

impl Manifest {
    /// The raw argv excludes the root executable name.
    pub fn select<'a>(&'a self, root: &str, argv: &[String]) -> Result<Selected<'a>, Error> {
        let mut node = &self
            .roots
            .get(root)
            .ok_or_else(|| Error::Invalid(format!("{root}: not a root command of this manifest")))?
            .tree;
        let mut path = vec![root.to_owned()];
        let mut index = 0;
        while let Node::Variant0(group) = node {
            let Some(token) = argv.get(index) else {
                break;
            };
            if token == "--help" {
                break;
            }
            node = group
                .subcommands
                .iter()
                .find(|child| child.name() == token)
                .ok_or_else(|| {
                    Error::Invalid(format!("Unknown subcommand \"{} {token}\"", path.join(" ")))
                })?;
            path.push(node.name().to_owned());
            index += 1;
        }
        Ok(Selected {
            node,
            path,
            argument_index: index,
        })
    }
}

impl Selected<'_> {
    /// Parses argv without reading stdin. Help therefore never consumes a body.
    pub fn parse(&self, argv: &[String]) -> Result<Parsed, Error> {
        let mut result = Parsed {
            path: self.path.clone(),
            values: Map::new(),
            json: false,
            help: false,
        };
        let Some(leaf) = self.node.leaf() else {
            result.help = true;
            return Ok(result);
        };
        let properties = leaf.properties();
        let mut index = self.argument_index;
        let mut positional = 0;
        let mut options_ended = false;
        while let Some(token) = argv.get(index) {
            index += 1;
            if !options_ended && token == "--" {
                if let Some(field) = leaf.rest_field() {
                    result.values.insert(
                        field.clone(),
                        Value::Array(argv[index..].iter().cloned().map(Value::String).collect()),
                    );
                    break;
                }
                options_ended = true;
                continue;
            }
            if !options_ended && token == "--help" {
                result.help = true;
                return Ok(result);
            }
            if !options_ended && token == "--json" {
                if leaf.json_output().is_none() {
                    return Err(Error::Invalid(format!(
                        "Command \"{}\" does not define JSON output",
                        self.path.join(" ")
                    )));
                }
                result.json = true;
                continue;
            }
            if !options_ended && let Some(option) = token.strip_prefix("--") {
                let (field, inline) = option
                    .split_once('=')
                    .map_or((option, None), |(name, value)| (name, Some(value)));
                let schema = properties
                    .and_then(|properties| properties.get(field))
                    .ok_or_else(|| Error::Invalid(format!("Unknown option \"--{field}\"")))?;
                if leaf.stdin_field().as_deref() == Some(field) {
                    return Err(Error::Invalid(format!(
                        "\"{}\" reads {field} only from stdin. Remove --{field} and use a quoted heredoc, pipe, or input redirection.",
                        self.path.join(" ")
                    )));
                }
                if leaf.rest_field().as_deref() == Some(field)
                    || leaf
                        .positionals()
                        .as_ref()
                        .is_some_and(|fields| fields.iter().any(|candidate| candidate == field))
                {
                    return Err(Error::Invalid(format!("\"{field}\" is not an option")));
                }
                let value = if let Some(value) = inline {
                    Value::String(value.into())
                } else if schema_type(schema) == Some("boolean")
                    && argv.get(index).is_none_or(|value| value.starts_with("--"))
                {
                    Value::Bool(true)
                } else {
                    let value = argv
                        .get(index)
                        .filter(|value| !value.starts_with("--"))
                        .ok_or_else(|| {
                            Error::Invalid(format!("Missing value for \"--{field}\""))
                        })?;
                    index += 1;
                    Value::String(value.clone())
                };
                set_value(&mut result.values, field, value, schema)?;
                continue;
            }
            let field = leaf
                .positionals()
                .as_ref()
                .and_then(|fields| fields.get(positional))
                .ok_or_else(|| {
                    Error::Invalid(format!("Unexpected positional argument \"{token}\""))
                })?;
            positional += 1;
            let schema = &properties.expect("validated positional schema")[field];
            set_value(
                &mut result.values,
                field,
                Value::String(token.clone()),
                schema,
            )?;
        }
        Ok(result)
    }
}

impl Parsed {
    /// Adds the explicitly consumed body, applies declared defaults and validates.
    pub fn validate(mut self, leaf: &Leaf, stdin: Option<String>) -> Result<Self, Error> {
        if self.help {
            return Ok(self);
        }
        if let Some(field) = leaf.stdin_field() {
            self.values.insert(
                field.clone(),
                Value::String(stdin.ok_or_else(|| {
                    Error::Invalid("stdin field was not supplied by dispatcher".into())
                })?),
            );
        } else if stdin.is_some() {
            return Err(Error::Invalid(
                "stdin body supplied to a leaf without stdinField".into(),
            ));
        }
        if let Some(properties) = leaf.properties() {
            for (field, schema) in properties {
                if let Some(value) = self.values.remove(field) {
                    self.values.insert(field.clone(), coerce(value, schema)?);
                } else if let Some(default) = schema.get("default") {
                    self.values.insert(field.clone(), default.clone());
                }
            }
        }
        if let Some(schema) = leaf.input() {
            let validator = jsonschema::validator_for(&serde_json::to_value(schema)?)
                .map_err(|error| Error::Invalid(error.to_string()))?;
            validator
                .validate(&Value::Object(self.values.clone()))
                .map_err(|error| Error::Invalid(format!("Invalid command arguments: {error}")))?;
        }
        Ok(self)
    }
}

fn schema_type(schema: &Value) -> Option<&str> {
    schema.get("type").and_then(Value::as_str)
}

fn set_value(
    values: &mut Map<String, Value>,
    field: &str,
    value: Value,
    schema: &Value,
) -> Result<(), Error> {
    if let Some(previous) = values.get_mut(field) {
        if schema_type(schema) != Some("array") {
            return Err(Error::Invalid(format!("Duplicate value for \"{field}\"")));
        }
        if let Value::Array(values) = previous {
            values.push(value);
        } else {
            let first = previous.take();
            *previous = Value::Array(vec![first, value]);
        }
    } else {
        values.insert(field.to_owned(), value);
    }
    Ok(())
}

fn coerce(value: Value, schema: &Value) -> Result<Value, Error> {
    if schema_type(schema) == Some("array") {
        return Ok(match value {
            Value::Array(_) => value,
            _ => Value::Array(vec![value]),
        });
    }
    if let Value::String(text) = &value {
        match schema_type(schema) {
            Some("number" | "integer") if !text.trim().is_empty() => {
                let number = text
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .and_then(|value| {
                        if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0 {
                            Some(Number::from(value as i64))
                        } else {
                            Number::from_f64(value)
                        }
                    })
                    .ok_or_else(|| Error::Invalid(format!("Invalid numeric argument: {text}")))?;
                return Ok(Value::Number(number));
            }
            Some("boolean") if text == "true" => return Ok(Value::Bool(true)),
            Some("boolean") if text == "false" => return Ok(Value::Bool(false)),
            _ => {}
        }
    }
    Ok(value)
}
