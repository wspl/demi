//! Browser output is schema validated and bounded before any success bytes escape.

use serde_json::Value;
use std::io::Write;
use tokio_util::sync::CancellationToken;

use super::{
    BrowserError, Result,
    protocol::{INLINE_BYTES, validate_result},
};

/// Publish a browser-generated asset as a new Host file after a complete write.
pub(super) async fn save(
    cwd: &str,
    output: &str,
    bytes: Vec<u8>,
    cancel: &CancellationToken,
) -> Result<String> {
    let path = crate::files::resolve_path(cwd, output).map_err(BrowserError::Configuration)?;
    let cancelled = cancel.clone();
    tokio::task::spawn_blocking(move || {
        let parent = path.parent().ok_or_else(|| {
            BrowserError::Configuration("browser output has no parent directory".into())
        })?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        for chunk in bytes.chunks(64 * 1024) {
            if cancelled.is_cancelled() {
                return Err(BrowserError::Cancelled);
            }
            temporary.write_all(chunk)?;
        }
        temporary.flush()?;
        if cancelled.is_cancelled() {
            return Err(BrowserError::Cancelled);
        }
        temporary
            .persist_noclobber(&path)
            .map_err(|error| BrowserError::Io(error.error))?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await?
}

pub(super) fn render(operation: &str, mut value: Value, json: bool) -> Result<Vec<u8>> {
    loop {
        let bytes = serde_json::to_vec(&value)
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
        if bytes.len() <= INLINE_BYTES {
            break;
        }
        if let Some(content) = value.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                let end = content.floor_char_boundary(content.len() / 2);
                value["content"] = Value::String(content[..end].into());
                value["truncated"] = Value::Bool(true);
                continue;
            }
        }
        let collection = ["tree", "matches", "entries", "tabs", "values"]
            .into_iter()
            .find(|key| value.get(*key).is_some_and(Value::is_array));
        if let Some(key) = collection {
            let items = value[key].as_array_mut().expect("collection was checked");
            if items.pop().is_some() {
                value["truncated"] = Value::Bool(true);
                continue;
            }
        }
        return Err(BrowserError::InvalidResult(
            "result_too_large: save or narrow browser output".into(),
        ));
    }
    let value = validate_result(operation, value)
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    let mut result = if json {
        serde_json::to_string(&value)
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
    } else if let Some(nodes) = value
        .get("tree")
        .or_else(|| value.get("matches"))
        .and_then(Value::as_array)
    {
        let mut text = String::new();
        for node in nodes {
            let depth = node["depth"].as_u64().unwrap_or(0).min(64) as usize;
            let role = node["role"].as_str().unwrap_or("");
            let name = node["name"].as_str().unwrap_or("");
            let reference = node["ref"].as_str().unwrap_or("");
            text.push_str(&format!(
                "{}{role:?} {name:?} [ref={reference}]\n",
                "  ".repeat(depth)
            ));
        }
        if nodes.is_empty() {
            text.push_str("No matching nodes.\n");
        }
        if value["truncated"] == true {
            text.push_str("[truncated]\n");
        }
        text
    } else {
        serde_json::to_string_pretty(&value)
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
    };
    if !result.ends_with('\n') {
        result.push('\n');
    }
    if result.len() > INLINE_BYTES {
        return Err(BrowserError::InvalidResult(
            "result_too_large: narrow browser output".into(),
        ));
    }
    Ok(result.into_bytes())
}
