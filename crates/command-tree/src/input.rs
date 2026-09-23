//! The command input subset (`commands.md` § The input subset): the JSON
//! Schema forms a command's input may take, checked once when the command is
//! declared, and the settings every declaration's schema is generated with.

use std::collections::HashSet;

use schemars::{
    Schema as JsonSchema,
    generate::SchemaSettings,
    transform::{Transform, transform_subschemas},
};
use serde_json::{Map, Value};

use crate::{DeclarationError, Schema, invalid};

/// A command's input as a table of fields, in declaration order. A schema
/// outside the subset has none: [`InputSpec::from_schema`] refuses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSpec {
    fields: Vec<InputField>,
}

/// One field of a command's input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputField {
    pub name: String,
    pub kind: FieldKind,
    pub required: bool,
}

/// What values a field takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    String,
    Number,
    Integer,
    Boolean,
    /// One of these strings.
    Enum(Vec<String>),
    /// Values of one scalar kind, repeated.
    Array(Box<FieldKind>),
}

/// What an input object may say besides its fields.
const OBJECT_KEYWORDS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "title",
    "description",
];

/// What a field's schema may say.
const FIELD_KEYWORDS: &[&str] = &[
    "type",
    "enum",
    "items",
    "description",
    "format",
    "minLength",
    "maxLength",
    "pattern",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "minItems",
    "maxItems",
];

