//! Observe downloads before input and publish only completed Host files.

use std::{path::PathBuf, sync::Arc};

use chromiumoxide::cdp::browser_protocol::{
    browser::{
        CancelDownloadParams, DownloadProgressState, EventDownloadProgress, EventDownloadWillBegin,
    },
    dom::GetNodeForLocationParams,
    input::MouseButton,
    network::EventResponseReceived,
    page::GetResourceTreeParams,
};
use demi_command_service::InvocationContext;
use futures_util::{FutureExt, StreamExt};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    BrowserEnvironment, BrowserError, BrowserTab, Result, element,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
    protocol::BrowserCommand,
};

enum Phase {
    NotStarted,
    Triggered,
    Active(Arc<EventDownloadWillBegin>),
    Finished(Arc<EventDownloadWillBegin>),
}

/// Save the download caused by this action without replaying the action.
pub(super) async fn execute(
    context: &InvocationContext,
    environment: &BrowserEnvironment,
    tab: Option<&BrowserTab>,
    command: &BrowserCommand,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<Value> {
    let tab = tab.ok_or(BrowserError::TabNotFound)?;
    let BrowserCommand::Download(input) = command else {
        unreachable!("download dispatch accepts only download");
    };
    let operation = Operation::for_tab(tab, cancel, deadline);
    let mut references = tab
        .state
        .operations
        .try_lock()
        .map_err(|_| operation.failure(BrowserError::Busy, &tab.id(), None))?;
    let browser = environment.browser.upgrade().ok_or(BrowserError::Closed)?;
    let (mut beginnings, mut progress) = operation
        .run(async {
            let browser = browser.lock().await;
            Ok((
                browser.event_listener::<EventDownloadWillBegin>().await?,
                browser.event_listener::<EventDownloadProgress>().await?,
            ))
        })
        .await?;
    let mut responses = operation
        .run(async { Ok(tab.page.event_listener::<EventResponseReceived>().await?) })
        .await?;
    let frames = operation
        .run(super::frames::capture(&tab.page))
        .await?
        .frames;
    let mut phase = Phase::NotStarted;
    let mut work = async {
        let output = match &input.output {
            Some(output) => super::output::preflight(&context.request.cwd, output, input.overwrite == Some(true)).await?,
            None => std::env::temp_dir().join(super::handles::fresh("demi-download")?),
        };
        let target = command.target();
        let point = match &input.xy {
            Some(xy) => operation.run(tab.coordinates(xy)).await?,
            None => {
                let target = target.as_ref().ok_or_else(|| BrowserError::Configuration("download requires an element or --xy".into()))?;
                tab.ready_element(target, &mut references, element::CLICK, &operation).await?.1.point()
            }
        };
        if input.xy.is_some() {
            let hit = operation.run(async {
                Ok(tab.page.execute(GetNodeForLocationParams::new(point.x.floor() as i64, point.y.floor() as i64)).await?.result)
            }).await?;
            let media_page = &frames.iter().find(|document| document.frame.id == hit.frame_id)
                .ok_or(BrowserError::TargetNotFound)?.page;
            let media = operation.run(element::TargetElement::resolve(media_page, hit.backend_node_id)).await?;
            let source: Option<String> = operation.run(element::call(&tab.page, &media,
                "function() { return ['img','video','audio'].includes(this.localName) ? (this.currentSrc || this.src || null) : null; }", vec![])).await?;
            if let Some(source) = source {
                let conditions = element::prepared_state(&tab.page, &media, element::CLICK, true, &operation).await?;
                if conditions.failed.is_some() { return Err(conditions.failure()); }
                let tree = operation.run(async { Ok(media_page.execute(GetResourceTreeParams {}).await?.result.frame_tree) }).await?;
                let mut pending = vec![tree];
                let mut mime = None;
                while let Some(frame) = pending.pop() {
                    pending.extend(frame.child_frames.unwrap_or_default());
                    if frame.frame.id == hit.frame_id {
                        mime = frame.resources.into_iter().find(|resource| resource.url == source).map(|resource| resource.mime_type);
                    }
                }
                let bytes = operation.run(super::assets::resource_bytes(media_page, &hit.frame_id, &source)).await?;
                let count = bytes.len();
                let suggested = url::Url::parse(&source).ok().and_then(|url| url.path_segments().and_then(|mut parts| parts.next_back()).map(str::to_owned)).unwrap_or_default();
                let path = super::output::save_with_overwrite(&context.request.cwd, &output.to_string_lossy(), bytes, input.overwrite == Some(true), cancel, deadline).await?;
                return Ok(json!({"path":path,"suggestedFilename":suggested,"bytes":count,"mimeType":mime.unwrap_or_else(||"application/octet-stream".into())}));
            }
        }
        phase = Phase::Triggered;
        tab.click_at(point, MouseButton::Left, 1, 0, &operation).await?;
        let beginning = operation.run(async {
            loop {
                let event = beginnings.next().await.ok_or(BrowserError::Closed)??;
                if frames.iter().any(|document| document.frame.id == event.frame_id) { return Ok(event); }
            }
        }).await?;
        phase = Phase::Active(beginning.clone());
        operation.run(async {
            loop {
                let event = progress.next().await.ok_or(BrowserError::Closed)??;
                if event.guid != beginning.guid { continue; }
                match event.state {
                    DownloadProgressState::Completed => return Ok(()),
                    DownloadProgressState::Canceled => {
                        phase = Phase::Finished(beginning.clone());
                        return Err(BrowserError::Io(std::io::Error::other("Chrome cancelled the download")));
                    }
                    DownloadProgressState::InProgress => {}
                }
            }
        }).await?;
        phase = Phase::Finished(beginning.clone());
        let source = spool_path(environment, &beginning.guid)?;
        let bytes = operation.run(async { Ok(tokio::fs::metadata(&source).await?.len()) }).await?;
        let mut mime = "application/octet-stream".to_owned();
        while let Some(Some(response)) = responses.next().now_or_never() {
            let response = response?;
            if response.response.url == beginning.url { mime = response.response.mime_type.clone(); }
        }
        let path = super::output::publish_file(&context.request.cwd, &output.to_string_lossy(), source, input.overwrite == Some(true), cancel, deadline).await?;
        Ok(json!({"path": path, "suggestedFilename": beginning.suggested_filename, "bytes": bytes, "mimeType": mime}))
    }.await;
    // A click can finish before its download event arrives. On failure retain
    // the installed observation during bounded cleanup, then cancel that GUID.
    if work.is_err() && matches!(phase, Phase::Triggered) {
        let pending = tokio::time::timeout(CONTROL_TIMEOUT, async {
            while let Some(event) = beginnings.next().await {
                let event = event.map_err(BrowserError::from)?;
                if frames
                    .iter()
                    .any(|document| document.frame.id == event.frame_id)
                {
                    return Ok::<_, BrowserError>(Some(event));
                }
            }
            Ok(None)
        })
        .await;
        match pending {
            Ok(Ok(Some(found))) => phase = Phase::Active(found),
            Ok(Ok(None)) => {}
            // No observed download remains owned by this invocation. A future
            // page timer is a later page action and is never replayed here.
            Err(_) => {}
            Ok(Err(error)) => {
                work = after_cleanup(work, Err(error));
            }
        }
    }
    let file_cleanup = async {
        if let Phase::Active(download) | Phase::Finished(download) = &phase {
            let source = spool_path(environment, &download.guid)?;
            let stopping = if matches!(phase, Phase::Active(_)) {
                tokio::time::timeout(CONTROL_TIMEOUT, async {
                    browser
                        .lock()
                        .await
                        .execute(CancelDownloadParams::new(download.guid.clone()))
                        .await?;
                    while let Some(event) = progress.next().await {
                        let event = event?;
                        if event.guid == download.guid
                            && event.state != DownloadProgressState::InProgress
                        {
                            return Ok::<_, BrowserError>(());
                        }
                    }
                    Err(BrowserError::Closed)
                })
                .await
                .map_err(|_| BrowserError::Timeout)
                .and_then(std::convert::identity)
            } else {
                Ok(())
            };
            let mut removal = Ok(());
            for path in [
                source,
                environment
                    .download_directory
                    .join(format!("{}.crdownload", download.guid)),
            ] {
                let result = match tokio::fs::remove_file(path).await {
                    Ok(()) => Ok(()),
                    // Chrome removes cancelled partials itself on some platforms.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(BrowserError::Io(error)),
                };
                removal = after_cleanup(removal, result);
            }
            after_cleanup(stopping, removal)?;
        }
        Ok(())
    }
    .await;
    let cleanup = after_cleanup(file_cleanup, tab.release_objects().await);
    drop(beginnings);
    drop(progress);
    drop(responses);
    after_cleanup(work, cleanup).map_err(|error| operation.failure(error, &tab.id(), None))
}

/// Resolve a Chrome download GUID inside the environment's private spool.
fn spool_path(environment: &BrowserEnvironment, guid: &str) -> Result<PathBuf> {
    if guid.is_empty()
        || !guid
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(BrowserError::Cdp(chromiumoxide::error::CdpError::msg(
            "download identifier is not a safe spool name",
        )));
    }
    Ok(environment.download_directory.join(guid))
}
