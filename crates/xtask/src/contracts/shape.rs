//! The subset of JSON Schema the emitter translates (`contracts.md`
//! § Generated TypeScript): the schemas schemars writes for serde's
//! representation of the contract types, read into [`Shape`]s. Anything
//! outside the subset is an [`Unsupported`] error that names the place.

use serde_json::{Map, Value};

/// Core's `MAX_SAFE_INTEGER`, the bound of every integer the browser reads,
/// as a signed bound.
pub const MAX_SAFE_INTEGER: i64 = demi_core::MAX_SAFE_INTEGER as i64;

/// Keywords that describe a schema without constraining it. schemars writes
/// `default` for a field serde fills in when it is absent, which the field's
/// absence from `required` already says.
const ANNOTATIONS: [&str; 8] = [
    "description",
    "title",
    "default",
    "examples",
    "deprecated",
    "readOnly",
    "writeOnly",
    "$comment",
];

/// Where a schema's definitions are, as schemars writes a reference.
const DEFINITIONS: &str = "#/$defs/";

/// One schema of the subset.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A named definition.
    Ref(String),
    String(StringShape),
    /// An integer within its bounds, which lie within JavaScript's safe range.
    Integer { min: i64, max: i64 },
    Number { min: Option<Bound>, max: Option<Bound> },
    Boolean,
    /// One exact string, such as a variant's tag.
    Literal(String),
    /// A closed set of strings.
    Enum(Vec<String>),
    Array {
        items: Box<Shape>,
        min: Option<u64>,
        max: Option<u64>,
    },
    /// An object whose keys are data, such as failure facts by block id.
    Record(Box<Shape>),
    Object(Object),
    /// An internally tagged enum: each variant an object with the tag.
    Union { tag: String, variants: Vec<Variant> },
    /// A value or `null`.
    Nullable(Box<Shape>),
    /// Any JSON value.
    Json,
}

/// A string and its rules.
#[derive(Debug, Clone, PartialEq)]
pub struct StringShape {
    pub format: StringFormat,
    pub min_length: Option<u64>,
    pub max_length: Option<u64>,
    pub pattern: Option<String>,
}

/// What a string holds beyond its length and pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringFormat {
    Text,
    /// Text web-api's `Trimmed` trims when it arrives: its bounds count what
    /// remains (format `trimmed`).
    Trimmed,
    /// A time as core's `Timestamp` writes it.
    DateTime,
    Email,
    /// An `http` or `https` URL, as web-api's `EndpointUrl` reads it (format
    /// `http-url`).
    HttpUrl,
    /// Bytes as core's `B64Bytes` writes them.
    Base64,
}

/// A bound of a number, and whether the bound itself is excluded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bound {
    pub value: f64,
    pub exclusive: bool,
}

/// An object's fields, in the order serde writes them.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub properties: Vec<Property>,
    /// Whether the Rust type refuses unknown fields (`additionalProperties:
    /// false`).
    pub denies_unknown: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    pub shape: Shape,
    pub required: bool,
    pub description: Option<String>,
}

/// One variant of an internally tagged enum: its own fields, or, for a
/// newtype variant of a struct schemars refers to by name, that struct with
/// the tag added.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub extends: Option<String>,
    pub object: Object,
    pub description: Option<String>,
}

/// A schema outside the emitter's subset, and where it is.
#[derive(Debug, thiserror::Error)]
#[error("{at}: {problem}")]
pub struct Unsupported {
    pub at: String,
    pub problem: String,
}

fn unsupported(at: &str, problem: impl Into<String>) -> Unsupported {
    Unsupported {
        at: at.to_owned(),
        problem: problem.into(),
    }
}

/// A schema's description, if it has one.
pub fn description(schema: &Value) -> Option<String> {
    schema.get("description").and_then(Value::as_str).map(str::to_owned)
}

/// The definition a `$ref` names.
fn reference(value: &Value, at: &str) -> Result<String, Unsupported> {
    let text = value
        .as_str()
        .ok_or_else(|| unsupported(at, "a `$ref` that is not a string"))?;
    let name = text
        .strip_prefix(DEFINITIONS)
        .ok_or_else(|| unsupported(at, format!("a reference outside the definitions: {text}")))?;
    Ok(name.to_owned())
}

