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

#[cfg(test)]
mod tests {
    use demi_shared_h264::PixelFormat;

    use super::*;

    fn picture(width: u32, height: u32, paint: &[(u32, u32)]) -> Vec<u8> {
        let mut pixels = vec![0; width as usize * height as usize * 4];
        for (x, y) in paint {
            pixels[(*y as usize * width as usize + *x as usize) * 4 + 1] = 255;
        }
        pixels
    }

    fn raw(width: u32, height: u32, pixels: &[u8]) -> Raw<'_> {
        Raw {
            width,
            height,
            format: PixelFormat::Rgba,
            pixels,
        }
    }

    /// The same picture is no change, a one-pixel change is found where it is,
    /// and another size differs everywhere.
    ///
    /// Cost: pictures of a few pixels; microseconds.
    #[test]
    fn a_change_is_found_exactly_where_it_is() {
        let before = picture(8, 6, &[]);
        assert_eq!(changed(&raw(8, 6, &before), &raw(8, 6, &before.clone())), None);
        let after = picture(8, 6, &[(2, 1), (5, 3)]);
        assert_eq!(changed(&raw(8, 6, &after), &raw(8, 6, &before)), Some(Area { x: 2, y: 1, width: 4, height: 3 }));
        let last = picture(8, 6, &[(7, 5)]);
        assert_eq!(changed(&raw(8, 6, &last), &raw(8, 6, &before)), Some(Area { x: 7, y: 5, width: 1, height: 1 }));
        let other = picture(6, 6, &[]);
        assert_eq!(changed(&raw(6, 6, &other), &raw(8, 6, &before)), Some(Area::whole(6, 6)));
    }
}
