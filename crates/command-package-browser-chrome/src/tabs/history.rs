//! Whether a tab's history has a page to go back or forward to
//! (`live-view.md` § The tab methods): Chrome's navigation history of the
//! tab, read again after each navigation of its main frame. Each change of
//! them counts as a change of what the browser shows, so the live view sends
//! its viewers the tab list again.

use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::page::{
    EventFrameNavigated, EventNavigatedWithinDocument, GetFrameTreeParams,
    GetNavigationHistoryParams,
};
use futures_util::StreamExt;
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::driver::operation::Result;

/// Which ends of its history a tab is away from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryEnds {
    /// A page before the current one.
    pub back: bool,
    /// A page after the current one.
    pub forward: bool,
}

/// The history ends of `page` as Chrome has them now.
async fn read(page: &Page) -> Result<HistoryEnds> {
    let history = page.execute(GetNavigationHistoryParams {}).await?.result;
    let last = i64::try_from(history.entries.len()).unwrap_or(i64::MAX) - 1;
    Ok(HistoryEnds {
        back: history.current_index > 0,
        forward: history.current_index < last,
    })
}

/// Follows the history ends of `page` until `ended`, counting each change
/// in `changes`.
pub(crate) async fn observe(
    page: &Page,
    ended: CancellationToken,
    tasks: &TaskTracker,
    changes: watch::Sender<u64>,
) -> Result<watch::Sender<HistoryEnds>> {
    let mut navigated = page.event_listener::<EventFrameNavigated>().await?;
    let mut within = page.event_listener::<EventNavigatedWithinDocument>().await?;
    let main = page
        .execute(GetFrameTreeParams {})
        .await?
        .result
        .frame_tree
        .frame
        .id;
    let ends = watch::channel(read(page).await?).0;
    let follows = ends.clone();
    let page = page.clone();
    tasks.spawn(async move {
        loop {
            tokio::select! {
                _ = ended.cancelled() => break,
                event = navigated.next() => match event {
                    Some(Ok(event)) if event.frame.parent_id.is_none() => {}
                    Some(_) => continue,
                    None => break,
                },
                event = within.next() => match event {
                    Some(Ok(event)) if event.frame_id == main => {}
                    Some(_) => continue,
                    None => break,
                },
            }
            let now = tokio::select! {
                _ = ended.cancelled() => break,
                now = read(&page) => now,
            };
            // A page that cannot answer is closing or replaced; the next
            // navigation, if any, reads the history again.
            let Ok(now) = now else {
                continue;
            };
            if follows.send_replace(now) != now {
                changes.send_modify(|revision| *revision += 1);
            }
        }
    });
    Ok(ends)
}
