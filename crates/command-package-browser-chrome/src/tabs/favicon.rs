//! A tab's icon (`live-view.md` § A browser tab in the panel): what its page
//! names, or its site's `/favicon.ico`, drawn 32 pixels square on the Host
//! each time the main frame stops loading, as a web browser's tab shows the
//! site's icon. Each change counts as a change of what the browser shows, so
//! the tab list carries it.
//!
//! The browser loads the icon for the Host, outside the page's document, as a
//! web browser's own icon request is: an icon the page's document loaded
//! would show among the page's resources, in its asset inventory and its
//! resource timing, though the page never asked for it.

use std::io::Cursor;
use std::time::Duration;

use base64::Engine;
use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::io::{CloseParams, ReadParams, StreamHandle};
use chromiumoxide::cdp::browser_protocol::network::{
    LoadNetworkResourceOptions, LoadNetworkResourceParams,
};
use chromiumoxide::cdp::browser_protocol::page::{
    CreateIsolatedWorldParams, EventFrameStoppedLoading, FrameId, GetFrameTreeParams,
};
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use demi_command_package_browser_protocol::browser::FAVICON_LENGTH;
use futures_util::StreamExt;
use image::{ImageFormat, ImageReader, Limits, RgbaImage, imageops::FilterType};
use resvg::{tiny_skia, usvg};
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::driver::operation::{BrowserError, Result};
use crate::tabs::tab::BrowserTab;

/// How long finding, loading and drawing an icon may take: a site that does
/// not answer for its icon leaves the tab without one.
const DRAW_TIME: Duration = Duration::from_secs(10);

/// The largest icon file the Host draws; a larger one leaves the tab without
/// an icon.
const ICON_BYTES: usize = 1024 * 1024;

/// The largest picture a raster icon may decode to, and the memory decoding
/// it may take.
const ICON_PIXELS: u32 = 2048;
const DECODE_BYTES: u64 = 64 * 1024 * 1024;

/// The drawn icon's width and height in pixels.
const ICON_SIDE: u32 = 32;

/// What the tab's PNG `data:` URL starts with.
const PNG_URL: &str = "data:image/png;base64,";

/// A stream of the browser's that is closed when dropped, however its reading
/// ends.
struct OpenStream {
    page: Page,
    handle: StreamHandle,
    tasks: TaskTracker,
}

impl Drop for OpenStream {
    fn drop(&mut self) {
        let page = self.page.clone();
        let close = CloseParams::new(self.handle.clone());
        self.tasks.spawn(async move {
            // A page that is gone took its streams with it.
            let _ = page.execute(close).await;
        });
    }
}

/// The address of the page's icon in the frame `frame`, as `favicon.js` finds
/// it; none for a page that is not on the web.
async fn locate(page: &Page, frame: &FrameId) -> Result<Option<String>> {
    let world = CreateIsolatedWorldParams::builder()
        .frame_id(frame.clone())
        .world_name("demi-favicon")
        .build()
        .map_err(BrowserError::Configuration)?;
    let context = page.execute(world).await?.result.execution_context_id;
    let evaluate = EvaluateParams::builder()
        .expression(include_str!("favicon.js"))
        .context_id(context)
        .return_by_value(true)
        .build()
        .map_err(BrowserError::Configuration)?;
    let located = page.execute(evaluate).await?.result.result.value;
    Ok(located.and_then(|value| value.as_str().map(str::to_owned)))
}

/// The icon file at `url`, as the browser loads it for the frame `frame`
/// with the site's cookies; none when it does not load, does not succeed or
/// is larger than [`ICON_BYTES`].
async fn load(
    page: &Page,
    frame: &FrameId,
    url: String,
    tasks: &TaskTracker,
) -> Result<Option<Vec<u8>>> {
    let request = LoadNetworkResourceParams::builder()
        .frame_id(frame.clone())
        .url(url)
        .options(LoadNetworkResourceOptions::new(false, true))
        .build()
        .map_err(BrowserError::Configuration)?;
    let loaded = page.execute(request).await?.result.resource;
    let Some(handle) = loaded.stream else {
        return Ok(None);
    };
    let stream = OpenStream {
        page: page.clone(),
        handle,
        tasks: tasks.clone(),
    };
    let succeeded = loaded.success
        && loaded
            .http_status_code
            .is_none_or(|status| (200.0..300.0).contains(&status));
    if !succeeded {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    loop {
        let read = page
            .execute(ReadParams::new(stream.handle.clone()))
            .await?
            .result;
        if read.base64_encoded == Some(true) {
            base64::engine::general_purpose::STANDARD
                .decode_vec(read.data, &mut bytes)
                .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
        } else {
            bytes.extend_from_slice(read.data.as_bytes());
        }
        if bytes.len() > ICON_BYTES {
            return Ok(None);
        }
        if read.eof {
            return Ok(Some(bytes));
        }
    }
}

/// The icon file `bytes` drawn [`ICON_SIDE`] pixels square, as a PNG
/// `data:` URL; none for a file that is neither a picture nor an SVG
/// document, or whose drawing is longer than [`FAVICON_LENGTH`].
fn draw(bytes: &[u8]) -> Option<String> {
    let drawn = raster(bytes).or_else(|| vector(bytes))?;
    let mut png = Vec::new();
    drawn
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .ok()?;
    let url = format!(
        "{PNG_URL}{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    );
    (url.len() <= FAVICON_LENGTH).then_some(url)
}

/// A picture file (an `.ico`, PNG, JPEG, GIF, WebP or BMP) scaled to the
/// icon's size.
fn raster(bytes: &[u8]) -> Option<RgbaImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(ICON_PIXELS);
    limits.max_image_height = Some(ICON_PIXELS);
    limits.max_alloc = Some(DECODE_BYTES);
    reader.limits(limits);
    let picture = reader.decode().ok()?.into_rgba8();
    Some(image::imageops::resize(
        &picture,
        ICON_SIDE,
        ICON_SIDE,
        FilterType::Triangle,
    ))
}

/// An SVG document drawn at the icon's size. It draws nothing from outside
/// the document: no file of the Host's, no font.
fn vector(bytes: &[u8]) -> Option<RgbaImage> {
    let mut options = usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree = usvg::Tree::from_data(bytes, &options).ok()?;
    let size = tree.size();
    let side = ICON_SIDE as f32;
    let mut pixmap = tiny_skia::Pixmap::new(ICON_SIDE, ICON_SIDE)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(side / size.width(), side / size.height()),
        &mut pixmap.as_mut(),
    );
    RgbaImage::from_raw(ICON_SIDE, ICON_SIDE, pixmap.take_demultiplied())
}

/// The icon of the page's frame `frame`; none for a page without one, or
/// whose icon did not load or draw.
async fn find(page: &Page, frame: &FrameId, tasks: &TaskTracker) -> Result<Option<String>> {
    let Some(url) = locate(page, frame).await? else {
        return Ok(None);
    };
    let Some(bytes) = load(page, frame, url, tasks).await? else {
        return Ok(None);
    };
    Ok(tokio::task::spawn_blocking(move || draw(&bytes)).await?)
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
    let streams = tasks.clone();
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
                drawn = tokio::time::timeout(DRAW_TIME, find(&page, &main, &streams)) => drawn,
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
