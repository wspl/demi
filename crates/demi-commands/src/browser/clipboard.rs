//! Use the managed browser's isolated Clipboard API with bounded raw input.

use base64::Engine;
use chromiumoxide::cdp::{
    browser_protocol::browser::PermissionType, js_protocol::runtime::EvaluateParams,
};
// The browser design requires this context-scoped permission grant; upstream marks it deprecated.
#[allow(deprecated)]
use chromiumoxide::cdp::browser_protocol::browser::GrantPermissionsParams;
use demi_command_service::InvocationContext;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    BrowserEnvironment, BrowserError, BrowserTab, Result,
    operation::Operation,
    protocol::{
        BrowserOperation, CLIPBOARD_PNG_BYTES, CLIPBOARD_PNG_PIXELS, Capability, ClipboardItem,
        ClipboardMime, ClipboardReadResult, ClipboardWriteResult, STDIN_BYTES,
    },
};

/// Why the pinned headless clipboard may not be the browser's own, or none
/// where its isolation from the Host user's clipboard is verified.
pub(super) fn unisolated() -> Option<&'static str> {
    if cfg!(target_os = "macos")
        || (cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_none())
    {
        None
    } else {
        Some(
            "clipboard isolation from the Host user's system clipboard has not been verified for this platform configuration",
        )
    }
}

/// Publish the pinned headless clipboard's verified platform isolation policy.
pub(super) async fn capability(page: &chromiumoxide::Page) -> Result<Capability> {
    let reason = match unisolated() {
        Some(reason) => Some(reason),
        None => {
            let available: bool = page
                .evaluate_expression(
                    "Boolean(navigator.clipboard && typeof ClipboardItem === 'function')",
                )
                .await?
                .into_value()
                .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
            (!available).then_some(
                "the current document does not expose the Clipboard API; use a secure context",
            )
        }
    };
    Ok(Capability {
        id: "clipboard".into(),
        available: reason.is_none(),
        reason: reason.map(str::to_owned),
        schema: reason.is_none().then(|| {
            json!({"mimeTypes": ["text/plain", "text/html", "image/png"], "help": "demi browser clipboard --help"})
        }),
    })
}

/// Lets pages use the browser's own clipboard.
pub(super) async fn grant(
    browser: &std::sync::Weak<tokio::sync::Mutex<chromiumoxide::Browser>>,
) -> Result<()> {
    let browser = browser.upgrade().ok_or(BrowserError::Closed)?;
    #[allow(deprecated)]
    let permissions = GrantPermissionsParams::new(vec![
        PermissionType::ClipboardReadWrite,
        PermissionType::ClipboardSanitizedWrite,
    ]);
    browser.lock().await.execute(permissions).await?;
    Ok(())
}

