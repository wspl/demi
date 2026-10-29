//! A document drawn in a browser of its own (`draw`).
//!
//! Chrome starts for the document with a profile of its own and no capture
//! extension, draws it at the width asked, writes its picture to a file and
//! ends. It opens no tab of a conversation's browser and shares no profile
//! with one, so a drawn page reads none of the conversation's sign-ins.
//!
//! - The picture waits for the document's load, its fonts and two frames,
//!   then a short settle for what its scripts draw after load. The page is
//!   told the reader prefers reduced motion, so a chart that honors it draws
//!   at once rather than animating in.
//! - Its height is the asked one, or the document's own, which it answers
//!   either way: the lowest bottom of the body and of every element in it,
//!   which holds what is positioned out of the flow, but not of what an
//!   element that clips holds, such as a scroller's rows.
//! - The picture is a file in a directory of its own beside the profiles,
//!   readable only by its user, which its caller reads as a Host file, since
//!   bulk bytes never travel in an answer. Each drawing removes its user's
//!   pictures older than `DRAWING_KEPT`.
//! - A drawing runs in a task of its own, its cleanup included, so a
//!   cancelled one answers at once whatever stage it is at, and its Chrome
//!   and directories go after: the resident gives a cancelled command only a
//!   few seconds.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use chromiumoxide::Browser;
use chromiumoxide::BrowserConfig;
use chromiumoxide::cdp::browser_protocol::emulation::{
    MediaFeature, SetDeviceMetricsOverrideParams, SetEmulatedMediaParams,
};
use chromiumoxide::cdp::browser_protocol::page::{
    CaptureScreenshotFormat, CaptureScreenshotParams, Viewport,
};
use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::driver::installation::Installation;
use crate::driver::operation::{BrowserError, CONTROL_TIMEOUT, Result, after_cleanup};
use crate::driver::process::ChromeProcess;
use crate::tabs::environment::{DirectoryBases, EnvironmentDirectories, REQUEST_TIMEOUT};
use demi_command_package_browser_protocol::browser::{DRAW_MAX_HEIGHT, DRAWING_KEPT, DrawInput, DrawResult};

/// The picture's pixels per CSS pixel: a Retina screen's.
const SCALE: f64 = 2.0;

/// The viewport's height while the document is laid out to be measured,
/// which a document that takes the viewport's height takes.
const MEASURE_HEIGHT: u32 = 800;

/// What the document has, at most, to load and settle.
const SETTLE_WITHIN: Duration = Duration::from_secs(15);

/// The name each picture's directory in the profiles' base starts with.
const DRAWING_PREFIX: &str = "demi-drawing-";

/// Resolves once the document has loaded, its fonts are ready, two frames
/// have drawn and 300 ms more have passed, answering its content's height.
const SETTLED: &str = r#"new Promise((settled) => {
  const clips = (element) => getComputedStyle(element).overflowY !== 'visible'
  const height = () => {
    let bottom = document.body.getBoundingClientRect().bottom
    const walk = (element) => {
      for (const child of element.children) {
        bottom = Math.max(bottom, child.getBoundingClientRect().bottom)
        if (!clips(child)) walk(child)
      }
    }
    // The body's overflow is the viewport's, which clips nothing it holds.
    walk(document.body)
    return Math.ceil(bottom + scrollY)
  }
  const done = () => document.fonts.ready.then(() =>
    requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(() => settled(height()), 300))))
  if (document.readyState === 'complete') done()
  else addEventListener('load', done, { once: true })
})"#;

/// Draws `input` with the pinned release installed as `installation`.
pub async fn draw(
    installation: Installation,
    input: &DrawInput,
    html: String,
    cancellation: &CancellationToken,
) -> Result<DrawResult> {
    let stop = CancellationToken::new();
    let drawing = tokio::spawn(drawn(installation, input.clone(), html, stop.clone()));
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            // The task goes on to clean up after itself.
            stop.cancel();
            Err(BrowserError::Cancelled)
        }
        joined = drawing => joined.map_err(|error| BrowserError::Io(std::io::Error::other(error)))?,
    }
}

