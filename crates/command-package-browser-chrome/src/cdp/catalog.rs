//! The pinned CDP protocol as JSON schemas, which every `cdp` parameter,
//! result and event is validated against.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::driver::operation::{BrowserError, Result};

pub(crate) struct Catalog {
    definitions: Value,
    schemas: HashMap<String, Value>,
    /// The validators compiled so far. A std mutex: every debugging pump and
    /// command in the process shares this cache, and each holds the mutex
    /// only to look up or to add a validator, never while compiling one or
    /// across an await.
    compiled: std::sync::Mutex<HashMap<String, Arc<jsonschema::Validator>>>,
}

pub(crate) fn catalog() -> Result<&'static Catalog> {
    static CATALOG: OnceLock<std::result::Result<Catalog, String>> = OnceLock::new();
    CATALOG
        .get_or_init(Catalog::load)
        .as_ref()
        .map_err(|error| BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.clone())))
}

impl Catalog {
    fn load() -> std::result::Result<Self, String> {
        let protocol: PinnedProtocol = serde_json::from_str(include_str!(
            "../../../../vendor/chromiumoxide/chromiumoxide_cdp/pdl/protocol.json"
        ))
        .map_err(|error| error.to_string())?;
        let mut definitions = serde_json::Map::new();
        let mut schemas = HashMap::new();
        for domain in protocol.domains {
            for shape in domain.types.into_iter().flatten() {
                definitions.insert(
                    format!(
                        "{}.{}",
                        domain.domain,
                        shape.id.as_deref().ok_or("CDP type has no id")?
                    ),
                    shape.schema(&domain.domain)?,
                );
            }
            for command in domain.commands {
                schemas.insert(
                    format!("{}.{}:params", domain.domain, command.name),
                    object_schema(&command.parameters, &domain.domain)?,
                );
                schemas.insert(
                    format!("{}.{}:returns", domain.domain, command.name),
                    object_schema(&command.returns, &domain.domain)?,
                );
            }
            for event in domain.events {
                schemas.insert(
                    format!("{}.{}:event", domain.domain, event.name),
                    object_schema(&event.parameters, &domain.domain)?,
                );
            }
        }
        Ok(Self {
            definitions: Value::Object(definitions),
            schemas,
            compiled: std::sync::Mutex::new(HashMap::new()),
        })
    }
    fn cache(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<jsonschema::Validator>>> {
        // Nothing that can panic runs while the mutex is held.
        self.compiled
            .lock()
            .expect("the CDP schema cache is intact")
    }

    pub(crate) fn schema(&self, method: &str, kind: &str) -> Result<&Value> {
        self.schemas
            .get(&format!("{method}:{kind}"))
            .ok_or_else(|| {
                BrowserError::Configuration(format!("unknown pinned CDP {kind} method: {method}"))
            })
    }
    pub(crate) fn validate(&self, method: &str, kind: &str, value: &Value) -> Result<()> {
        let key = format!("{method}:{kind}");
        let cached = self.cache().get(&key).cloned();
        let validator = match cached {
            Some(validator) => validator,
            None => {
                let mut schema = self
                    .schema(method, kind)
                    .map_err(|error| {
                        BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string()))
                    })?
                    .clone();
                schema["$defs"] = self.definitions.clone();
                let validator = Arc::new(jsonschema::options().offline().build(&schema).map_err(
                    |error| {
                        BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string()))
                    },
                )?);
                // Another caller may have compiled the same schema meanwhile;
                // either copy validates alike.
                self.cache().entry(key).or_insert(validator).clone()
            }
        };
        validator.validate(value).map_err(|error| {
            BrowserError::Cdp(chromiumoxide::error::CdpError::msg(format!(
                "invalid {method} {kind}: {error}"
            )))
        })
    }
}

#[derive(Deserialize)]
struct PinnedProtocol {
    domains: Vec<Domain>,
}
#[derive(Deserialize)]
struct Domain {
    domain: String,
    #[serde(default)]
    types: Option<Vec<Shape>>,
    #[serde(default)]
    commands: Vec<Declaration>,
    #[serde(default)]
    events: Vec<Declaration>,
}
#[derive(Deserialize)]
struct Declaration {
    name: String,
    #[serde(default)]
    parameters: Vec<Shape>,
    #[serde(default)]
    returns: Vec<Shape>,
}
#[derive(Deserialize)]
struct Shape {
    id: Option<String>,
    name: Option<String>,
    #[serde(default)]
    optional: bool,
    #[serde(rename = "$ref")]
    reference: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "enum")]
    variants: Option<Vec<String>>,
    items: Option<Box<Shape>>,
    properties: Option<Vec<Shape>>,
}
impl Shape {
    fn schema(&self, domain: &str) -> std::result::Result<Value, String> {
        if let Some(reference) = &self.reference {
            return Ok(
                json!({"$ref":format!("#/$defs/{}",if reference.contains('.') {reference.clone()}else{format!("{domain}.{reference}")})}),
            );
        }
        let kind = self
            .kind
            .as_deref()
            .ok_or("CDP type has no type or reference")?;
        let mut schema = match kind {
            "any" => json!({}),
            "binary" => {
                json!({"type":"string","pattern":"^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$"})
            }
            "array" => {
                json!({"type":"array","items":self.items.as_ref().ok_or("CDP array has no items")?.schema(domain)?})
            }
            "object" if self.properties.is_some() => {
                object_schema(self.properties.as_ref().expect("properties exist"), domain)?
            }
            "object" | "string" | "integer" | "number" | "boolean" => json!({"type":kind}),
            _ => return Err(format!("unknown pinned CDP type {kind}")),
        };
        if let Some(variants) = &self.variants {
            schema["enum"] = json!(variants);
        }
        Ok(schema)
    }
}

/// Translate pinned CDP record declarations into strict JSON Schema objects.
fn object_schema(properties: &[Shape], domain: &str) -> std::result::Result<Value, String> {
    let mut members = serde_json::Map::new();
    let mut required = Vec::new();
    for property in properties {
        let name = property.name.as_ref().ok_or("CDP parameter has no name")?;
        members.insert(name.clone(), property.schema(domain)?);
        if !property.optional {
            required.push(name);
        }
    }
    Ok(
        json!({"type":"object","properties":members,"required":required,"additionalProperties":false}),
    )
}
