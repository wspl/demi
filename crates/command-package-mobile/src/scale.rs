//! Screenshot pixels and the screen's points
//! (`crates/command-package-mobile/src/operations.rs`): the
//! screenshot is the screen's pixels scaled so its long side is at most
//! 1568, at the size its driver says ([`Screen::screenshot`]), and every
//! coordinate a command takes or prints is one of its pixels, so the agent
//! taps what it sees.

use demi_command_package_mobile_protocol::mobile::{Frame, Point};

use crate::driver::{Points, Rect, Screen};

/// How a screenshot of `screen` maps to its points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    /// The screenshot's size in pixels.
    pub width: u32,
    pub height: u32,
    /// Screenshot pixels per point, across and down.
    across: f64,
    down: f64,
}

/// The size of a picture of `pixels` scaled so its long side is at most
/// `long_side`, never enlarged, each side rounded, as the iOS helper draws
/// its screenshots.
pub fn fitted((pixels_wide, pixels_high): (u32, u32), long_side: u32) -> (u32, u32) {
    let long = pixels_wide.max(pixels_high).max(1);
    let factor = (f64::from(long_side) / f64::from(long)).min(1.0);
    let side = |pixels: u32| ((f64::from(pixels) * factor).round() as u32).max(1);
    (side(pixels_wide), side(pixels_high))
}

impl Scale {
    pub fn of(screen: &Screen) -> Self {
        let (width, height) = screen.screenshot;
        Self {
            width,
            height,
            across: f64::from(width) / screen.points.x.max(1.0),
            down: f64::from(height) / screen.points.y.max(1.0),
        }
    }

    pub fn point(&self, at: Points) -> Point {
        Point {
            x: (at.x * self.across).round() as i32,
            y: (at.y * self.down).round() as i32,
        }
    }

    pub fn frame(&self, rect: &Rect) -> Frame {
        let left = (rect.x * self.across).round();
        let top = (rect.y * self.down).round();
        let right = ((rect.x + rect.width) * self.across).round();
        let bottom = ((rect.y + rect.height) * self.down).round();
        Frame {
            x: left as i32,
            y: top as i32,
            width: (right - left).max(0.0) as u32,
            height: (bottom - top).max(0.0) as u32,
        }
    }

    /// The point under screenshot pixel `x`, `y`; none outside the
    /// screenshot.
    pub fn points(&self, x: f64, y: f64) -> Option<Points> {
        let inside = |value: f64, length: u32| value >= 0.0 && value <= f64::from(length);
        (inside(x, self.width) && inside(y, self.height)).then(|| Points {
            x: x / self.across,
            y: y / self.down,
        })
    }
}
