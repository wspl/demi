//! A tab's viewport (`live-view.md` § Viewport and pixel ratio): who
//! decides it, its size in CSS pixels, and the pixel ratio the page renders at.

use chromiumoxide::{
    Page,
    cdp::browser_protocol::{
        browser::{Bounds, GetWindowForTargetParams, SetWindowBoundsParams},
        emulation::{
            GetScreenInfosParams, ScreenId, SetDeviceMetricsOverrideParams,
            SetTouchEmulationEnabledParams, SetUserAgentOverrideParams, UserAgentMetadata,
        },
        page::{CaptureScreenshotFormat, CaptureScreenshotParams},
    },
    types::{Command, Method, MethodId},
};
use serde_json::Value;

use super::{
    BrowserError, BrowserTab, Result,
    launch::WINDOW_CHROME_HEIGHT,
    protocol::{BrowserViewport, ViewportMode},
};

/// Flush this page's layout to Chrome's painted view without resizing it.
pub(super) async fn paint(page: &Page) -> Result<()> {
    // View snapshots force a redraw without the surface screenshot's temporary
    // device-emulation changes. The low-quality image is only a paint barrier.
    let result = page
        .execute(
            CaptureScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Jpeg)
                .quality(1)
                .from_surface(false)
                .capture_beyond_viewport(false)
                .build(),
        )
        .await;
    match result {
        Ok(_) => Ok(()),
        // Chrome reports an empty native-view snapshot for a background macOS
        // tab only after its ForceRedraw callback. Painting is complete; this
        // barrier does not need the image. Earlier protocol failures still fail.
        Err(chromiumoxide::error::CdpError::Chrome(error))
            if error.code == -32000 && error.message == "Unable to capture screenshot" =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// A tab nobody has watched.
pub(super) const UNWATCHED: BrowserViewport = BrowserViewport {
    width: 1280,
    height: 720,
    device_pixel_ratio: 1.0,
    mode: ViewportMode::Web,
};
/// Mobile mode's size in CSS pixels.
pub(super) const PHONE: (u32, u32) = (390, 844);

/// A viewport's picture size in device pixels at `scale`, even for the encoder.
pub(super) fn pixels(viewport: &BrowserViewport, scale: f64) -> (u32, u32) {
    let even = |length: u32| {
        ((f64::from(length) * viewport.device_pixel_ratio * scale / 2.0).ceil() as u32) * 2
    };
    (even(viewport.width), even(viewport.height))
}

/// A tab's viewport, and the Web viewport it returns to when the agent's
/// setting is reset.
#[derive(Debug, Clone, Copy)]
pub(super) struct Viewports {
    pub current: BrowserViewport,
    pub web: BrowserViewport,
}

impl Default for Viewports {
    fn default() -> Self {
        Self {
            current: UNWATCHED,
            web: UNWATCHED,
        }
    }
}

/// The largest picture side the capture encodes.
const MAX_SIDE: f64 = 4096.0;
/// The largest picture H.264 High level 5.1 encodes: 36,864 macroblocks.
const MAX_PIXELS: f64 = 9_437_184.0;

/// The screen's pixel ratio for a viewer at `device_ratio`. macOS accepts
/// only whole ratios: a fractional one rounds up and the page scales the
/// picture down.
pub(super) fn screen_ratio(device_ratio: f64) -> f64 {
    let ratio = device_ratio.max(1.0);
    if cfg!(target_os = "macos") {
        ratio.ceil()
    } else {
        ratio
    }
}

/// The pixel ratio a page of `width` × `height` CSS pixels renders at for a
/// viewer at `device_ratio`, within what the capture encodes.
pub(super) fn ratio_for(device_ratio: f64, width: u32, height: u32) -> f64 {
    let (width, height) = (f64::from(width), f64::from(height));
    let limit = (MAX_SIDE / width.max(height)).min((MAX_PIXELS / (width * height)).sqrt());
    let limit = if cfg!(target_os = "macos") {
        limit.floor()
    } else {
        limit
    };
    screen_ratio(device_ratio).min(limit).max(1.0)
}

/// The virtual screen every window of the browser shares: the size of the
/// viewer's screen in CSS pixels, and its pixel ratio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Screen {
    pub width: u32,
    pub height: u32,
    pub ratio: f64,
}

/// `Emulation.updateScreen`, newer than the vendored protocol.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateScreenParams {
    screen_id: ScreenId,
    width: i64,
    height: i64,
    device_pixel_ratio: f64,
}

impl Method for UpdateScreenParams {
    fn identifier(&self) -> MethodId {
        "Emulation.updateScreen".into()
    }
}

impl Command for UpdateScreenParams {
    type Response = Value;
}

