//! A tab's icon (`live-view.md` § A browser tab in the panel): what its page
//! names, or its site's `/favicon.ico`, drawn 32 pixels square in a world of
//! its own beside the page's scripts each time the main frame stops loading,
//! as a web browser's tab shows the site's icon. Each change counts as a
//! change of what the browser shows, so the tab list carries it.

use std::time::Duration;

use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::page::{
    CreateIsolatedWorldParams, EventFrameStoppedLoading, FrameId, GetFrameTreeParams,
};
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use demi_command_package_browser_protocol::browser::FAVICON_LENGTH;
use futures_util::StreamExt;
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::driver::operation::{BrowserError, Result};
use crate::tabs::tab::BrowserTab;

/// How long drawing an icon may take: a site that does not answer for its
/// icon leaves the tab without one.
const DRAW_TIME: Duration = Duration::from_secs(10);

/// What the tab's PNG `data:` URL starts with.
const PNG_URL: &str = "data:image/png;base64,";

/// The page's icon as `favicon.js` draws it in the frame `frame`; none for a
/// page without one, or whose icon did not load or draw.
async fn draw(page: &Page, frame: &FrameId) -> Result<Option<String>> {
    let world = CreateIsolatedWorldParams::builder()
        .frame_id(frame.clone())
        .world_name("demi-favicon")
        .build()
        .map_err(BrowserError::Configuration)?;
    let context = page.execute(world).await?.result.execution_context_id;
    let evaluate = EvaluateParams::builder()
        .expression(include_str!("favicon.js"))
        .context_id(context)
        .await_promise(true)
        .return_by_value(true)
        .build()
        .map_err(BrowserError::Configuration)?;
    let drawn = page.execute(evaluate).await?.result.result.value;
    Ok(drawn
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|url| url.starts_with(PNG_URL) && url.len() <= FAVICON_LENGTH))
}

/// Follows the icon of `page` until `ended`, counting each change in
/// `changes`.
pub(crate) async fn observe(
    page: &Page,
    ended: CancellationToken,
    tasks: &TaskTracker,
    changes: watch::Sender<u64>,
) -> Result<watch::Sender<Option<String>>> {
    let mut stopped = page.event_listener::<EventFrameStoppedLoading>().await?;
    let main = page
        .execute(GetFrameTreeParams {})
        .await?
        .result
        .frame_tree
        .frame
        .id;
    let icon = watch::channel(None).0;
    let follows = icon.clone();
    let page = page.clone();
    tasks.spawn(async move {
        loop {
            tokio::select! {
                _ = ended.cancelled() => break,
                event = stopped.next() => match event {
                    Some(Ok(event)) if event.frame_id == main => {}
                    Some(_) => continue,
                    None => break,
                },
            }
            let drawn = tokio::select! {
                _ = ended.cancelled() => break,
                drawn = tokio::time::timeout(DRAW_TIME, draw(&page, &main)) => drawn,
            };
            // A page that cannot answer, or whose icon takes too long, has
            // none; the next load draws it again.
            let now = drawn.ok().and_then(Result::ok).flatten();
            if follows.send_if_modified(|held| {
                let changed = *held != now;
                *held = now;
                changed
            }) {
                changes.send_modify(|revision| *revision += 1);
            }
        }
    });
    Ok(icon)
}

impl BrowserTab {
    /// The page's icon as a PNG `data:` URL, once its page drew one.
    pub fn favicon(&self) -> Option<String> {
        self.state.favicon.borrow().clone()
    }
}
