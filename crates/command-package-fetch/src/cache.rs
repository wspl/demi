//! The pages read, kept on the Host for 15 minutes (`web.md` § demi web
//! fetch), so that reading on fetches a page once: one file per URL under
//! the temporary directory's `demi-fetch/pages/`, on disk since the service
//! may exit between calls. Only text answered with a 2xx status is kept.

use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use demi_command_package_fetch_protocol::CACHE_SECONDS;
use serde::{Deserialize, Serialize};

/// A page read as text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    pub final_url: String,
    pub status: u16,
    pub media_type: Option<String>,
    pub html: bool,
    pub text: String,
}

/// How long a page is kept.
const KEPT: Duration = Duration::from_secs(CACHE_SECONDS);

/// The page `url` read within the last 15 minutes. One that cannot be read
/// is a miss: the fetch reads the page again and replaces it.
pub async fn read(url: &str) -> Option<Page> {
    let path = path(url);
    let modified = tokio::fs::metadata(&path).await.ok()?.modified().ok()?;
    if !fresh(modified) {
        return None;
    }
    let bytes = tokio::fs::read(&path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Keeps `page` as the one `url` read, and removes the pages kept longer
/// than 15 minutes. A page that cannot be kept is fetched again next time,
/// so a failure here only costs that fetch.
pub async fn write(url: &str, page: &Page) {
    let directory = crate::saved::directory().join(PAGES);
    let written = async {
        tokio::fs::create_dir_all(&directory).await?;
        let path = path(url);
        let partial = path.with_extension("partial");
        tokio::fs::write(&partial, serde_json::to_vec(page)?).await?;
        tokio::fs::rename(&partial, &path).await
    }
    .await;
    if let Err(error) = written {
        tracing::warn!("a fetched page could not be kept: {error}");
    }
    let Ok(mut entries) = tokio::fs::read_dir(&directory).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let stale = entry
            .metadata()
            .await
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| !fresh(modified));
        if stale {
            // Another call may have removed or replaced it first.
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

/// The directory of the pages, under the package's own.
const PAGES: &str = "pages";

fn path(url: &str) -> PathBuf {
    crate::saved::directory()
        .join(PAGES)
        .join(format!("{}.json", crate::saved::digest(url)))
}

fn fresh(modified: SystemTime) -> bool {
    modified
        .elapsed()
        .is_ok_and(|elapsed| elapsed < KEPT)
}
