//! Writes Zod v4 source and `z.infer` types for the definitions of one
//! generated module (`contracts.md` § Generated TypeScript).

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use super::shape::{Bound, MAX_SAFE_INTEGER, Object, Shape, StringFormat, StringShape, Unsupported, Variant};

/// A definition the module declares.
pub struct Definition<'a> {
    pub name: &'a str,
    pub shape: &'a Shape,
    pub description: Option<&'a str>,
    /// Whether the browser receives the type, so that its objects are
    /// tolerant; otherwise they are strict.
    pub received: bool,
}

/// The name a definition's schema is exported under: `Block` is
/// `blockSchema`.
pub fn schema_name(definition: &str) -> String {
    let mut characters = definition.chars();
    let first = characters.next().map(|first| first.to_ascii_lowercase());
    first.into_iter().chain(characters).chain("Schema".chars()).collect()
}

/// The declarations of `definitions`, in their order, each followed by its
/// type. A reference to a definition declared later in the module, which
/// only a recursive type makes, is a getter of the object property it is in.
/// Answers the source and the definitions it refers to that are declared
/// elsewhere.
pub fn declarations(definitions: &[Definition<'_>]) -> Result<(String, BTreeSet<String>), Unsupported> {
    let order: HashMap<&str, usize> = definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| (definition.name, index))
        .collect();
    let mut imported = BTreeSet::new();
    let mut source = String::new();
    for (index, definition) in definitions.iter().enumerate() {
        let mut writer = Writer {
            order: &order,
            current: index,
            received: definition.received,
            imported: &mut imported,
        };
        let rendered = writer.shape(definition.shape, definition.name, 0)?;
        if rendered.forward {
            return Err(Unsupported {
                at: definition.name.to_owned(),
                problem: "a recursive reference outside an object property".into(),
            });
        }
        source.push('\n');
        push_doc(&mut source, definition.description, 0);
        let schema = schema_name(definition.name);
        writeln!(source, "export const {schema} = {}", rendered.code).expect("writing to a string");
        writeln!(source, "export type {} = z.infer<typeof {schema}>", definition.name).expect("writing to a string");
    }
    Ok((source, imported))
}

/// A rendered schema, and whether it refers to a definition not yet
/// declared.
struct Rendered {
    code: String,
    forward: bool,
}

impl Rendered {
    fn now(code: String) -> Self {
        Self { code, forward: false }
    }
}

struct Writer<'a> {
    order: &'a HashMap<&'a str, usize>,
    current: usize,
    received: bool,
    imported: &'a mut BTreeSet<String>,
}

