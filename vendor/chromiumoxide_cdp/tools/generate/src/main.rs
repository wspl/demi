//! Regenerate the vendored CDP bindings and validation catalog from their pinned PDL.
use chromiumoxide_pdl::{
    build::Generator,
    pdl::{parser::parse_pdl, resolver::resolve_pdl},
};
use serde_json::Value;
use std::{fs, path::PathBuf};

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
    fs::write(root.join("pdl/protocol.json"), serde_json::to_vec(&value)?)?;
    Generator::default()
        .out_dir(root.join("src"))
        .allowed_deprecated_type("emulateNetworkConditions")
        .allowed_deprecated_type("grantPermissions")
        .compile_pdls(&paths)?;
    Ok(())
}
