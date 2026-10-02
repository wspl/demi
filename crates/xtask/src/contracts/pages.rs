//! Each page package's generated module (`plugin-pages.md` § Types): its
//! plugin's id, the types of every schema its plugin's manifest declares for
//! the page, and the constants of the plugin's streams. The plugins are the
//! backend's, in their order of registration; xtask keeps no list of its
//! own.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use demi_plugin_interface::{Manifest, Page};
use schemars::SchemaGenerator;
use serde_json::{Map, Value};

use super::roots::{self, Direction};
use super::zod::{push_doc, quote};
use super::{Directory, Error, module, read_definitions, received, register, source};

/// The workspace's packages share one scope; a package of it lives in
/// `packages/<name>`.
pub const SCOPE: &str = "@demicodes/";

/// Each plugin the backend registers whose manifest names a page package,
/// with its page, in registration order.
pub fn pages() -> Vec<(Manifest, Page)> {
    demi_backend::plugins::builtin()
        .iter()
        .filter_map(|factory| {
            let manifest = factory.manifest();
            let page = manifest.page.clone()?;
            Some((manifest.clone(), page))
        })
        .collect()
}

/// The directory a page package lives in, relative to the repository.
pub fn package_directory(package: &str) -> Result<String, Error> {
    let name = package.strip_prefix(SCOPE).ok_or_else(|| {
        Error::Registry(format!(
            "the page package {package} is not a workspace package of {SCOPE}"
        ))
    })?;
    Ok(format!("packages/{name}"))
}

/// Each page package's generated directory and its one module, `plugin.ts`.
pub fn modules() -> Result<Vec<Directory>, Error> {
    pages()
        .iter()
        .map(|(manifest, page)| {
            let directory = format!("{}/src/generated", package_directory(&page.package)?);
            Ok((directory, vec![("plugin.ts", page_module(manifest, page)?)]))
        })
        .collect()
}

/// The module of `page`, the page of `manifest`'s plugin. Definitions
/// `@demicodes/protocol` declares are imported from it, so a package never
/// declares a protocol type a second time.
fn page_module(manifest: &Manifest, page: &Page) -> Result<String, Error> {
    let mut schemas: Vec<(&BTreeMap<String, Value>, Direction)> = Vec::new();
    for state in [&page.user, &page.conversation].into_iter().flatten() {
        schemas.push((state.schema.value(), Direction::Receives));
    }
    for method in &page.methods {
        schemas.push((method.params.value(), Direction::Sends));
        schemas.push((method.result.value(), Direction::Receives));
    }
    for stream in &manifest.streams {
        schemas.push((stream.receives.value(), Direction::Receives));
        schemas.push((stream.sends.value(), Direction::Sends));
    }
    let mut generator = SchemaGenerator::default();
    register(&mut generator, roots::protocol())?;
    let protocol_names: BTreeSet<String> = generator.definitions().keys().cloned().collect();
    let mut collected = Map::new();
    let mut named = Vec::new();
    for (schema, direction) in schemas {
        if let Some(name) = collect(schema, &mut collected)? {
            named.push((name, direction));
        }
    }
    let definitions = read_definitions(collected)?;
    let received = received(
        &definitions,
        named
            .iter()
            .filter(|(_, direction)| *direction == Direction::Receives),
    );
    let names: BTreeSet<String> = definitions
        .keys()
        .filter(|name| !protocol_names.contains(*name))
        .cloned()
        .collect();
    let (declarations, imports) = module(&definitions, &received, &names)?;
    let mut text = source(&declarations, &imports);
    text.push('\n');
    push_doc(
        &mut text,
        Some("The plugin this package is the page of."),
        0,
    );
    writeln!(
        text,
        "export const PLUGIN = {}",
        quote(manifest.id.as_str())
    )
    .expect("writing to a string");
    for constant in manifest.streams.iter().flat_map(|stream| &stream.constants) {
        push_doc(&mut text, Some(&constant.description), 0);
        let value = serde_json::to_string(&constant.value).expect("a JSON value serializes");
        writeln!(text, "export const {} = {value}", constant.name).expect("writing to a string");
    }
    Ok(text)
}

/// Adds the definitions of one root schema, as schemars wrote it, to
/// `definitions`: those under its `$defs`, and the root itself under its
/// title. Answers the root's name; none for the unit type, a method's
/// empty result, which needs no type.
fn collect(
    schema: &BTreeMap<String, Value>,
    definitions: &mut Map<String, Value>,
) -> Result<Option<String>, Error> {
    let mut root: Map<String, Value> = schema
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if let Some(defs) = root.remove("$defs") {
        let Value::Object(defs) = defs else {
            return Err(Error::Definitions(format!(
                "a schema's $defs is not an object: {defs}"
            )));
        };
        for (name, definition) in defs {
            insert(definitions, name, definition)?;
        }
    }
    if root.get("type").and_then(Value::as_str) == Some("null") {
        return Ok(None);
    }
    let Some(Value::String(name)) = root.remove("title") else {
        return Err(Error::Definitions(format!(
            "a manifest schema is not a named type: {}",
            Value::Object(root)
        )));
    };
    insert(definitions, name.clone(), Value::Object(root))?;
    Ok(Some(name))
}

/// Adds `definition` as `name`; two schemas of one name must agree.
fn insert(
    definitions: &mut Map<String, Value>,
    name: String,
    definition: Value,
) -> Result<(), Error> {
    match definitions.get(&name) {
        Some(existing) if *existing != definition => Err(Error::Definitions(format!(
            "two schemas of {name} differ: {existing} and {definition}"
        ))),
        Some(_) => Ok(()),
        None => {
            definitions.insert(name, definition);
            Ok(())
        }
    }
}