/// The keywords of `schema` that constrain it, in order.
fn keywords(schema: &Map<String, Value>) -> Vec<&str> {
    schema
        .keys()
        .map(String::as_str)
        .filter(|key| !ANNOTATIONS.contains(key))
        .collect()
}

/// Refuses every keyword of `schema` outside `allowed` and the annotations.
fn only(schema: &Map<String, Value>, at: &str, allowed: &[&str]) -> Result<(), Unsupported> {
    match keywords(schema).into_iter().find(|key| !allowed.contains(key)) {
        Some(key) => Err(unsupported(at, format!("the keyword `{key}` in this schema"))),
        None => Ok(()),
    }
}

fn count(schema: &Map<String, Value>, key: &str, at: &str) -> Result<Option<u64>, Unsupported> {
    let Some(value) = schema.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| unsupported(at, format!("`{key}` that is not a whole number")))
}

/// Reads one schema.
pub fn read(schema: &Value, at: &str) -> Result<Shape, Unsupported> {
    match schema {
        Value::Bool(true) => Ok(Shape::Json),
        Value::Object(map) => read_map(map, at),
        other => Err(unsupported(at, format!("the schema {other}"))),
    }
}

fn read_map(schema: &Map<String, Value>, at: &str) -> Result<Shape, Unsupported> {
    if let Some(target) = schema.get("$ref") {
        only(schema, at, &["$ref"])?;
        return Ok(Shape::Ref(reference(target, at)?));
    }
    if let Some(options) = schema.get("anyOf") {
        only(schema, at, &["anyOf"])?;
        return read_nullable_any_of(options, at);
    }
    if let Some(options) = schema.get("oneOf") {
        only(schema, at, &["oneOf"])?;
        return read_one_of(options, &format!("{at}.oneOf"));
    }
    match schema.get("type") {
        Some(Value::String(kind)) => read_typed(schema, kind, at),
        Some(Value::Array(kinds)) => read_nullable_type(schema, kinds, at),
        Some(other) => Err(unsupported(at, format!("the type {other}"))),
        None => Err(unsupported(at, "a schema without a type")),
    }
}

/// `anyOf` of a schema and `null`, as schemars writes a nullable reference.
fn read_nullable_any_of(options: &Value, at: &str) -> Result<Shape, Unsupported> {
    let null = serde_json::json!({ "type": "null" });
    let options = options
        .as_array()
        .ok_or_else(|| unsupported(at, "`anyOf` that is not a list"))?;
    match options.as_slice() {
        [value, other] | [other, value] if *other == null && *value != null => {
            Ok(Shape::Nullable(Box::new(read(value, &format!("{at}.anyOf"))?)))
        }
        _ => Err(unsupported(at, "`anyOf` other than a value or null (an untagged enum)")),
    }
}

/// `"type": [T, "null"]`, as schemars writes a nullable value of a simple
/// type.
fn read_nullable_type(schema: &Map<String, Value>, kinds: &[Value], at: &str) -> Result<Shape, Unsupported> {
    let kind = match kinds {
        [Value::String(kind), Value::String(null)] | [Value::String(null), Value::String(kind)]
            if null == "null" && kind != "null" =>
        {
            kind
        }
        _ => return Err(unsupported(at, format!("the types {kinds:?}"))),
    };
    let inner = read_typed(schema, kind, at)?;
    Ok(Shape::Nullable(Box::new(inner)))
}

fn read_typed(schema: &Map<String, Value>, kind: &str, at: &str) -> Result<Shape, Unsupported> {
    match kind {
        "string" => read_string(schema, at),
        "integer" => read_integer(schema, at),
        "number" => read_number(schema, at),
        "boolean" => {
            only(schema, at, &["type"])?;
            Ok(Shape::Boolean)
        }
        "array" => read_array(schema, at),
        "object" => read_object(schema, at),
        other => Err(unsupported(at, format!("the type `{other}` on its own"))),
    }
}

