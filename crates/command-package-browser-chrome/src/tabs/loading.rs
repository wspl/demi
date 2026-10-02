//! Whether a tab loads its top-level page (`live-view.md` § The tab
//! methods): Chrome's frame events of the tab's main frame, followed for the
//! tab's lifetime. Each change counts as a change of what the browser shows,
//! so the live view sends its viewers the tab list again and the work panel
//! shows the page loading only while it does.

use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::page::{
    EventFrameStartedLoading, EventFrameStoppedLoading, GetFrameTreeParams,
};
use futures_util::StreamExt;
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::driver::operation::Result;

/// Follows the main frame's loading of `page` until `ended`, counting each
/// change in `changes`.
pub(crate) async fn observe(
    page: &Page,
    ended: CancellationToken,
    tasks: &TaskTracker,
    changes: watch::Sender<u64>,
) -> Result<watch::Sender<bool>> {
    let mut started = page.event_listener::<EventFrameStartedLoading>().await?;
    let mut stopped = page.event_listener::<EventFrameStoppedLoading>().await?;
    let main = page
        .execute(GetFrameTreeParams {})
        .await?
        .result
        .frame_tree
        .frame
        .id;
    // The tab is followed from its registration, before its page can be
    // operated, so every load after that is seen from its start.
    let loading = watch::channel(false).0;
    let follows = loading.clone();
    tasks.spawn(async move {
        loop {
            let now = tokio::select! {
                _ = ended.cancelled() => break,
                event = started.next() => match event {
                    Some(Ok(event)) if event.frame_id == main => true,
                    Some(_) => continue,
                    None => break,
                },
                event = stopped.next() => match event {
                    Some(Ok(event)) if event.frame_id == main => false,
                    Some(_) => continue,
                    None => break,
                },
            };
            if follows.send_replace(now) != now {
                changes.send_modify(|revision| *revision += 1);
            }
        }
    });
    Ok(loading)
}
