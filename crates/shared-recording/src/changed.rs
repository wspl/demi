//! Where one picture differs from another, exactly: the test a source makes
//! before it codes a picture, so that a screen that drew the same pixels again
//! costs nothing.

use demi_shared_h264::Raw;
use demi_shared_recording_protocol::Area;

/// The smallest area holding every pixel of `now` that differs from `before`;
/// none when the two are the same picture. A picture of another size or order
/// differs everywhere.
pub fn changed(now: &Raw<'_>, before: &Raw<'_>) -> Option<Area> {
    if (now.width, now.height, now.format) != (before.width, before.height, before.format) {
        return Some(Area::whole(now.width, now.height));
    }
    if now.pixels == before.pixels {
        return None;
    }
    let bytes = now.format.bytes();
    let length = now.width as usize * bytes;
    let differs = |y: u32| now.pixels[y as usize * length..][..length] != before.pixels[y as usize * length..][..length];
    let top = (0..now.height).find(|y| differs(*y))?;
    let bottom = (top..now.height).rev().find(|y| differs(*y)).expect("the top row differs");
    let (mut left, mut right) = (usize::MAX, 0usize);
    for y in top..=bottom {
        let (a, b) = (&now.pixels[y as usize * length..][..length], &before.pixels[y as usize * length..][..length]);
        if a == b {
            continue;
        }
        let first = a.iter().zip(b).position(|(a, b)| a != b).expect("the row differs") / bytes;
        let last = a.iter().zip(b).rposition(|(a, b)| a != b).expect("the row differs") / bytes;
        left = left.min(first);
        right = right.max(last);
    }
    Some(Area {
        x: left as u32,
        y: top,
        width: (right - left + 1) as u32,
        height: bottom - top + 1,
    })
}
