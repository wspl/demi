//! Regenerate the vendored CDP bindings and validation catalog from their pinned PDL.
//!
//! The validation catalog (`pdl/protocol.json`) holds every domain, since a
//! raw CDP command may name any. The bindings (`src/cdp.rs`) hold only the
//! domains that Demi and chromiumoxide use and the domains their types refer
//! to: the rest are about a third of the generated code and nothing reads
//! them, and an event of a domain left out decodes as `CdpEvent::Other`.
//!
//! Both hold the numbers that Chrome writes as null ([`NULL_WHEN_INFINITE`]).
use chromiumoxide_pdl::{
    build::Generator,
    pdl::{parser::parse_pdl, resolver::resolve_pdl},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// The domains whose types Demi (`crates/`) and chromiumoxide name.
const USED: &[&str] = &[
    "Accessibility",
    "Animation",
    "Browser",
    "CSS",
    "DOM",
    "Debugger",
    "Emulation",
    "Fetch",
    "Input",
    "Log",
    "Network",
    "Page",
    "Performance",
    "Runtime",
    "Security",
    "Storage",
    "Target",
];

/// Required array parameters that a command sends empty, which the
/// generator would leave out when empty: each parameter's struct and its
/// Rust field.
const SENT_EMPTY: &[(&str, &str)] = &[
    // TouchEnd and TouchCancel send no touch point, and Chrome refuses the
    // command without the parameter.
    ("DispatchTouchEventParams", "touch_points"),
];

/// Number properties whose description says Chrome writes null for a value
/// JSON cannot hold (±Inf), which PDL has no way to declare: each one's
/// domain, type and name. The catalog marks each `"nullable": true` and
/// admits null there; the bindings make each optional, so null decodes as
/// `None`.
const NULL_WHEN_INFINITE: &[(&str, &str, &str)] = &[
    // A cookie whose Max-Age reaches beyond the times Chrome keeps.
    ("Network", "Cookie", "expires"),
];

/// `protocol`, the catalog, with each [`NULL_WHEN_INFINITE`] property marked
/// nullable.
fn mark_nullable(protocol: &mut Value) -> Result<(), Box<dyn std::error::Error>> {
    for (domain, shape, property) in NULL_WHEN_INFINITE {
        let declared = protocol["domains"]
            .as_array_mut()
            .ok_or("PDL domains are not an array")?
            .iter_mut()
            .find(|candidate| candidate["domain"] == *domain)
            .and_then(|found| found["types"].as_array_mut())
            .and_then(|types| types.iter_mut().find(|candidate| candidate["id"] == *shape))
            .and_then(|found| found["properties"].as_array_mut())
            .and_then(|properties| {
                properties
                    .iter_mut()
                    .find(|candidate| candidate["name"] == *property)
            })
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("the pinned protocol has no {domain}.{shape}.{property}"))?;
        if declared.get("type").and_then(Value::as_str) != Some("number") {
            return Err(format!("{domain}.{shape}.{property} is not a number").into());
        }
        declared.insert("nullable".into(), Value::Bool(true));
    }
    Ok(())
}

/// `source`, the PDL of `domain`, with each [`NULL_WHEN_INFINITE`] property
/// of the domain declared optional, for the bindings.
fn optional_nullable(domain: &str, source: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
    for (_, shape, property) in NULL_WHEN_INFINITE
        .iter()
        .filter(|(named, _, _)| *named == domain)
    {
        let declared = format!("type {shape} extends object");
        let start = lines
            .iter()
            .position(|line| line.trim_start().ends_with(&declared))
            .ok_or_else(|| format!("{domain}.pdl declares no {shape}"))?;
        // The type's members are indented deeper than the declarations
        // around it.
        let members = lines[start + 1..]
            .iter()
            .take_while(|line| line.is_empty() || line.starts_with("    "))
            .count();
        let line = lines[start + 1..start + 1 + members]
            .iter_mut()
            .find(|line| line.split_whitespace().rev().take(2).eq([*property, "number"]))
            .ok_or_else(|| format!("{domain}.{shape} has no number {property}"))?;
        let at = line
            .find("number")
            .expect("the line names its type");
        line.insert_str(at, "optional ");
    }
    Ok(lines.join("\n") + "\n")
}

/// `bindings` with each [`SENT_EMPTY`] parameter serialized even when empty.
fn send_empty(bindings: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut bindings = bindings.to_owned();
    for (structure, field) in SENT_EMPTY {
        let start = bindings
            .find(&format!("pub struct {structure} {{"))
            .ok_or_else(|| format!("the bindings have no {structure}"))?;
        let end = start
            + bindings[start..]
                .find(&format!("pub {field}:"))
                .ok_or_else(|| format!("{structure} has no {field}"))?;
        let attribute = "#[serde(skip_serializing_if = \"Vec::is_empty\")]";
        let at = start
            + bindings[start..end]
                .rfind(attribute)
                .ok_or_else(|| format!("{structure}'s {field} is not skipped when empty"))?;
        bindings.replace_range(
            at..at + attribute.len(),
            "// Demi: a required parameter, sent empty too.",
        );
    }
    Ok(bindings)
}

// PDL's serializer includes generator metadata; CDP's JSON schema does not.
fn protocol_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("raw_name");
            object.remove("is_circular_dep");
            for value in object.values_mut() {
                protocol_value(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                protocol_value(value);
            }
        }
        _ => {}
    }
}