/// A phone's user agent: Android Chrome at the pinned version, with client
/// hints that keep the browser's own brands.
fn phone(version: &str) -> SetUserAgentOverrideParams {
    let major = version.split('.').next().unwrap_or(version);
    let mut agent = SetUserAgentOverrideParams::new(format!(
        "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Mobile Safari/537.36"
    ));
    agent.platform = Some("Linux armv8l".into());
    agent.user_agent_metadata = Some(UserAgentMetadata {
        brands: None,
        full_version_list: None,
        platform: "Android".into(),
        platform_version: "14.0.0".into(),
        architecture: String::new(),
        model: "Pixel 7".into(),
        mobile: true,
        bitness: None,
        wow64: None,
        form_factors: None,
    });
    agent
}

impl BrowserTab {
    pub(super) fn viewport(&self) -> BrowserViewport {
        self.state.viewport.borrow().current
    }

    /// The Web viewport the tab last had, which `viewport reset` returns to.
    pub(super) fn web_viewport(&self) -> BrowserViewport {
        self.state.viewport.borrow().web
    }

    /// Pins the page's viewport and sizes its window to it plus the browser's
    /// own chrome, so the page never sees an outer size smaller than its inner
    /// size. Entering or leaving Mobile mode turns the phone's touch and user
    /// agent on or off.
    pub(super) async fn set_viewport(&self, viewport: BrowserViewport) -> Result<()> {
        let mobile = viewport.mode == ViewportMode::Mobile;
        if mobile != (self.viewport().mode == ViewportMode::Mobile) {
            let agent = if mobile {
                phone(&super::installation::pinned_version()?)
            } else {
                // An empty user agent ends the override.
                SetUserAgentOverrideParams::new("")
            };
            self.page.execute(agent).await?;
            let mut touch = SetTouchEmulationEnabledParams::new(mobile);
            touch.max_touch_points = mobile.then_some(5);
            self.page.execute(touch).await?;
        }
        // Resizing the native window after the emulated viewport can leave
        // macOS capture letterboxed at the old size, with mismatched input.
        self.fit_window(viewport).await?;
        self.page
            .execute(SetDeviceMetricsOverrideParams::new(
                i64::from(viewport.width),
                i64::from(viewport.height),
                viewport.device_pixel_ratio,
                mobile,
            ))
            .await?;
        // Metrics acknowledgement can precede the compositor's resized surface.
        // Publish only after it has painted, so capture cannot scale the old one.
        paint(&self.page).await?;
        self.state.viewport.send_modify(|viewports| {
            viewports.current = viewport;
            if viewport.mode == ViewportMode::Web {
                viewports.web = viewport;
            }
        });
        self.state.changes.send_modify(|revision| *revision += 1);
        Ok(())
    }

    async fn fit_window(&self, viewport: BrowserViewport) -> Result<()> {
        let browser = self.browser.call()?;
        let window = browser
            .execute(
                GetWindowForTargetParams::builder()
                    .target_id(self.page.target_id().clone())
                    .build(),
            )
            .await?
            .result
            .window_id;
        browser
            .execute(SetWindowBoundsParams::new(
                window,
                Bounds::builder()
                    .width(i64::from(viewport.width))
                    .height(i64::from(viewport.height + WINDOW_CHROME_HEIGHT))
                    .build(),
            ))
            .await?;
        Ok(())
    }

    /// Sets the virtual screen all windows share; windows keep their tabs'
    /// viewports.
    pub(super) async fn update_screen(&self, screen: Screen) -> Result<()> {
        let screens = self
            .page
            .execute(GetScreenInfosParams::default())
            .await?
            .result
            .screen_infos;
        let first = screens
            .into_iter()
            .next()
            .ok_or_else(|| BrowserError::InvalidResult("the browser has no screen".into()))?;
        self.page
            .execute(UpdateScreenParams {
                screen_id: first.id,
                width: (f64::from(screen.width) * screen.ratio).round() as i64,
                height: (f64::from(screen.height) * screen.ratio).round() as i64,
                device_pixel_ratio: screen.ratio,
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ratio_stays_within_what_the_capture_encodes() {
        let whole = if cfg!(target_os = "macos") { 2.0 } else { 1.5 };
        assert_eq!(ratio_for(1.5, 1280, 720), whole);
        assert_eq!(ratio_for(2.0, 1440, 900), 2.0);
        assert_eq!(ratio_for(0.8, 1440, 900), 1.0);
        // 1800 × 1000 at 3 would be 5400 pixels wide.
        let limited = ratio_for(3.0, 1800, 1000);
        assert!(limited * 1800.0 <= 4096.0 && limited >= 2.0, "{limited}");
        // A phone at ratio 3 fits whole.
        assert_eq!(ratio_for(3.0, 390, 844), 3.0);
    }

    #[test]
    fn a_picture_has_even_device_pixels() {
        let viewport = BrowserViewport {
            mode: ViewportMode::Web,
            width: 701,
            height: 401,
            device_pixel_ratio: 1.5,
        };
        assert_eq!(pixels(&viewport, 1.0), (1052, 602));
        assert_eq!(pixels(&viewport, 0.5), (526, 302));
    }
}
