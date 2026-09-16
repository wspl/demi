//! Collect page content from a registered batch of temporary environment tabs.

use demi_command_service::InvocationContext;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    BrowserEnvironment, BrowserError, BrowserTab, Result,
    navigation::Navigation,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
    protocol::{BrowserCommand, INLINE_BYTES},
};

/// Keep every temporary tab registered until collection and cleanup finish.
pub(super) async fn execute(
    context: &InvocationContext,
    environment: &BrowserEnvironment,
    _tab: Option<&BrowserTab>,
    command: &BrowserCommand,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<Value> {
    let BrowserCommand::ContentFetch(input) = command else {
        unreachable!("fetch dispatch accepts only content fetch");
    };
    // Validate all URLs before creating any tab or allowing page side effects.
    for url in &input.url {
        super::navigation::validate_url(url)?;
    }
    let caller = context.request.caller.as_deref().ok_or_else(|| {
        BrowserError::Configuration("temporary browser tabs require a trusted caller".into())
    })?;
    let batch = environment
        .temporary_tabs(caller, input.url.len(), cancel, deadline)
        .await?;
    let mut pages = Vec::with_capacity(input.url.len());
    let mut truncated = false;
    let collection = async {
        for (tab, requested) in batch.tabs().iter().zip(&input.url) {
            let operation = Operation::until(&tab.ended, cancel, deadline);
            let item = async {
                let mut references = tab.state.operations.try_lock().map_err(|_| BrowserError::Busy)?;
                let url = tab.navigate(Navigation::Url(requested.clone()), "domcontentloaded", &operation, &mut references).await?;
                let content = operation.run(tab.content(input.format.as_deref().unwrap_or("text"), &mut references)).await?;
                let title = operation.run(async { Ok(tab.page.get_title().await?.unwrap_or_default()) }).await?;
                Ok((url, title, content))
            }.await;
            match item {
                Ok((url, title, content)) => {
                    let limit = INLINE_BYTES / input.url.len().max(1) / 2;
                    let end = content.floor_char_boundary(content.len().min(limit));
                    truncated |= end < content.len();
                    pages.push(json!({"requestedUrl": requested, "url": url, "title": title, "content": &content[..end]}));
                }
                Err(error) => {
                    let error = if tab.ended.is_cancelled()
                        && !environment.ended.is_cancelled()
                        && !cancel.is_cancelled()
                    {
                        BrowserError::TabNotFound
                    } else {
                        error
                    };
                    if matches!(error.code(), "cancelled" | "timeout" | "browser_lost") {
                        return Err(error);
                    }
                    pages.push(json!({"requestedUrl": requested, "url": requested, "title": "", "content": "", "error": {"code": error.code(), "message": error.to_string(), "details": error.details()}}));
                }
            }
        }
        Ok(json!({"pages": pages, "truncated": truncated}))
    }.await;
    // The batch owns partial creation and already-closed targets. Joining its
    // cleanup releases the environment hold only after all targets are gone.
    let cleanup = batch.close(CONTROL_TIMEOUT).await;
    after_cleanup(collection, cleanup)
}