fn read_string(schema: &Map<String, Value>, at: &str) -> Result<Shape, Unsupported> {
    if let Some(value) = schema.get("const") {
        only(schema, at, &["type", "const"])?;
        let text = value
            .as_str()
            .ok_or_else(|| unsupported(at, "a string `const` that is not a string"))?;
        return Ok(Shape::Literal(text.to_owned()));
    }
    if let Some(values) = schema.get("enum") {
        only(schema, at, &["type", "enum"])?;
        return Ok(Shape::Enum(strings(values, at)?));
    }
    only(
        schema,
        at,
        &["type", "minLength", "maxLength", "pattern", "format", "contentEncoding"],
    )?;
    let format = match (
        schema.get("format").and_then(Value::as_str),
        schema.get("contentEncoding").and_then(Value::as_str),
    ) {
        (None, None) => StringFormat::Text,
        (Some("trimmed"), None) => StringFormat::Trimmed,
        (Some("date-time"), None) => StringFormat::DateTime,
        (Some("email"), None) => StringFormat::Email,
        (Some("http-url"), None) => StringFormat::HttpUrl,
        (None, Some("base64")) => StringFormat::Base64,
        (format, encoding) => {
            return Err(unsupported(
                at,
                format!("the string format {format:?} with the encoding {encoding:?}"),
            ));
        }
    };
    let pattern = match schema.get("pattern") {
        None => None,
        Some(Value::String(pattern)) => {
            check_pattern(pattern).map_err(|problem| unsupported(at, problem))?;
            Some(pattern.clone())
        }
        Some(other) => return Err(unsupported(at, format!("the pattern {other}"))),
    };
    Ok(Shape::String(StringShape {
        format,
        min_length: count(schema, "minLength", at)?,
        max_length: count(schema, "maxLength", at)?,
        pattern,
    }))
}

/// Every value of an `enum`, which must be strings.
fn strings(values: &Value, at: &str) -> Result<Vec<String>, Unsupported> {
    let values = values
        .as_array()
        .ok_or_else(|| unsupported(at, "an `enum` that is not a list"))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| unsupported(at, format!("the enum value {value}, which is not a string")))
        })
        .collect()
}

/// Refuses the constructs Rust's `regex` and ECMAScript read differently,
/// so that a pattern matches the same strings at both ends: the class
/// escapes `\d`, `\w`, `\s` and `\b` (Unicode in Rust, ASCII in the
/// browser), `.` (which excludes different line breaks), groups other than
/// `(?:`, Unicode properties, Rust's anchors `\A` and `\z`, and what Rust
/// reads as class syntax: a `[` inside a class (a nested class or
/// `[:alpha:]`), `&&`, `--` and `~~`, and a `]` that opens a class (a
/// literal in Rust, an empty class in the browser).
fn check_pattern(pattern: &str) -> Result<(), String> {
    let refuse = |what: &str| Err(format!("the pattern {pattern:?} uses {what}, which Rust and the browser read differently"));
    let characters: Vec<char> = pattern.chars().collect();
    let mut in_class = false;
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        let next = characters.get(index + 1).copied();
        match character {
            '\\' => {
                if let Some(escaped @ ('d' | 'D' | 'w' | 'W' | 's' | 'S' | 'b' | 'B' | 'p' | 'P' | 'A' | 'z' | 'Z')) = next {
                    return refuse(&format!("\\{escaped}"));
                }
                index += 2;
                continue;
            }
            '[' if in_class => return refuse("a `[` inside a class"),
            '[' => {
                let first = if next == Some('^') { characters.get(index + 2) } else { next.as_ref() };
                if first == Some(&']') {
                    return refuse("a `]` that opens a class");
                }
                in_class = true;
            }
            ']' if in_class => in_class = false,
            '&' | '-' | '~' if in_class && next == Some(character) => return refuse("class set operations"),
            '.' if !in_class => return refuse("`.`"),
            '(' if !in_class && next == Some('?') && characters.get(index + 2) != Some(&':') => {
                return refuse("a group other than `(?:`");
            }
            _ => {}
        }
        index += 1;
    }
    Ok(())
}

