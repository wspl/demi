//! The downloads a user starts in a watched tab (`live-view.md` § A browser
//! tab in the panel): the browser saves them in its download directory, as it
//! saves the agent's, and each is named there after the file the page
//! suggested once it is complete, so the Host's files show it by its name.
//! The page shows them as a browser's download bubble does.

use std::path::{Path, PathBuf};

use chromiumoxide::cdp::browser_protocol::browser::{
    DownloadProgressState, EventDownloadProgress, EventDownloadWillBegin,
};
use futures_util::StreamExt;
use tokio::sync::watch;
use tokio_util::task::TaskTracker;

use crate::driver::operation::Result;
use crate::tabs::tab::BrowserTab;
use demi_command_package_browser_protocol::live::{DownloadState, LiveDownload, MAX_DOWNLOADS};

/// The longest file name the protocol carries, in characters.
const NAME_CHARS: usize = 255;
/// A name taken this many times over gives up on a numbered one.
const NUMBERED: u32 = 1000;

/// Follows the downloads that start in `tab` until it ends, into
/// `downloads`. A download that starts while an agent's command holds the
/// tab is the command's: its `download` saves and moves the file itself, and
/// a click's download is not the user's.
pub(crate) async fn follow(
    tab: &BrowserTab,
    directory: PathBuf,
    downloads: watch::Sender<Vec<LiveDownload>>,
    tasks: &TaskTracker,
) -> Result<()> {
    let browser = tab.browser().call()?;
    let mut beginnings = browser.event_listener::<EventDownloadWillBegin>().await?;
    let mut progress = browser.event_listener::<EventDownloadProgress>().await?;
    drop(browser);
    let tab = tab.clone();
    tasks.spawn(async move {
        loop {
            tokio::select! {
                _ = tab.ended().cancelled() => break,
                beginning = beginnings.next() => match beginning {
                    Some(Ok(beginning)) => {
                        if tab.state().gate.held() || !owns(&tab, &beginning.frame_id).await {
                            continue;
                        }
                        let download = LiveDownload {
                            id: beginning.guid.clone(),
                            name: file_name(&beginning.suggested_filename),
                            state: DownloadState::InProgress,
                            received: 0,
                            total: 0,
                            path: String::new(),
                        };
                        downloads.send_modify(|list| {
                            list.push(download);
                            let excess = list.len().saturating_sub(MAX_DOWNLOADS);
                            list.drain(..excess);
                        });
                    }
                    // A lagged listener lost a beginning; the download goes on without a bubble.
                    Some(Err(_)) => {}
                    None => break,
                },
                event = progress.next() => match event {
                    Some(Ok(event)) => advance(&directory, &downloads, &event).await,
                    // A lagged listener lost a progress; the next one says where it stands.
                    Some(Err(_)) => {}
                    None => break,
                },
            }
        }
    });
    Ok(())
}

/// Whether `frame` is a document of `tab`, cross-site frames included.
async fn owns(tab: &BrowserTab, frame: &chromiumoxide::cdp::browser_protocol::page::FrameId) -> bool {
    match crate::driver::frames::capture(tab.page()).await {
        Ok(snapshot) => snapshot.frames.iter().any(|document| &document.frame.id == frame),
        // A tab going away keeps no downloads.
        Err(_) => false,
    }
}

/// Applies a progress event to the download it names, if it is one the user
/// started: a complete one is named after its file.
async fn advance(
    directory: &Path,
    downloads: &watch::Sender<Vec<LiveDownload>>,
    event: &EventDownloadProgress,
) {
    let Some(name) = downloads
        .borrow()
        .iter()
        .find(|download| download.id == event.guid && download.state == DownloadState::InProgress)
        .map(|download| download.name.clone())
    else {
        return;
    };
    let received = event.received_bytes.max(0.0) as u64;
    let total = event.total_bytes.max(0.0) as u64;
    let (state, path) = match event.state {
        DownloadProgressState::InProgress => (DownloadState::InProgress, String::new()),
        DownloadProgressState::Canceled => (DownloadState::Canceled, String::new()),
        DownloadProgressState::Completed => match name_file(directory, &event.guid, &name).await {
            Ok(path) => (DownloadState::Complete, path.to_string_lossy().into_owned()),
            Err(error) => {
                // The file stays under the browser's own name, which the
                // retirement removes; the bubble cannot offer it.
                tracing::warn!("live view download {}: {error}", event.guid);
                (DownloadState::Canceled, String::new())
            }
        },
    };
    downloads.send_if_modified(|list| {
        let Some(download) = list.iter_mut().find(|download| download.id == event.guid) else {
            return false;
        };
        let next = LiveDownload {
            state,
            received,
            // A complete file's size is what came, when the server never said it.
            total: if total == 0 && state == DownloadState::Complete { received } else { total },
            path,
            ..download.clone()
        };
        if *download == next {
            return false;
        }
        *download = next;
        true
    });
}

/// Moves the browser's file of the download `guid` to `name` in the same
/// directory, numbered as a browser numbers a name already taken:
/// `report (1).pdf`. A name is taken by linking, which never replaces a file.
async fn name_file(directory: &Path, guid: &str, name: &str) -> std::io::Result<PathBuf> {
    let source = directory.join(guid);
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    for number in 0..NUMBERED {
        let candidate = if number == 0 {
            directory.join(name)
        } else {
            directory.join(format!("{stem} ({number}){extension}"))
        };
        match tokio::fs::hard_link(&source, &candidate).await {
            Ok(()) => {
                tokio::fs::remove_file(&source).await?;
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other(format!("every name for {name} is taken")))
}

/// The file name a page suggested, as data: its last component, without
/// control characters, never empty, `.` or `..`.
fn file_name(suggested: &str) -> String {
    let last = suggested.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .filter(|character| !character.is_control())
        .take(NAME_CHARS)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return "download".to_owned();
    }
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_suggested_name_is_a_file_name_in_the_directory() {
        assert_eq!(file_name("report.pdf"), "report.pdf");
        assert_eq!(file_name("../../etc/passwd"), "passwd");
        assert_eq!(file_name("a\\b\\c.txt"), "c.txt");
        assert_eq!(file_name(".."), "download");
        assert_eq!(file_name(""), "download");
        assert_eq!(file_name("tab\tname.txt"), "tabname.txt");
    }
}