/// Read or replace browser clipboard data only after permission and input validation.
pub(super) async fn execute(
    context: &mut InvocationContext,
    environment: &BrowserEnvironment,
    tab: Option<&BrowserTab>,
    command: &BrowserOperation,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<Value> {
    let tab = tab.ok_or(BrowserError::TabNotFound)?;
    let operation = Operation::for_tab(tab, cancel, deadline);
    let capability = operation.run(capability(&tab.page)).await?;
    if !capability.available {
        return Err(BrowserError::UnsupportedCapability(
            capability
                .reason
                .unwrap_or_else(|| "clipboard unavailable".into()),
        ));
    }
    let _guard = tab
        .state
        .operations
        .try_lock()
        .map_err(|_| BrowserError::Busy)?;
    operation.run(grant(&environment.browser)).await?;
    match command {
        BrowserOperation::ClipboardWrite(input) => {
            let mime = input.mime.unwrap_or_default();
            let maximum = if mime == ClipboardMime::ImagePng {
                CLIPBOARD_PNG_BYTES
            } else {
                STDIN_BYTES
            };
            let mut bytes = Vec::new();
            loop {
                let chunk = operation
                    .run(async {
                        context
                            .input
                            .next()
                            .await
                            .map_err(|error| BrowserError::Configuration(error.to_string()))
                    })
                    .await?;
                let Some(chunk) = chunk else {
                    break;
                };
                if bytes.len().saturating_add(chunk.len()) > maximum {
                    return Err(BrowserError::Configuration(format!(
                        "{mime} clipboard input exceeds {maximum} bytes"
                    )));
                }
                bytes.extend_from_slice(&chunk);
            }
            let bytes = validate_payload(mime, bytes).await?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let script = format!(
                r#"(async () => {{
                const bytes = Uint8Array.from(atob({}), value => value.charCodeAt(0));
                await navigator.clipboard.write([new ClipboardItem({{[{}]: new Blob([bytes], {{type:{}}})}})]);
                return true;
            }})()"#,
                json!(encoded),
                json!(mime),
                json!(mime)
            );
            // No listener or page-side timer survives this one promise. The call
            // cannot be undone once the browser accepted the clipboard write.
            operation
                .run(async {
                    operation.begin_input();
                    let written: bool = tab
                        .page
                        .evaluate_expression(
                            EvaluateParams::builder()
                                .expression(script)
                                .await_promise(true)
                                .return_by_value(true)
                                .build()
                                .map_err(BrowserError::Configuration)?,
                        )
                        .await?
                        .into_value()
                        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
                    if !written {
                        return Err(BrowserError::InvalidResult(
                            "clipboard write did not complete".into(),
                        ));
                    }
                    operation.complete_input();
                    super::output::value(ClipboardWriteResult {
                        mime_type: mime,
                        bytes: bytes.len(),
                    })
                })
                .await
                .map_err(|error| operation.failure(error, tab.id().as_str(), None))
        }
        BrowserOperation::ClipboardRead(input) => {
            if input.format.is_some() == input.output_dir.is_some() {
                return Err(BrowserError::Configuration(
                    "clipboard read requires --format text or --output-dir".into(),
                ));
            }
            if input.format.is_some() {
                let text: String = operation
                    .run(async {
                        tab.page
                            .evaluate_expression(
                                EvaluateParams::builder()
                                    .expression("navigator.clipboard.readText()")
                                    .await_promise(true)
                                    .return_by_value(true)
                                    .build()
                                    .map_err(BrowserError::Configuration)?,
                            )
                            .await?
                            .into_value()
                            .map_err(|error| BrowserError::InvalidResult(error.to_string()))
                    })
                    .await?;
                if text.len() > STDIN_BYTES {
                    return Err(BrowserError::ResultTooLarge);
                }
                return super::output::value(ClipboardReadResult::Text { text });
            }
            let script = format!(
                r#"(async () => {{
                const result = [];
                let total = 0;
                for (const item of await navigator.clipboard.read()) {{
                    for (const mimeType of item.types) {{
                        if (!['text/plain','text/html','image/png'].includes(mimeType)) continue;
                        const blob = await item.getType(mimeType);
                        const limit = mimeType === 'image/png' ? {} : {};
                        total += blob.size;
                        if (blob.size > limit || total > {}) throw new Error('clipboard result exceeds the byte limit');
                        const bytes = new Uint8Array(await blob.arrayBuffer());
                        let text = '';
                        for (let offset = 0; offset < bytes.length; offset += 32768) text += String.fromCharCode(...bytes.subarray(offset, offset + 32768));
                        result.push({{mimeType, data:btoa(text)}});
                    }}
                }}
                return result;
            }})()"#,
                CLIPBOARD_PNG_BYTES, STDIN_BYTES, CLIPBOARD_PNG_BYTES
            );
            let items: Vec<Entry> = operation
                .run(async {
                    tab.page
                        .evaluate_expression(
                            EvaluateParams::builder()
                                .expression(script)
                                .await_promise(true)
                                .return_by_value(true)
                                .build()
                                .map_err(BrowserError::Configuration)?,
                        )
                        .await?
                        .into_value()
                        .map_err(|error| BrowserError::InvalidResult(error.to_string()))
                })
                .await?;
            let directory = crate::files::resolve_path(
                &context.request.cwd,
                input
                    .output_dir
                    .as_ref()
                    .expect("output directory selected"),
            )
            .map_err(BrowserError::Configuration)?;
            operation
                .run(async {
                    tokio::fs::create_dir_all(&directory).await?;
                    Ok(())
                })
                .await?;
            let mut decoded = Vec::with_capacity(items.len());
            for (index, item) in items.into_iter().enumerate() {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(item.data)
                    .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
                let mime = item.mime_type;
                let extension = match mime {
                    ClipboardMime::TextPlain => "txt",
                    ClipboardMime::TextHtml => "html",
                    ClipboardMime::ImagePng => "png",
                };
                let bytes = validate_payload(mime, bytes).await?;
                let path = directory.join(format!("item-{index}.{extension}"));
                super::output::preflight(
                    &context.request.cwd,
                    &path.to_string_lossy(),
                    input.overwrite == Some(true),
                )
                .await?;
                decoded.push((mime, path, bytes));
            }
            let mut saved = Vec::with_capacity(decoded.len());
            for (mime, path, bytes) in decoded {
                let count = bytes.len();
                let path = super::output::save_with_overwrite(
                    &context.request.cwd,
                    &path.to_string_lossy(),
                    bytes,
                    input.overwrite == Some(true),
                    cancel,
                    deadline,
                )
                .await?;
                saved.push(ClipboardItem {
                    mime_type: mime,
                    path,
                    bytes: count,
                });
            }
            super::output::value(ClipboardReadResult::Items { items: saved })
        }
        _ => unreachable!("clipboard dispatch accepts only read/write"),
    }
}