/// The bounds a schemars integer format gives its type, before the
/// schema's own `minimum` and `maximum`. A 64-bit type's reach past `i64`
/// changes nothing: it lies beyond JavaScript's safe range either way.
fn format_bounds(format: Option<&str>, at: &str) -> Result<(i64, i64), Unsupported> {
    let bounds = match format {
        Some("int8") => (i64::from(i8::MIN), i64::from(i8::MAX)),
        Some("int16") => (i64::from(i16::MIN), i64::from(i16::MAX)),
        Some("int32") => (i64::from(i32::MIN), i64::from(i32::MAX)),
        Some("int64" | "int") | None => (i64::MIN, i64::MAX),
        Some("uint8") => (0, i64::from(u8::MAX)),
        Some("uint16") => (0, i64::from(u16::MAX)),
        Some("uint32") => (0, i64::from(u32::MAX)),
        Some("uint64" | "uint") => (0, i64::MAX),
        Some(other) => return Err(unsupported(at, format!("the integer format `{other}`"))),
    };
    Ok(bounds)
}

/// An integer bound of the schema, which must be whole.
fn integer_bound(schema: &Map<String, Value>, key: &str, at: &str) -> Result<Option<i64>, Unsupported> {
    let Some(value) = schema.get(key) else {
        return Ok(None);
    };
    if let Some(whole) = value.as_i64() {
        return Ok(Some(whole));
    }
    // Beyond `i64` lies beyond the safe range the bounds must keep to anyway.
    if value.as_u64().is_some() {
        return Ok(Some(i64::MAX));
    }
    match value.as_f64() {
        Some(number) if number.fract() == 0.0 => Ok(Some(number.clamp(i64::MIN as f64, i64::MAX as f64) as i64)),
        _ => Err(unsupported(at, format!("an integer `{key}` of {value}"))),
    }
}

fn read_integer(schema: &Map<String, Value>, at: &str) -> Result<Shape, Unsupported> {
    only(
        schema,
        at,
        &["type", "format", "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"],
    )?;
    let (mut min, mut max) = format_bounds(schema.get("format").and_then(Value::as_str), at)?;
    if let Some(minimum) = integer_bound(schema, "minimum", at)? {
        min = min.max(minimum);
    }
    if let Some(exclusive) = integer_bound(schema, "exclusiveMinimum", at)? {
        min = min.max(exclusive.saturating_add(1));
    }
    if let Some(maximum) = integer_bound(schema, "maximum", at)? {
        max = max.min(maximum);
    }
    if let Some(exclusive) = integer_bound(schema, "exclusiveMaximum", at)? {
        max = max.min(exclusive.saturating_sub(1));
    }
    // An integer the browser reads is bounded to JavaScript's safe range
    // (`contracts.md` § Encoding conventions), in the Rust type too, so that
    // an end that decodes it refuses what the browser cannot hold.
    if min < -MAX_SAFE_INTEGER || max > MAX_SAFE_INTEGER {
        return Err(unsupported(
            at,
            format!("an integer from {min} to {max}, beyond JavaScript's safe range: bound it with garde's `range` to `MAX_SAFE_INTEGER`"),
        ));
    }
    if min > max {
        return Err(unsupported(at, format!("an integer between {min} and {max}")));
    }
    Ok(Shape::Integer { min, max })
}

fn number_bound(schema: &Map<String, Value>, inclusive: &str, exclusive: &str, at: &str) -> Result<Option<Bound>, Unsupported> {
    let read = |key: &str| {
        schema
            .get(key)
            .map(|value| value.as_f64().ok_or_else(|| unsupported(at, format!("`{key}` of {value}"))))
            .transpose()
    };
    match (read(inclusive)?, read(exclusive)?) {
        (Some(_), Some(_)) => Err(unsupported(at, format!("both `{inclusive}` and `{exclusive}`"))),
        (Some(value), None) => Ok(Some(Bound { value, exclusive: false })),
        (None, Some(value)) => Ok(Some(Bound { value, exclusive: true })),
        (None, None) => Ok(None),
    }
}

fn read_number(schema: &Map<String, Value>, at: &str) -> Result<Shape, Unsupported> {
    only(
        schema,
        at,
        &["type", "format", "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"],
    )?;
    match schema.get("format").and_then(Value::as_str) {
        None | Some("double") => {}
        Some(other) => return Err(unsupported(at, format!("the number format `{other}`"))),
    }
    Ok(Shape::Number {
        min: number_bound(schema, "minimum", "exclusiveMinimum", at)?,
        max: number_bound(schema, "maximum", "exclusiveMaximum", at)?,
    })
}