impl Writer<'_> {
    fn shape(&mut self, shape: &Shape, at: &str, indent: usize) -> Result<Rendered, Unsupported> {
        let rendered = match shape {
            Shape::Ref(name) => self.reference(name),
            Shape::String(string) => Rendered::now(string_schema(string)),
            Shape::Integer { min, max } => Rendered::now(integer_schema(*min, *max)),
            Shape::Number { min, max } => Rendered::now(number_schema(*min, *max)),
            Shape::Boolean => Rendered::now("z.boolean()".into()),
            Shape::Literal(value) => Rendered::now(format!("z.literal({})", quote(value))),
            Shape::Enum(values) => {
                let values: Vec<String> = values.iter().map(|value| quote(value)).collect();
                Rendered::now(format!("z.enum([{}])", values.join(", ")))
            }
            Shape::Array { items, min, max } => {
                let items = self.shape(items, &format!("{at}[]"), indent)?;
                let mut code = format!("z.array({})", items.code);
                if let Some(min) = min {
                    write!(code, ".min({min})").expect("writing to a string");
                }
                if let Some(max) = max {
                    write!(code, ".max({max})").expect("writing to a string");
                }
                Rendered {
                    code,
                    forward: items.forward,
                }
            }
            Shape::Record(values) => {
                let values = self.shape(values, &format!("{at}{{}}"), indent)?;
                Rendered {
                    code: format!("z.record(z.string(), {})", values.code),
                    forward: values.forward,
                }
            }
            Shape::Object(object) => Rendered::now(self.object(object, None, at, indent)?),
            Shape::Union { tag, variants } => Rendered::now(self.union(tag, variants, at, indent)?),
            Shape::Nullable(inner) => {
                let inner = self.shape(inner, at, indent)?;
                Rendered {
                    code: format!("{}.nullable()", inner.code),
                    forward: inner.forward,
                }
            }
            Shape::Json => Rendered::now("z.json()".into()),
        };
        Ok(rendered)
    }

    fn reference(&mut self, name: &str) -> Rendered {
        let forward = match self.order.get(name) {
            Some(index) => *index >= self.current,
            None => {
                self.imported.insert(name.to_owned());
                false
            }
        };
        Rendered {
            code: schema_name(name),
            forward,
        }
    }

    /// An object: tolerant when the browser receives the type, strict
    /// otherwise, which requires the Rust type to refuse unknown fields.
    /// `tag` is written first when the object is a variant.
    fn object(&mut self, object: &Object, tag: Option<&str>, at: &str, indent: usize) -> Result<String, Unsupported> {
        let constructor = if self.received {
            "z.object"
        } else if object.denies_unknown {
            "z.strictObject"
        } else {
            return Err(Unsupported {
                at: at.to_owned(),
                problem: "only the browser sends this object, but it accepts unknown fields; a type the backend receives refuses them".into(),
            });
        };
        Ok(format!("{constructor}({})", self.fields(object, tag, at, indent)?))
    }

    /// An object's fields as the object literal Zod takes.
    fn fields(&mut self, object: &Object, tag: Option<&str>, at: &str, indent: usize) -> Result<String, Unsupported> {
        if object.properties.is_empty() {
            return Ok("{}".into());
        }
        let mut ordered: Vec<_> = object.properties.iter().collect();
        ordered.sort_by_key(|property| Some(property.name.as_str()) != tag);
        let inner = indent + 1;
        let mut code = String::from("{\n");
        for property in ordered {
            let at = format!("{at}.{}", property.name);
            let rendered = self.shape(&property.shape, &at, inner)?;
            let value = if property.required {
                rendered.code
            } else {
                format!("{}.optional()", rendered.code)
            };
            push_doc(&mut code, property.description.as_deref(), inner);
            let key = key(&property.name);
            if rendered.forward {
                writeln!(code, "{}get {key}() {{ return {value} }},", pad(inner)).expect("writing to a string");
            } else {
                writeln!(code, "{}{key}: {value},", pad(inner)).expect("writing to a string");
            }
        }
        code.push_str(&pad(indent));
        code.push('}');
        Ok(code)
    }

    fn union(&mut self, tag: &str, variants: &[Variant], at: &str, indent: usize) -> Result<String, Unsupported> {
        let inner = indent + 1;
        let mut code = format!("z.discriminatedUnion({}, [\n", quote(tag));
        for (index, variant) in variants.iter().enumerate() {
            let at = format!("{at}[{index}]");
            push_doc(&mut code, variant.description.as_deref(), inner);
            let rendered = match &variant.extends {
                Some(base) => {
                    let base = self.reference(base);
                    if base.forward {
                        return Err(Unsupported {
                            at,
                            problem: "a variant that extends a type declared after it".into(),
                        });
                    }
                    format!("{}.extend({})", base.code, self.fields(&variant.object, Some(tag), &at, inner)?)
                }
                None => self.object(&variant.object, Some(tag), &at, inner)?,
            };
            writeln!(code, "{}{rendered},", pad(inner)).expect("writing to a string");
        }
        code.push_str(&pad(indent));
        code.push_str("])");
        Ok(code)
    }
}

fn string_schema(string: &StringShape) -> String {
    let mut code = String::from(match string.format {
        StringFormat::Text => "z.string()",
        // The bounds that follow check what the trim leaves, as Rust's do.
        StringFormat::Trimmed => "z.string().trim()",
        // Core's `Timestamp` writes UTC with three fractional digits, the
        // contract's one spelling of a time.
        StringFormat::DateTime => "z.iso.datetime({ precision: 3 })",
        StringFormat::Email => "z.email()",
        StringFormat::HttpUrl => "z.url({ protocol: z.regexes.httpProtocol })",
        StringFormat::Base64 => "z.base64()",
    });
    if let Some(min) = string.min_length {
        write!(code, ".min({min})").expect("writing to a string");
    }
    if let Some(max) = string.max_length {
        write!(code, ".max({max})").expect("writing to a string");
    }
    if let Some(pattern) = &string.pattern {
        // `u`: the browser matches code points, as Rust's `regex` does.
        write!(code, ".regex(new RegExp({}, \"u\"))", quote(pattern)).expect("writing to a string");
    }
    code
}

