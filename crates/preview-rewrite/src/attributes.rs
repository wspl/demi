//! Resource attributes in markup and the DOM, refresh directives and import maps.

use serde_json::{Map, Value};

use crate::address::{Context, Role, convert_written_url, map_module_specifier, map_url};
use crate::css::rewrite_css;

/// Policies removed from previewed documents to keep pages working (`docs/browser/preview.md` § Response headers).
pub const REMOVED_CONTENT_POLICIES: [&str; 6] = [
    "x-frame-options",
    "content-security-policy",
    "content-security-policy-report-only",
    "x-content-security-policy",
    "x-content-security-policy-report-only",
    "x-webkit-csp",
];

/// Attributes whose value is one address.
pub const URL_ATTRIBUTES: [&str; 9] = ["src", "href", "action", "formaction", "poster", "data", "manifest", "cite", "background"];

fn html_space(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\x0c' | '\r' | ' ')
}

/// Rewrite only the candidate URLs of a `srcset`, following the URL and descriptor scanning
/// states of HTML's srcset parsing algorithm, so descriptors stay byte for byte.
pub fn rewrite_srcset(value: &str, base: &str, reflect: bool, context: &Context) -> String {
    let characters: Vec<(usize, char)> = value.char_indices().collect();
    let mut position = 0;
    let mut copied = 0;
    let mut output = String::with_capacity(value.len());
    let at = |index: usize| characters.get(index).map_or(value.len(), |&(offset, _)| offset);
    while position < characters.len() {
        while position < characters.len() && (html_space(characters[position].1) || characters[position].1 == ',') {
            position += 1;
        }
        let start = position;
        while position < characters.len() && !html_space(characters[position].1) {
            position += 1;
        }
        let mut end = position;
        while end > start && characters[end - 1].1 == ',' {
            end -= 1;
        }
        if end > start {
            output.push_str(&value[at(copied)..at(start)]);
            output.push_str(&convert_written_url(&value[at(start)..at(end)], base, reflect, Role::Resource, context));
            copied = end;
        }
        if end != position {
            continue;
        }
        let mut parentheses = false;
        while position < characters.len() {
            let character = characters[position].1;
            position += 1;
            if parentheses {
                if character == ')' {
                    parentheses = false;
                }
            } else if character == ',' {
                break;
            } else if character == '(' {
                parentheses = true;
            }
        }
    }
    output.push_str(&value[at(copied)..]);
    output
}

/// One resource attribute of an element, forward to the proxy or with `reflect` back to
/// the page's text. `role` distinguishes frames and modules (see [`Role`]).
pub fn rewrite_resource_attribute(name: &str, value: &str, base: &str, reflect: bool, role: Role, context: &Context) -> String {
    let attribute = name.to_ascii_lowercase();
    match attribute.as_str() {
        "http-equiv" if !reflect && REMOVED_CONTENT_POLICIES.contains(&value.to_ascii_lowercase().as_str()) => String::new(),
        "style" => rewrite_css(value, base, reflect, context),
        "srcset" | "imagesrcset" => rewrite_srcset(value, base, reflect, context),
        "ping" => value
            .split(html_space)
            .map(|part| if part.is_empty() { String::new() } else { convert_written_url(part, base, reflect, role, context) })
            .collect::<Vec<_>>()
            .join(" "),
        attribute if URL_ATTRIBUTES.contains(&attribute) => convert_written_url(value, base, reflect, role, context),
        _ => value.to_owned(),
    }
}

/// A refresh directive (`5; url=/next`) with its address mapped as a navigation.
pub fn rewrite_refresh(value: &str, base: &str, context: &Context) -> String {
    let trimmed = value.trim_start();
    let digits = trimmed.find(|character: char| !character.is_ascii_digit() && character != '.').unwrap_or(trimmed.len());
    if digits == 0 {
        return value.to_owned();
    }
    let rest = trimmed[digits..].trim_start();
    let Some(after_separator) = rest.strip_prefix(';').or_else(|| rest.strip_prefix(',')) else { return value.to_owned() };
    let address_start = after_separator.trim_start();
    let address = if address_start.len() >= 3 && address_start[..3].eq_ignore_ascii_case("url") {
        address_start[3..].trim_start().strip_prefix('=').map_or(address_start, str::trim_start)
    } else {
        address_start
    };
    let unquoted = address.trim().trim_matches(|character| character == '\'' || character == '"');
    let Some(mapped) = map_url(unquoted, Some(base), Role::Navigation, context) else { return value.to_owned() };
    format!("{}{mapped}", &value[..value.len() - address.len()])
}

/// The address-bearing parts of an import map through the proxy. An invalid map stays as
/// written, so the browser reports it.
pub fn rewrite_import_map(source: &str, base: &str, context: &Context) -> String {
    let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(source) else { return source.to_owned() };
    let specifiers = |entries: &Value| -> Value {
        let Value::Object(entries) = entries else { return entries.clone() };
        Value::Object(
            entries
                .iter()
                .map(|(key, value)| {
                    let value = match value {
                        Value::String(text) => Value::String(map_module_specifier(text, Some(base), context)),
                        other => other.clone(),
                    };
                    (map_module_specifier(key, Some(base), context), value)
                })
                .collect(),
        )
    };
    let addresses = |key: &str| map_module_specifier(key, Some(base), context);
    if let Some(imports) = map.get("imports") {
        let mapped = specifiers(imports);
        map.insert("imports".into(), mapped);
    }
    if let Some(Value::Object(scopes)) = map.get("scopes") {
        let mapped: Map<String, Value> = scopes.iter().map(|(key, value)| (addresses(key), specifiers(value))).collect();
        map.insert("scopes".into(), Value::Object(mapped));
    }
    if let Some(Value::Object(integrity)) = map.get("integrity") {
        let mapped: Map<String, Value> = integrity.iter().map(|(key, value)| (addresses(key), value.clone())).collect();
        map.insert("integrity".into(), Value::Object(mapped));
    }
    serde_json::to_string(&Value::Object(map)).unwrap_or_else(|_| source.to_owned()).replace('<', "\\u003c")
}