fn read_array(schema: &Map<String, Value>, at: &str) -> Result<Shape, Unsupported> {
    only(schema, at, &["type", "items", "minItems", "maxItems"])?;
    let items = schema
        .get("items")
        .ok_or_else(|| unsupported(at, "an array without `items`"))?;
    Ok(Shape::Array {
        items: Box::new(read(items, &format!("{at}.items"))?),
        min: count(schema, "minItems", at)?,
        max: count(schema, "maxItems", at)?,
    })
}

fn read_object(schema: &Map<String, Value>, at: &str) -> Result<Shape, Unsupported> {
    only(schema, at, &["type", "properties", "required", "additionalProperties"])?;
    match schema.get("additionalProperties") {
        None | Some(Value::Bool(false)) => Ok(Shape::Object(read_fields(schema, at)?)),
        Some(values) if !schema.contains_key("properties") => {
            Ok(Shape::Record(Box::new(read(values, &format!("{at}.additionalProperties"))?)))
        }
        Some(_) => Err(unsupported(at, "an object with both fields and data keys")),
    }
}

/// An object's `properties`, `required` and `additionalProperties`.
fn read_fields(schema: &Map<String, Value>, at: &str) -> Result<Object, Unsupported> {
    let empty = Map::new();
    let properties = match schema.get("properties") {
        None => &empty,
        Some(Value::Object(properties)) => properties,
        Some(other) => return Err(unsupported(at, format!("the properties {other}"))),
    };
    let required = match schema.get("required") {
        None => Vec::new(),
        Some(names) => strings(names, &format!("{at}.required"))?,
    };
    if let Some(unknown) = required.iter().find(|name| !properties.contains_key(*name)) {
        return Err(unsupported(at, format!("the required field `{unknown}`, which has no schema")));
    }
    let properties = properties
        .iter()
        .map(|(name, property)| {
            Ok(Property {
                name: name.clone(),
                shape: read(property, &format!("{at}.{name}"))?,
                required: required.contains(name),
                description: description(property),
            })
        })
        .collect::<Result<_, Unsupported>>()?;
    Ok(Object {
        properties,
        denies_unknown: schema.get("additionalProperties") == Some(&Value::Bool(false)),
    })
}

/// `oneOf`: a closed set of strings (schemars lists a documented unit
/// variant on its own), or an internally tagged enum.
fn read_one_of(options: &Value, at: &str) -> Result<Shape, Unsupported> {
    let options = options
        .as_array()
        .ok_or_else(|| unsupported(at, "`oneOf` that is not a list"))?;
    if let Some(values) = string_set(options) {
        return Ok(Shape::Enum(values));
    }
    let variants = options
        .iter()
        .enumerate()
        .map(|(index, option)| read_variant(option, &format!("{at}[{index}]")))
        .collect::<Result<Vec<_>, _>>()?;
    let tag = discriminator(&variants).ok_or_else(|| {
        unsupported(
            at,
            "variants without one required string tag they all carry (an externally tagged or untagged enum)",
        )
    })?;
    Ok(Shape::Union { tag, variants })
}

/// The values of a `oneOf` whose every option is a string `const` or
/// `enum`.
fn string_set(options: &[Value]) -> Option<Vec<String>> {
    let mut values = Vec::new();
    for option in options {
        let option = option.as_object()?;
        if option.get("type").and_then(Value::as_str) != Some("string") {
            return None;
        }
        let keys = keywords(option);
        if let Some(value) = option.get("const").and_then(Value::as_str) {
            if keys.len() != 2 {
                return None;
            }
            values.push(value.to_owned());
        } else if let Some(Value::Array(set)) = option.get("enum") {
            if keys.len() != 2 {
                return None;
            }
            for value in set {
                values.push(value.as_str()?.to_owned());
            }
        } else {
            return None;
        }
    }
    Some(values)
}