/// Validate browser clipboard bytes before replacing data or exporting a file.
async fn validate_payload(mime: ClipboardMime, bytes: Vec<u8>) -> Result<Vec<u8>> {
    let maximum = if mime == ClipboardMime::ImagePng {
        CLIPBOARD_PNG_BYTES
    } else {
        STDIN_BYTES
    };
    if bytes.len() > maximum {
        return Err(BrowserError::ResultTooLarge);
    }
    if mime != ClipboardMime::ImagePng {
        std::str::from_utf8(&bytes).map_err(|error| {
            BrowserError::Configuration(format!("clipboard text is not UTF-8: {error}"))
        })?;
        return Ok(bytes);
    }
    tokio::task::spawn_blocking(move || {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        decoder.set_limits(png::Limits {
            bytes: CLIPBOARD_PNG_PIXELS * 8,
        });
        let mut reader = decoder.read_info().map_err(|error| {
            BrowserError::Configuration(format!("invalid clipboard PNG: {error}"))
        })?;
        let pixels = u64::from(reader.info().width) * u64::from(reader.info().height);
        if pixels > u64::try_from(CLIPBOARD_PNG_PIXELS).expect("pixel limit fits u64") {
            return Err(BrowserError::ResultTooLarge);
        }
        let size = reader
            .output_buffer_size()
            .ok_or(BrowserError::ResultTooLarge)?;
        if size > CLIPBOARD_PNG_PIXELS * 8 {
            return Err(BrowserError::ResultTooLarge);
        }
        let mut output = vec![0; size];
        reader.next_frame(&mut output).map_err(|error| {
            BrowserError::Configuration(format!("invalid clipboard PNG: {error}"))
        })?;
        reader.finish().map_err(|error| {
            BrowserError::Configuration(format!("invalid clipboard PNG: {error}"))
        })?;
        drop(reader);
        Ok(bytes)
    })
    .await?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Entry {
    mime_type: ClipboardMime,
    data: String,
}