impl InputSpec {
    /// Checks that `schema` is inside the input subset: an object that allows
    /// no other properties, whose fields are strings, numbers, integers,
    /// booleans, string enums or arrays of one of those, each with only the
    /// bounds the subset names. A refusal names the field and why.
    pub fn from_schema(schema: &Schema) -> Result<Self, DeclarationError> {
        let object = schema.value();
        for keyword in object.keys() {
            if !OBJECT_KEYWORDS.contains(&keyword.as_str()) {
                return Err(invalid(format!(
                    "command input uses \"{keyword}\", outside the command input subset"
                )));
            }
        }
        if object.get("type").and_then(Value::as_str) != Some("object") {
            return Err(invalid("command input must describe an object".into()));
        }
        if object.get("additionalProperties") != Some(&Value::Bool(false)) {
            return Err(invalid(
                "command input must refuse unknown fields (additionalProperties: false)".into(),
            ));
        }
        let empty = Map::new();
        let properties = match object.get("properties") {
            None => &empty,
            Some(Value::Object(properties)) => properties,
            Some(_) => return Err(invalid("command input properties must be an object".into())),
        };
        let required: HashSet<&str> = match object.get("required") {
            None => HashSet::new(),
            Some(Value::Array(names)) => names
                .iter()
                .map(|name| {
                    name.as_str()
                        .filter(|name| properties.contains_key(*name))
                        .ok_or_else(|| {
                            invalid(format!("command input requires an undeclared field {name}"))
                        })
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(invalid("command input required must be a list".into())),
        };
        let fields = properties
            .iter()
            .map(|(name, schema)| {
                let kind = field_kind(schema)
                    .map_err(|reason| invalid(format!("input \"{name}\": {reason}")))?;
                Ok(InputField {
                    name: name.clone(),
                    kind,
                    required: required.contains(name.as_str()),
                })
            })
            .collect::<Result<_, DeclarationError>>()?;
        Ok(Self { fields })
    }

    pub fn fields(&self) -> &[InputField] {
        &self.fields
    }
}

/// The kind of a field's schema, or why the subset refuses it.
fn field_kind(schema: &Value) -> Result<FieldKind, String> {
    let Value::Object(schema) = schema else {
        return Err("must be a schema object".into());
    };
    if schema.contains_key("default") {
        return Err("carries a default; a missing value is the handler's to supply".into());
    }
    if ["oneOf", "anyOf", "allOf"]
        .iter()
        .any(|keyword| schema.contains_key(*keyword))
    {
        return Err("is a union, which has no command-line form".into());
    }
    if schema.contains_key("$ref") {
        return Err("refers to another schema".into());
    }
    if schema.contains_key("properties") {
        return Err("is a nested object, which has no command-line form".into());
    }
    if let Some(keyword) = schema
        .keys()
        .find(|keyword| !FIELD_KEYWORDS.contains(&keyword.as_str()))
    {
        return Err(format!(
            "uses \"{keyword}\", outside the command input subset"
        ));
    }
    let kind = match schema.get("type") {
        Some(Value::String(kind)) => kind.as_str(),
        Some(Value::Array(kinds)) if kinds.iter().any(|kind| kind == "null") => {
            return Err("allows null; an optional field is absent instead".into());
        }
        Some(Value::Array(_)) => return Err("has several types".into()),
        _ => return Err("declares no type".into()),
    };
    if schema.contains_key("format") && !matches!(kind, "integer" | "number") {
        return Err("carries a format, which only a number's schema may".into());
    }
    if schema.contains_key("items") && kind != "array" {
        return Err("has items but is not an array".into());
    }
    match kind {
        "string" => match schema.get("enum") {
            None => Ok(FieldKind::String),
            Some(Value::Array(values)) => values
                .iter()
                .map(|value| match value {
                    Value::String(value) => Ok(value.clone()),
                    Value::Null => Err("allows null; an optional field is absent instead".into()),
                    _ => Err("is an enum of values that are not all strings".to_owned()),
                })
                .collect::<Result<_, _>>()
                .map(FieldKind::Enum),
            Some(_) => Err("has an enum that is not a list".into()),
        },
        _ if schema.contains_key("enum") => Err("is an enum of values that are not strings".into()),
        "number" => Ok(FieldKind::Number),
        "integer" => Ok(FieldKind::Integer),
        "boolean" => Ok(FieldKind::Boolean),
        "array" => {
            let items = schema
                .get("items")
                .ok_or_else(|| "is an array without items".to_owned())?;
            if items.get("type").and_then(Value::as_str) == Some("array") {
                return Err("is an array of arrays, which has no command-line form".into());
            }
            let element = field_kind(items).map_err(|reason| format!("its items: {reason}"))?;
            Ok(FieldKind::Array(Box::new(element)))
        }
        "object" => Err("is a nested object, which has no command-line form".into()),
        other => Err(format!("has type \"{other}\", outside the command input subset")),
    }
}

/// The settings every declaration's JSON Schema is generated with: draft
/// 2020-12 without a `$schema` keyword, every subschema inline so a field is
/// never a `$ref`, and optional properties that never allow null. Input
/// schemas use them for deserialization, the default contract; a `--json`
/// output schema uses them with `for_serialize()`.
pub fn command_schema_settings() -> SchemaSettings {
    SchemaSettings::draft2020_12()
        .with(|settings| {
            settings.meta_schema = None;
            settings.inline_subschemas = true;
        })
        .with_transform(OptionalNeverNull)
}

/// An optional property is absent when it has no value, never null: argv has
/// no spelling for null, and an output leaves such a field out. schemars lets
/// every `Option` field allow null, with no setting to turn that off, so this
/// transform takes null out of the properties an object does not require.
#[derive(Clone)]
struct OptionalNeverNull;

impl Transform for OptionalNeverNull {
    fn transform(&mut self, schema: &mut JsonSchema) {
        transform_subschemas(self, schema);
        let Some(object) = schema.as_object_mut() else {
            return;
        };
        let required: HashSet<String> = object
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|name| name.as_str().map(str::to_owned))
            .collect();
        let Some(Value::Object(properties)) = object.get_mut("properties") else {
            return;
        };
        for (name, property) in properties.iter_mut() {
            if !required.contains(name) {
                refuse_null(property);
            }
        }
    }
}

/// Takes null out of a property's schema: from its types, from its enum, and
/// as the second branch of `anyOf`, which schemars writes for a schema it
/// cannot extend in place.
fn refuse_null(property: &mut Value) {
    let Value::Object(schema) = property else {
        return;
    };
    if let Some(Value::Array(kinds)) = schema.get_mut("type") {
        kinds.retain(|kind| kind != "null");
        if let [kind] = kinds.as_mut_slice() {
            let kind = kind.take();
            schema.insert("type".into(), kind);
        }
    }
    if let Some(Value::Array(values)) = schema.get_mut("enum") {
        values.retain(|value| !value.is_null());
    }
    let is_null = |value: &Value| value.get("type").and_then(Value::as_str) == Some("null");
    let kept = match schema.get("anyOf") {
        Some(Value::Array(branches)) if branches.len() == 2 => branches
            .iter()
            .position(is_null)
            .and_then(|null| branches[1 - null].as_object().cloned()),
        _ => None,
    };
    if let Some(kept) = kept {
        schema.remove("anyOf");
        for (keyword, value) in kept {
            schema.entry(keyword).or_insert(value);
        }
    }
}
