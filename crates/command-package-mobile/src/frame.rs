//! A device's frame in the live view (`mobile.md` § The live view): the one
//! its platform installs for the model, which the driver draws, or an
//! outline the view draws itself, whose buttons this places on the edges
//! of the device as its picture shows it.

use base64::Engine as _;

use demi_command_package_mobile_protocol::{
    live::{LiveButtonMark, LiveFrame, LiveRect, LiveSide},
    mobile::Orientation,
};

use crate::driver::Frame;

/// The frame as the view takes it, for a device of `model` turned to
/// `orientation`.
pub fn live(frame: Frame, model: &str, orientation: Orientation) -> LiveFrame {
    match frame {
        Frame::Drawn {
            png,
            width,
            height,
            screen,
            corner_radius,
        } => LiveFrame::Image {
            png: base64::engine::general_purpose::STANDARD.encode(&png),
            width,
            height,
            screen: LiveRect {
                x: screen.x.max(0.0),
                y: screen.y.max(0.0),
                width: screen.width.max(1.0),
                height: screen.height.max(1.0),
            },
            corner_radius: corner_radius.clamp(0.0, 4096.0),
        },
        Frame::Undrawn { corner_radius } => outline(corner_radius, model, orientation),
    }
}

/// An outline around a screen whose corners have `corner_radius` points,
/// with the buttons of the model's kind.
pub fn outline(corner_radius: f64, model: &str, orientation: Orientation) -> LiveFrame {
    LiveFrame::Outline {
        corner_radius: corner_radius.clamp(0.0, 4096.0),
        buttons: portrait_buttons(model)
            .into_iter()
            .map(|mark| turned(mark, orientation))
            .collect(),
    }
}

/// Where a model's buttons are, held upright: an iPad's top button and its
/// volume on the right; an iPhone's Action button and volume on the left,
/// its side button on the right, as Xcode's frames place them.
fn portrait_buttons(model: &str) -> Vec<LiveButtonMark> {
    let mark = |side, start, length| LiveButtonMark { side, start, length };
    if model.starts_with("iPad") {
        vec![
            mark(LiveSide::Top, 0.78, 0.08),
            mark(LiveSide::Right, 0.1, 0.05),
            mark(LiveSide::Right, 0.17, 0.05),
        ]
    } else {
        vec![
            mark(LiveSide::Left, 0.18, 0.04),
            mark(LiveSide::Left, 0.24, 0.07),
            mark(LiveSide::Left, 0.33, 0.07),
            mark(LiveSide::Right, 0.29, 0.11),
        ]
    }
}

/// `mark` of the upright device, on the edge it is on once the device is
/// turned to `orientation`, as the picture is turned: the helper's
/// mapping of the picture's points to the screen's (`DeviceSession`).
fn turned(mark: LiveButtonMark, orientation: Orientation) -> LiveButtonMark {
    let reversed = |side| LiveButtonMark {
        side,
        start: (1.0 - mark.start - mark.length).max(0.0),
        length: mark.length,
    };
    let kept = |side| LiveButtonMark { side, ..mark };
    use LiveSide::{Bottom, Left, Right, Top};
    match (orientation, mark.side) {
        (Orientation::Portrait, side) => kept(side),
        (Orientation::LandscapeLeft, Left) => kept(Bottom),
        (Orientation::LandscapeLeft, Right) => kept(Top),
        (Orientation::LandscapeLeft, Top) => reversed(Left),
        (Orientation::LandscapeLeft, Bottom) => reversed(Right),
        (Orientation::LandscapeRight, Left) => reversed(Top),
        (Orientation::LandscapeRight, Right) => reversed(Bottom),
        (Orientation::LandscapeRight, Top) => kept(Right),
        (Orientation::LandscapeRight, Bottom) => kept(Left),
        (Orientation::PortraitUpsideDown, Left) => reversed(Right),
        (Orientation::PortraitUpsideDown, Right) => reversed(Left),
        (Orientation::PortraitUpsideDown, Top) => reversed(Bottom),
        (Orientation::PortraitUpsideDown, Bottom) => reversed(Top),
    }
}
