//! A tab's viewport (`browser-live-view.md` § Viewport and pixel ratio): who
//! decides it, its size in CSS pixels, and the pixel ratio the page renders at.

use chromiumoxide::cdp::browser_protocol::{
    browser::{Bounds, GetWindowForTargetParams, SetWindowBoundsParams},
    emulation::SetDeviceMetricsOverrideParams,
};
use serde_json::{Value, json};

use super::{BrowserError, BrowserTab, Result, launch::WINDOW_CHROME_HEIGHT};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Mode {
    /// The user's panel decides the size and the viewer's screen the ratio.
    Web,
    /// The agent set the size and the ratio.
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Viewport {
    pub mode: Mode,
    pub width: u32,
    pub height: u32,
    pub ratio: f64,
}

impl Viewport {
    /// A tab nobody has watched.
    pub const UNWATCHED: Self = Self {
        mode: Mode::Web,
        width: 1280,
        height: 720,
        ratio: 1.0,
    };

    /// The result field every command that reports a viewport shares.
    pub fn report(&self) -> Value {
        json!({
            "width": self.width,
            "height": self.height,
            "devicePixelRatio": self.ratio,
            "mode": match self.mode {
                Mode::Web => "web",
                Mode::Custom => "custom",
            },
        })
    }
}

/// A tab's viewport, and the Web viewport it returns to when the agent's
/// setting is reset.
pub(super) struct Viewports {
    pub current: Viewport,
    pub web: Viewport,
}

impl Default for Viewports {
    fn default() -> Self {
        Self {
            current: Viewport::UNWATCHED,
            web: Viewport::UNWATCHED,
        }
    }
}

impl BrowserTab {
    pub(super) fn viewport(&self) -> Viewport {
        self.state
            .viewport
            .lock()
            .expect("viewport lock poisoned")
            .current
    }

    /// The Web viewport the tab last had, which `viewport reset` returns to.
    pub(super) fn web_viewport(&self) -> Viewport {
        self.state
            .viewport
            .lock()
            .expect("viewport lock poisoned")
            .web
    }

    /// Pins the page's viewport and sizes its window to it plus the browser's
    /// own chrome, so the page never sees an outer size smaller than its inner
    /// size.
    pub(super) async fn set_viewport(&self, viewport: Viewport) -> Result<()> {
        self.page
            .execute(SetDeviceMetricsOverrideParams::new(
                i64::from(viewport.width),
                i64::from(viewport.height),
                viewport.ratio,
                false,
            ))
            .await?;
        let browser = self.browser.upgrade().ok_or(BrowserError::Closed)?;
        let browser = browser.lock().await;
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
        drop(browser);
        let mut viewports = self.state.viewport.lock().expect("viewport lock poisoned");
        viewports.current = viewport;
        if viewport.mode == Mode::Web {
            viewports.web = viewport;
        }
        Ok(())
    }
}