/// `z.int()` holds JavaScript's safe integers; only tighter bounds are
/// written.
fn integer_schema(min: i64, max: i64) -> String {
    let mut code = String::from("z.int()");
    if min > -MAX_SAFE_INTEGER {
        write!(code, ".min({min})").expect("writing to a string");
    }
    if max < MAX_SAFE_INTEGER {
        write!(code, ".max({max})").expect("writing to a string");
    }
    code
}

fn number_schema(min: Option<Bound>, max: Option<Bound>) -> String {
    let mut code = String::from("z.number()");
    if let Some(min) = min {
        let method = if min.exclusive { "gt" } else { "min" };
        write!(code, ".{method}({})", number(min.value)).expect("writing to a string");
    }
    if let Some(max) = max {
        let method = if max.exclusive { "lt" } else { "max" };
        write!(code, ".{method}({})", number(max.value)).expect("writing to a string");
    }
    code
}

/// A number as a JavaScript literal.
fn number(value: f64) -> String {
    serde_json::Number::from_f64(value).map_or_else(|| value.to_string(), |number| number.to_string())
}

/// A string as a JavaScript literal: JSON's quoting is JavaScript's.
pub fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}

/// A property name as an object literal's key.
fn key(name: &str) -> String {
    let mut characters = name.chars();
    let identifier = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '$');
    if identifier { name.to_owned() } else { quote(name) }
}

fn pad(indent: usize) -> String {
    "  ".repeat(indent)
}

/// Writes `description` as a JSDoc comment at `indent`.
pub fn push_doc(code: &mut String, description: Option<&str>, indent: usize) {
    let Some(description) = description else {
        return;
    };
    let text = description.replace("*/", "*\\/");
    let pad = pad(indent);
    let lines: Vec<&str> = text.lines().collect();
    if let [line] = lines.as_slice() {
        writeln!(code, "{pad}/** {line} */").expect("writing to a string");
        return;
    }
    writeln!(code, "{pad}/**").expect("writing to a string");
    for line in lines {
        if line.is_empty() {
            writeln!(code, "{pad} *").expect("writing to a string");
        } else {
            writeln!(code, "{pad} * {line}").expect("writing to a string");
        }
    }
    writeln!(code, "{pad} */").expect("writing to a string");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::shape::{Property, read};
    use serde_json::json;

    fn tree() -> Shape {
        read(
            &json!({ "type": "object", "properties": {
                "name": { "type": "string" },
                "children": { "type": "array", "items": { "$ref": "#/$defs/Tree" } },
            }, "required": ["name", "children"] }),
            "Tree",
        )
        .unwrap()
    }

    #[test]
    fn a_recursive_type_refers_to_itself_through_a_getter() {
        let shape = tree();
        let definitions = [Definition {
            name: "Tree",
            shape: &shape,
            description: None,
            received: true,
        }];
        let (source, imported) = declarations(&definitions).unwrap();
        assert!(source.contains("get children() { return z.array(treeSchema) },"), "{source}");
        assert!(source.contains("name: z.string(),"), "{source}");
        assert!(imported.is_empty());
    }

    #[test]
    fn an_object_only_the_browser_sends_must_refuse_unknown_fields() {
        let open = Shape::Object(Object {
            properties: vec![Property {
                name: "id".into(),
                shape: Shape::Boolean,
                required: true,
                description: None,
            }],
            denies_unknown: false,
        });
        let definitions = [Definition {
            name: "Request",
            shape: &open,
            description: None,
            received: false,
        }];
        let error = declarations(&definitions).unwrap_err();
        assert_eq!(error.at, "Request");
    }
}