/// The domains a `$ref` in `value` names outside its own domain.
fn referenced(value: &Value, domains: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                match (key.as_str(), value) {
                    ("$ref", Value::String(name)) => {
                        if let Some((domain, _)) = name.split_once('.') {
                            domains.insert(domain.to_owned());
                        }
                    }
                    _ => referenced(value, domains),
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                referenced(value, domains);
            }
        }
        _ => {}
    }
}

/// [`USED`] and every domain their types refer to, directly or through
/// other domains.
fn kept(domains: &[Value]) -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
    let mut kept = BTreeSet::new();
    let mut pending: Vec<String> = USED.iter().map(|&name| name.to_owned()).collect();
    while let Some(name) = pending.pop() {
        if !kept.insert(name.clone()) {
            continue;
        }
        let domain = domains
            .iter()
            .find(|domain| domain["domain"] == name.as_str())
            .ok_or_else(|| format!("the pinned protocol has no domain {name}"))?;
        let mut named = BTreeSet::new();
        referenced(domain, &mut named);
        pending.extend(named.into_iter().filter(|name| !kept.contains(name)));
    }
    Ok(kept)
}

/// Whether a PDL line opens the definition of a domain, and which.
fn domain_line(line: &str) -> Option<&str> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let at = words.iter().position(|&word| word == "domain")?;
    let modifiers = &words[..at];
    modifiers
        .iter()
        .all(|&word| word == "experimental" || word == "deprecated")
        .then(|| words.get(at + 1).copied())
        .flatten()
}

/// `source`, a PDL file, with only the domains in `kept`: an `include` of a
/// domain file stays when its domain does, and an inline domain's lines
/// stay with it. The lines before the first domain, the version among them,
/// stay.
fn filtered(source: &str, kept: &BTreeSet<String>) -> String {
    let mut output = String::new();
    let mut keeping = true;
    for line in source.lines() {
        if let Some(file) = line.strip_prefix("include domains/") {
            let domain = file.trim_end_matches(".pdl");
            if kept.contains(domain) {
                output.push_str(line);
                output.push('\n');
            }
            continue;
        }
        if let Some(domain) = domain_line(line) {
            keeping = kept.contains(domain);
        }
        if keeping {
            output.push_str(line);
            output.push('\n');
        }
    }
    output
}

/// Writes the PDL of the kept domains into `directory`, with the domain
/// files the browser protocol includes, and returns the two top files.
fn write_kept(
    root: &Path,
    paths: &[PathBuf; 2],
    kept: &BTreeSet<String>,
    directory: &Path,
) -> Result<[PathBuf; 2], Box<dyn std::error::Error>> {
    fs::create_dir_all(directory.join("domains"))?;
    for domain in kept {
        let file = root.join("pdl/domains").join(format!("{domain}.pdl"));
        if file.exists() {
            fs::write(
                directory.join("domains").join(format!("{domain}.pdl")),
                optional_nullable(domain, &fs::read_to_string(&file)?)?,
            )?;
        }
    }
    let mut written = paths.clone();
    for (path, output) in paths.iter().zip(written.iter_mut()) {
        let name = path.file_name().ok_or("a PDL path names a file")?;
        *output = directory.join(name);
        fs::write(&*output, filtered(&fs::read_to_string(path)?, kept))?;
    }
    Ok(written)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("expected chromiumoxide_cdp directory")?,
    );
    let paths = [
        root.join("pdl/js_protocol.pdl"),
        root.join("pdl/browser_protocol.pdl"),
    ];
    let sources = paths
        .iter()
        .map(|path| {
            let input = fs::read_to_string(path)?;
            resolve_pdl(path, &input).map_err(|error| std::io::Error::other(error.message))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let protocols = sources
        .iter()
        .map(|source| parse_pdl(source).map_err(|error| std::io::Error::other(error.message)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut value = serde_json::to_value(&protocols[0])?;
    value["domains"]
        .as_array_mut()
        .ok_or("PDL domains are not an array")?
        .extend(
            serde_json::to_value(&protocols[1])?["domains"]
                .as_array()
                .ok_or("PDL domains are not an array")?
                .iter()
                .cloned(),
        );
    for domain in value["domains"]
        .as_array_mut()
        .ok_or("PDL domains are not an array")?
    {
        if let Some(types) = domain["types"].as_array_mut() {
            for definition in types {
                let object = definition
                    .as_object_mut()
                    .ok_or("PDL type is not an object")?;
                let name = object.remove("name").ok_or("PDL type has no name")?;
                object.insert("id".into(), name);
            }
        }
    }
    protocol_value(&mut value);
    mark_nullable(&mut value)?;
    fs::write(root.join("pdl/protocol.json"), serde_json::to_vec(&value)?)?;
    let kept = kept(value["domains"].as_array().ok_or("PDL domains are not an array")?)?;
    let directory = std::env::temp_dir().join(format!("demi-cdp-{}", std::process::id()));
    let generated = write_kept(&root, &paths, &kept, &directory).and_then(|kept_paths| {
        Generator::default()
            .out_dir(root.join("src"))
            .allowed_deprecated_type("emulateNetworkConditions")
            .allowed_deprecated_type("grantPermissions")
            .compile_pdls(&kept_paths)
            .map_err(Into::into)
    });
    // The kept PDL is scratch: removed whether or not the bindings compiled.
    let removed = fs::remove_dir_all(&directory);
    generated?;
    removed?;
    let bindings = root.join("src/cdp.rs");
    fs::write(&bindings, send_empty(&fs::read_to_string(&bindings)?)?)?;
    Ok(())
}
