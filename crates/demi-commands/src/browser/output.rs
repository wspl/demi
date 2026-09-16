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
        temporary.persist_noclobber(&path).map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                BrowserError::OutputExists(path.display().to_string())
            } else {
                BrowserError::Io(error.error)
            }
        })?;
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
        if let Some(content) = value.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
            let end = content.floor_char_boundary(content.len() / 2);
            value["content"] = Value::String(content[..end].into());
            value["truncated"] = Value::Bool(true);
            continue;
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
        return Err(BrowserError::ResultTooLarge);
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
        let mut pending: Vec<_> = nodes.iter().rev().map(|node| (node, 0_usize)).collect();
        while let Some((node, depth)) = pending.pop() {
            if let Some(children) = node["children"].as_array() {
                pending.extend(children.iter().rev().map(|child| (child, depth + 1)));
            }
            let depth = depth.min(64);
            let role = node["role"].as_str().unwrap_or("");
            let name = node["name"].as_str().unwrap_or("");
            let reference = node["ref"].as_str().unwrap_or("");
            text.push_str(&format!("{}{role} {name:?}", "  ".repeat(depth)));
            if !reference.is_empty() {
                text.push_str(&format!(" [ref={reference}]"));
            }
            if let Some(states) = node["states"].as_array() {
                for state in states.iter().filter_map(Value::as_str) {
                    text.push_str(&format!(" [{state}]"));
                }
            }
            if let Some(value) = node.get("value") {
                text.push_str(&format!(" [value={value}]"));
            }
            text.push('\n');
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
        return Err(BrowserError::ResultTooLarge);
    }
    Ok(result.into_bytes())
}

/// Render browser failures with the same progress and details as their JSON form.
pub(super) fn render_error(code: &str, message: &str, details: &Value) -> String {
    let action = details["action"].as_str().unwrap_or("not_started");
    let mut text = format!("Error: {code}\n{message}\nAction: {action}.\n");
    if let Some(details) = details.as_object() {
        for (key, value) in details {
            if key == "action" {
                continue;
            }
            let title = match key.as_str() {
                "url" => "Current URL".into(),
                _ => {
                    let mut characters = key.chars();
                    characters.next().map_or_else(String::new, |first| {
                        first.to_uppercase().collect::<String>() + characters.as_str()
                    })
                }
            };
            let value = match value {
                Value::String(value) => value.clone(),
                _ => value.to_string(),
            };
            text.push_str(&format!("{title}: {value}\n"));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_errors_show_progress_and_individual_details() {
        let text = render_error(
            "not_actionable",
            "The button is covered.",
            &json!({
                "action": "not_started", "tab": "t_test", "url": "https://example.test/",
                "interceptor": "<div id=overlay>", "delivered": 0,
            }),
        );
        assert!(
            text.starts_with(
                "Error: not_actionable\nThe button is covered.\nAction: not_started.\n"
            )
        );
        for line in [
            "Tab: t_test\n",
            "Current URL: https://example.test/\n",
            "Interceptor: <div id=overlay>\n",
            "Delivered: 0\n",
        ] {
            assert!(text.contains(line));
        }
        assert!(!text.contains("Details:"));
    }

    #[test]
    fn inspect_text_renders_nested_states_and_values() {
        let value = json!({
            "tab": "t_test", "url": "about:blank", "title": "", "view": "accessibility",
            "tree": [{"role": "main", "children": [{"role": "checkbox", "name": "Confirm", "states": ["checked=false"], "value": "hello"}]}],
            "truncated": false,
        });
        let text = String::from_utf8(render("inspect", value.clone(), false).unwrap()).unwrap();
        assert!(text.contains("main "));
        assert!(text.contains("  checkbox \"Confirm\" [checked=false] [value=\"hello\"]"));
        let structured: Value =
            serde_json::from_slice(&render("inspect", value, true).unwrap()).unwrap();
        assert_eq!(
            structured["tree"][0]["children"][0]["states"][0],
            "checked=false"
        );
    }

    #[test]
    fn navigation_results_require_url_but_allow_missing_metadata() {
        for operation in ["open", "goto", "reload", "back", "forward"] {
            assert!(
                validate_result(operation, json!({"tab": "t_test", "url": "about:blank"})).is_ok()
            );
            assert!(validate_result(operation, json!({"tab": "t_test"})).is_err());
        }
    }
}
