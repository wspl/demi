//! Whether a tab loads its top-level page (`live-view.md` § The tab
//! methods): Chrome's frame events of the tab's main frame, followed for the
//! tab's lifetime. Each change counts as a change of what the browser shows,
//! so the live view sends its viewers the tab list again and the work panel
//! shows the page loading only while it does.

use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::page::{
    EventFrameNavigated, EventFrameStartedLoading, EventFrameStoppedLoading, GetFrameTreeParams,
};
use futures_util::StreamExt;
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::driver::operation::Result;

/// Where a tab's top-level page is in its loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageLoad {
    /// The page does not load.
    Idle,
    /// A navigation started and its new document has not committed. Chrome
    /// holds every command for the page until the document commits or the
    /// navigation ends without one, which for an address that does not
    /// answer takes until its connection times out; the browser's own
    /// commands, such as a window's bounds, still apply at once.
    Navigating,
    /// The new document committed and still loads.
    Loading,
}

/// Follows the main frame's loading of `page` until `ended`, counting each
/// start and end of a load in `changes`.
pub(crate) async fn observe(
    page: &Page,
    ended: CancellationToken,
    tasks: &TaskTracker,
    changes: watch::Sender<u64>,
) -> Result<watch::Sender<PageLoad>> {
    let mut started = page.event_listener::<EventFrameStartedLoading>().await?;
    let mut committed = page.event_listener::<EventFrameNavigated>().await?;
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
    let load = watch::channel(PageLoad::Idle).0;
    let follows = load.clone();
    tasks.spawn(async move {
        loop {
            let now = tokio::select! {
                _ = ended.cancelled() => break,
                event = started.next() => match event {
                    Some(Ok(event)) if event.frame_id == main => PageLoad::Navigating,
                    Some(_) => continue,
                    None => break,
                },
                event = committed.next() => match event {
                    // A document that commits outside a load, such as one a
                    // load before this one committed late, changes nothing.
                    Some(Ok(event))
                        if event.frame.id == main && *follows.borrow() == PageLoad::Navigating =>
                    {
                        PageLoad::Loading
                    }
                    Some(_) => continue,
                    None => break,
                },
                event = stopped.next() => match event {
                    Some(Ok(event)) if event.frame_id == main => PageLoad::Idle,
                    Some(_) => continue,
                    None => break,
                },
            };
            // What the browser shows is whether the page loads, not whether
            // its document committed yet.
            let was = follows.send_replace(now);
            if (was == PageLoad::Idle) != (now == PageLoad::Idle) {
                changes.send_modify(|revision| *revision += 1);
            }
        }
    });
    Ok(load)
}
