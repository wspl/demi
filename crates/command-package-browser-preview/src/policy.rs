//! Response policies the engine adapts: HSTS on a redirect,
//! Document-Isolation-Policy for COEP documents, and CORS and CORP decided on
//! logical origins (`preview.md` § CORS, CORP and Referer).

use demi_preview_rewrite::address::site_of;
use url::Url;

use crate::engine::Fields;

/// The header's directives: split at semicolons outside quoted strings (RFC 6797 § 6.1).
fn directives(header: &str) -> Vec<&str> {
    let mut directives = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in header.char_indices() {
        if escaped {
            escaped = false;
        } else if quoted && character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character == ';' && !quoted {
            directives.push(&header[start..index]);
            start = index + 1;
        }
    }
    directives.push(&header[start..]);
    directives
}

/// Apply the current HTTPS response's HSTS policy to its own redirect, as a browser does
/// before following it (RFC 6797 § 6.1, 8.3). Only this response's policy: no state is kept.
pub(crate) fn apply_redirect_hsts(location: &str, response_url: &Url, header: Option<&str>) -> String {
    let Some(header) = header else { return location.to_owned() };
    let Ok(mut destination) = response_url.join(location) else { return location.to_owned() };
    let host = response_url.host_str().unwrap_or_default();
    let ip = host.starts_with('[') || host.parse::<std::net::IpAddr>().is_ok();
    if response_url.scheme() != "https" || destination.scheme() != "http" || ip {
        return location.to_owned();
    }
    if header.bytes().any(|byte| byte != b'\t' && !(0x20..=0x7e).contains(&byte) && byte < 0x80) {
        return location.to_owned();
    }
    let mut fields: Vec<(String, Option<String>)> = Vec::new();
    for directive in directives(header) {
        let directive = directive.trim_matches(|character| character == ' ' || character == '\t');
        if directive.is_empty() {
            continue;
        }
        let (name, value) = match directive.split_once('=') {
            Some((name, value)) => (name.trim(), Some(value.trim())),
            None => (directive, None),
        };
        let token = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&byte));
        if !token(name) {
            return location.to_owned();
        }
        let value = match value {
            Some(value) if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') => Some(value[1..value.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")),
            Some(value) if token(value) => Some(value.to_owned()),
            Some(_) => return location.to_owned(),
            None => None,
        };
        let name = name.to_ascii_lowercase();
        if fields.iter().any(|(existing, _)| *existing == name) {
            return location.to_owned();
        }
        fields.push((name, value));
    }
    let field = |name: &str| fields.iter().find(|(existing, _)| existing == name).map(|(_, value)| value.clone());
    let Some(Some(maximum_age)) = field("max-age") else { return location.to_owned() };
    if maximum_age.is_empty() || !maximum_age.bytes().all(|byte| byte.is_ascii_digit()) || maximum_age.bytes().all(|byte| byte == b'0') {
        return location.to_owned();
    }
    let subdomains = field("includesubdomains");
    if matches!(subdomains, Some(Some(_))) {
        return location.to_owned();
    }
    let target = destination.host_str().unwrap_or_default();
    let applies = target == host || subdomains.is_some() && target.ends_with(&format!(".{host}"));
    if !applies {
        return location.to_owned();
    }
    if destination.set_scheme("https").is_err() {
        return location.to_owned();
    }
    // An explicit :80 belongs to http; the upgraded URL uses the default port.
    if destination.port() == Some(80) {
        let _ = destination.set_port(None);
    }
    destination.into()
}

/// Document-Isolation-Policy for a COEP document, so it is isolated inside the Demi page's
/// ordinary iframe; a document that declares its own policy keeps it.
pub(crate) fn document_isolation_for(coep: Option<&str>, existing: Option<&str>) -> Option<&'static str> {
    if existing.is_some() {
        return None;
    }
    // A structured-field token, optionally with parameters; anything else is ignored, as
    // browsers ignore invalid policies.
    match coep?.split(';').next()?.trim() {
        "require-corp" => Some("isolate-and-require-corp"),
        "credentialless" => Some("isolate-and-credentialless"),
        _ => None,
    }
}

/// CORP for a no-cors load, decided on the logical origins.
pub(crate) fn corp_allows(policy: Option<&str>, fields: &Fields, target_origin: &str) -> bool {
    let Some(policy) = policy else { return true };
    if fields.origin.is_empty() {
        return true;
    }
    match policy.trim().to_ascii_lowercase().as_str() {
        "same-origin" => fields.origin == target_origin,
        "same-site" => site_of(&fields.origin) == site_of(target_origin),
        _ => true,
    }
}

/// The upstream's CORS answer for a logical origin.
pub(crate) fn cors_allows(allow_origin: Option<&str>, allow_credentials: Option<&str>, origin: &str, credentials: bool) -> bool {
    let allowed = allow_origin.map(str::trim);
    if credentials {
        return allowed == Some(origin) && allow_credentials.map(str::trim) == Some("true");
    }
    allowed == Some("*") || allowed == Some(origin)
}
