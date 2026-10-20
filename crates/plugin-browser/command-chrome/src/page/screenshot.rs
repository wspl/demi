//! Browser captures use CDP clipping without changing the page's viewport.
//! Every capture is in CSS pixels, whatever ratio the page renders at, so the
//! image the model receives has the same size whoever watches the tab.
//!
//! Chrome captures at the page's own device pixels and the capture is scaled
//! down here. Chrome's own scaling (`clip.scale`) is not used: while the live
//! view's tab capture runs, Chrome 153 renders the scaled shot at full size and
//! then crops it to the scaled size (`page_handler.cc`, the bitmap that does
//! not match the requested size), so a watched tab at ratio 2 answered only its
//! top-left quarter.
//!
//! Chrome sends the PNG in one base64 CDP message, a third larger than the PNG,
//! so a capture over the CDP message limit fails with `result_too_large` and
//! says to take a part with `--clip`; the browser and tab go on. What the model
//! views is separately limited to 16 MiB.

use std::io::Cursor;
use std::time::Duration;

use base64::Engine;
use chromiumoxide::cdp::browser_protocol::page::{
    CaptureScreenshotFormat, CaptureScreenshotParams, GetLayoutMetricsParams, Viewport,
};
use tokio_util::sync::CancellationToken;

use crate::driver::operation::{BrowserError, Result};
use crate::tabs::tab::BrowserTab;

pub async fn bytes(tab: &BrowserTab, full_page: bool, clip: Option<&str>) -> Result<Vec<u8>> {
    if full_page && clip.is_some() {
        return Err(BrowserError::Configuration(
            "--full-page and --clip are mutually exclusive".into(),
        ));
    }
    let viewport = tab.viewport();
    // Chrome's `clip.scale`, which stays 1 (see the module's documentation).
    let scale = 1.0;
    // An explicit rectangle or the whole page may lie beyond the viewport.
    let beyond_viewport = full_page || clip.is_some();
    let clip = if let Some(clip) = clip {
        let values = clip
            .split(',')
            .map(str::parse::<f64>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| BrowserError::Configuration("clip requires x,y,width,height".into()))?;
        let [x, y, width, height] = values.as_slice() else {
            return Err(BrowserError::Configuration(
                "clip requires x,y,width,height".into(),
            ));
        };
        if values.iter().any(|value| !value.is_finite())
            || *x < 0.0
            || *y < 0.0
            || *width <= 0.0
            || *height <= 0.0
        {
            return Err(BrowserError::Configuration(
                "clip requires finite nonnegative coordinates and positive dimensions".into(),
            ));
        }
        Viewport {
            x: *x,
            y: *y,
            width: *width,
            height: *height,
            scale,
        }
    } else {
        let metrics = tab.page().execute(GetLayoutMetricsParams {}).await?.result;
        if full_page {
            let size = metrics.css_content_size;
            Viewport {
                x: size.x,
                y: size.y,
                width: size.width,
                height: size.height,
                scale,
            }
        } else {
            // What the page shows now, where it is scrolled to.
            let visible = metrics.css_visual_viewport;
            Viewport {
                x: visible.page_x,
                y: visible.page_y,
                width: f64::from(viewport.width),
                height: f64::from(viewport.height),
                scale,
            }
        }
    };
    let mut request = CaptureScreenshotParams::builder()
        .format(CaptureScreenshotFormat::Png)
        .from_surface(true)
        .capture_beyond_viewport(beyond_viewport)
        .build();
    let size = css_size(&clip);
    request.clip = Some(clip);
    let data = tab
        .page()
        .execute(request)
        .await
        .map_err(|error| match BrowserError::from(error) {
            BrowserError::MessageTooLarge(reason) => BrowserError::CaptureTooLarge(reason),
            error => error,
        })?
        .result
        .data;
    let png = base64::engine::general_purpose::STANDARD
        .decode(AsRef::<[u8]>::as_ref(&data))
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    if viewport.device_pixel_ratio == 1.0 {
        return Ok(png);
    }
    tokio::task::spawn_blocking(move || to_css_pixels(&png, size))
        .await
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
}

/// The size of `clip` in whole CSS pixels, one at least each way.
fn css_size(clip: &Viewport) -> (u32, u32) {
    let whole = |length: f64| (length.round() as u32).max(1);
    (whole(clip.width), whole(clip.height))
}

/// The PNG `png`, captured at device pixels, scaled to `size` CSS pixels.
///
/// - At a whole ratio, such as 2 or 3, each CSS pixel is the average of the
///   device pixels it covers, as sharp as the page drawn at ratio 1 and four
///   times as fast as a Catmull-Rom filter.
/// - At any other ratio, such as 1.25, that average would weigh a glyph's
///   columns unevenly, so Catmull-Rom scales it. It keeps text's edges sharp
///   where a triangle filter blurs them.
fn to_css_pixels(png: &[u8], (width, height): (u32, u32)) -> Result<Vec<u8>> {
    let captured = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
        .into_rgba8();
    let (captured_width, captured_height) = captured.dimensions();
    if (captured_width, captured_height) == (width, height) {
        return Ok(png.to_vec());
    }
    let whole = captured_width % width == 0
        && captured_height % height == 0
        && captured_width / width == captured_height / height;
    let scaled = if whole {
        image::imageops::thumbnail(&captured, width, height)
    } else {
        image::imageops::resize(
            &captured,
            width,
            height,
            image::imageops::FilterType::CatmullRom,
        )
    };
    let mut encoded = Vec::new();
    scaled
        .write_to(&mut Cursor::new(&mut encoded), image::ImageFormat::Png)
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    Ok(encoded)
}

/// A PNG of what the tab shows, taken holding its operation lock.
pub async fn screenshot(
    tab: &BrowserTab,
    cancellation: &CancellationToken,
    timeout: Duration,
) -> Result<Vec<u8>> {
    capture(tab, false, None, cancellation, timeout).await
}

/// A PNG of what the tab shows, of its whole page or of `clip`, taken
/// holding its operation lock.
pub async fn capture(
    tab: &BrowserTab,
    full_page: bool,
    clip: Option<&str>,
    cancellation: &CancellationToken,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let _session = tab.state().gate.try_checkout().ok_or(BrowserError::Busy)?;
    tab.operation(cancellation, tokio::time::Instant::now() + timeout)
        .run(bytes(tab, full_page, clip))
        .await
}
