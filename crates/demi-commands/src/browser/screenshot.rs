//! Browser captures use CDP clipping without changing the page's viewport.
//! Every capture is in CSS pixels, whatever ratio the page renders at
//! (`browser.md` § Screenshot and probe).
use super::{BrowserError, BrowserTab, Result};
use base64::Engine;
use chromiumoxide::cdp::browser_protocol::page::{
    CaptureScreenshotFormat, CaptureScreenshotParams, GetLayoutMetricsParams, Viewport,
};

impl BrowserTab {
    pub(super) async fn screenshot_bytes(
        &self,
        full_page: bool,
        clip: Option<&str>,
    ) -> Result<Vec<u8>> {
        if full_page && clip.is_some() {
            return Err(BrowserError::Configuration(
                "--full-page and --clip are mutually exclusive".into(),
            ));
        }
        let viewport = self.viewport();
        let scale = 1.0 / viewport.ratio;
        // An explicit rectangle or the whole page may lie beyond the viewport.
        let beyond_viewport = full_page || clip.is_some();
        let clip = if let Some(clip) = clip {
            let values = clip
                .split(',')
                .map(str::parse::<f64>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| {
                    BrowserError::Configuration("clip requires x,y,width,height".into())
                })?;
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
            let metrics = self.page.execute(GetLayoutMetricsParams {}).await?.result;
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
        request.clip = Some(clip);
        let data = self.page.execute(request).await?.result.data;
        base64::engine::general_purpose::STANDARD
            .decode(AsRef::<[u8]>::as_ref(&data))
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))
    }
}