/// The drawing of `input`, from Chrome's start to its directories' removal;
/// `cancellation` ends it early, its cleanup still run.
async fn drawn(
    installation: Installation,
    input: DrawInput,
    html: String,
    cancellation: CancellationToken,
) -> Result<DrawResult> {
    let input = &input;
    let cancellation = &cancellation;
    let version = crate::driver::installation::pinned_version()?;
    let Installation {
        executable,
        runtime,
    } = installation;
    let mut directories = EnvironmentDirectories::create(&DirectoryBases::host())?;
    let mut process = ChromeProcess::new(directories.runtime(), &executable);
    let builder = BrowserConfig::builder()
        .chrome_executable(executable)
        .user_data_dir(directories.profile())
        .launch_timeout(Duration::from_secs(60))
        .request_timeout(REQUEST_TIMEOUT);
    let config = crate::driver::launch::drawing(builder, &version, runtime.as_ref())
        .build()
        .map_err(BrowserError::Configuration)?;
    // From here only the cleanup below removes the directories: Chrome may
    // be writing into them, and an aborted drawing leaves them to the sweep.
    directories.keep_until_retired();
    let profile = directories.profile().to_owned();
    let launched = tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(BrowserError::Cancelled),
        result = Browser::launch_with(config, |command| {
            let command = crate::driver::launch::complete(command, None, runtime.as_ref());
            process.spawn(command, &profile)
        }) => result.map_err(BrowserError::from),
    };
    let (mut browser, mut handler) = match launched {
        Ok(launched) => launched,
        Err(error) => {
            let cleanup = directories.remove(process.terminate().await).await;
            return after_cleanup(Err(error), cleanup);
        }
    };
    let pump = AbortOnDropHandle::new(tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if event.is_err() {
                break;
            }
        }
    }));
    let drawn = tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(BrowserError::Cancelled),
        drawn = picture(&browser, input, &html) => drawn,
    };
    // Closing asks Chrome to end; one that does not is killed with its
    // process group either way.
    if !matches!(drawn, Err(BrowserError::Cancelled)) {
        let _closed = tokio::time::timeout(CONTROL_TIMEOUT, browser.close()).await;
    }
    let _ended = tokio::time::timeout(CONTROL_TIMEOUT, browser.wait()).await;
    drop(browser);
    drop(pump);
    let cleanup = directories.remove(process.terminate().await).await;
    after_cleanup(drawn, cleanup)
}

/// The document's picture in a page of `browser`, written to the drawings'
/// directory.
async fn picture(browser: &Browser, input: &DrawInput, html: &str) -> Result<DrawResult> {
    let page = browser.new_page("about:blank").await?;
    let metrics = |height: u32| {
        SetDeviceMetricsOverrideParams::new(i64::from(input.width), i64::from(height), SCALE, false)
    };
    page.execute(metrics(input.height.unwrap_or(MEASURE_HEIGHT)))
        .await?;
    page.execute(
        SetEmulatedMediaParams::builder()
            .features(vec![MediaFeature::new("prefers-reduced-motion", "reduce")])
            .build(),
    )
    .await?;
    page.set_content(html).await?;
    let content: f64 = tokio::time::timeout(SETTLE_WITHIN, page.evaluate(SETTLED))
        .await
        .map_err(|_| BrowserError::Timeout)??
        .into_value()
        .map_err(|error| BrowserError::Configuration(error.to_string()))?;
    let height = input
        .height
        .unwrap_or((content as u32).clamp(1, DRAW_MAX_HEIGHT));
    if input.height.is_none() {
        page.execute(metrics(height)).await?;
    }
    let shot = page
        .execute(
            CaptureScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Png)
                .clip(Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: f64::from(input.width),
                    height: f64::from(height),
                    scale: 1.0,
                })
                .capture_beyond_viewport(true)
                .build(),
        )
        .await?;
    let png = base64::engine::general_purpose::STANDARD
        .decode(shot.result.data.as_ref() as &str)
        .map_err(|error| BrowserError::Configuration(format!("Chrome's picture is not base64: {error}")))?;
    let path = keep(png).await?;
    Ok(DrawResult {
        path: path.to_string_lossy().into_owned(),
        width: input.width,
        height,
        content_height: content.ceil() as u32,
    })
}

/// Writes `png` to a new directory of its own in the profiles' base,
/// readable only by its user, after removing that user's pictures older than
/// `DRAWING_KEPT`.
async fn keep(png: Vec<u8>) -> Result<PathBuf> {
    let base = DirectoryBases::host().profiles;
    tokio::task::spawn_blocking(move || {
        remove_old(&base);
        let mut directory = tempfile::Builder::new();
        directory.prefix(DRAWING_PREFIX).rand_bytes(8);
        #[cfg(unix)]
        directory.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
        let directory = directory.tempdir_in(&base)?.keep();
        let path = directory.join("drawing.png");
        std::fs::write(&path, &png)?;
        Ok::<_, std::io::Error>(path)
    })
    .await
    .map_err(|error| BrowserError::Io(std::io::Error::other(error)))?
    .map_err(BrowserError::Io)
}

/// Removes the pictures in `base` older than `DRAWING_KEPT` that its user
/// may remove; one that cannot be removed, such as another user's, stays.
fn remove_old(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let ours = entry.file_name().to_str().is_some_and(|name| name.starts_with(DRAWING_PREFIX));
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > DRAWING_KEPT);
        if ours && old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