fn read_variant(option: &Value, at: &str) -> Result<Variant, Unsupported> {
    let map = option
        .as_object()
        .ok_or_else(|| unsupported(at, format!("the variant {option}")))?;
    let description = description(option);
    if let Some(target) = map.get("$ref") {
        only(map, at, &["$ref", "type", "properties", "required"])?;
        if map.get("type").and_then(Value::as_str) != Some("object") {
            return Err(unsupported(at, "a referring variant that is not an object"));
        }
        return Ok(Variant {
            extends: Some(reference(target, at)?),
            object: read_fields(map, at)?,
            description,
        });
    }
    match read(option, at)? {
        Shape::Object(object) => Ok(Variant {
            extends: None,
            object,
            description,
        }),
        _ => Err(unsupported(at, "a variant that is not an object")),
    }
}

/// The one field every variant requires as a distinct string literal.
fn discriminator(variants: &[Variant]) -> Option<String> {
    let tag_values = |variant: &Variant, name: &str| {
        variant
            .object
            .properties
            .iter()
            .find(|property| property.name == name && property.required)
            .and_then(|property| match &property.shape {
                Shape::Literal(value) => Some(value.clone()),
                _ => None,
            })
    };
    let first = variants.first()?;
    let mut candidates = first.object.properties.iter().filter_map(|property| {
        let values: Option<Vec<String>> = variants.iter().map(|variant| tag_values(variant, &property.name)).collect();
        let values = values?;
        let distinct = values
            .iter()
            .enumerate()
            .all(|(index, value)| !values[..index].contains(value));
        distinct.then(|| property.name.clone())
    });
    let tag = candidates.next()?;
    // Two fields that could each be the tag leave the choice to guesswork.
    candidates.next().is_none().then_some(tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_untagged_enum_and_keywords_outside_the_subset_are_refused() {
        let untagged = json!({ "oneOf": [{ "type": "string" }, { "type": "number" }] });
        let error = read(&untagged, "NodeValue").unwrap_err();
        assert!(error.to_string().starts_with("NodeValue.oneOf"), "{error}");
        for refused in [
            json!({ "anyOf": [{ "type": "string" }, { "type": "number" }] }),
            json!({ "type": "array", "items": { "type": "string" }, "uniqueItems": true }),
            json!({ "type": "object", "properties": { "a": { "type": "string" } }, "additionalProperties": { "type": "string" } }),
            json!({ "allOf": [{ "$ref": "#/$defs/A" }] }),
            json!({ "type": "string", "format": "ipv4" }),
            json!({ "type": "string", "pattern": "^\\d+$" }),
            json!({ "type": "string", "pattern": "^a.b$" }),
            json!({ "type": "string", "pattern": "^(?i)a$" }),
            json!({ "type": "string", "pattern": "^[a[b]]$" }),
            json!({ "type": "string", "pattern": "^[^]a]$" }),
            json!({ "$ref": "#/$defs/A", "maxLength": 3 }),
            json!({ "type": "integer", "format": "uint128" }),
        ] {
            assert!(read(&refused, "t").is_err(), "{refused}");
        }
        assert!(read(&json!({ "type": "string", "pattern": "^[^./\\\\\\x00][a-z.]*(?:x|y)$" }), "t").is_ok());
    }

    #[test]
    fn an_integer_the_rust_type_leaves_beyond_javascripts_safe_range_is_refused() {
        for (unbounded, place) in [
            (json!({ "type": "integer", "format": "uint64", "minimum": 0 }), "an integer from 0 to"),
            (json!({ "type": "integer", "format": "int64", "maximum": 5 }), "an integer from -"),
        ] {
            let error = read(&unbounded, "Summary.revision").unwrap_err().to_string();
            assert!(error.starts_with(&format!("Summary.revision: {place}")), "{error}");
        }
        let bounded = json!({ "type": "integer", "format": "uint64", "minimum": 0, "maximum": MAX_SAFE_INTEGER });
        assert_eq!(read(&bounded, "t").unwrap(), Shape::Integer { min: 0, max: MAX_SAFE_INTEGER });
        let narrow = json!({ "type": "integer", "format": "uint32", "minimum": 0 });
        assert_eq!(read(&narrow, "t").unwrap(), Shape::Integer { min: 0, max: i64::from(u32::MAX) });
    }
}
