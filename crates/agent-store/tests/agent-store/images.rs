//! Images as they enter a transcript (`runtime.md` § Images in the
//! transcript).

use std::io::Cursor;

use demi_agent_store::{
    images::{Fitted, Unfit, fit},
    testing::png as png_of,
};
use demi_shared_types::B64Bytes;
use image::{
    DynamicImage, ImageBuffer, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage,
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
};

/// The most bytes of a fitted image (`runtime.md` § Images in the
/// transcript).
const MAX_BYTES: usize = 3_750_000;

/// A JPEG of `width` × `height` px.
fn jpeg_of(width: u32, height: u32) -> B64Bytes {
    let image = RgbImage::from_fn(width, height, |x, y| {
        Rgb([(x % 256) as u8, (y % 256) as u8, 7])
    });
    let mut bytes = Vec::new();
    DynamicImage::ImageRgb8(image)
        .write_with_encoder(JpegEncoder::new_with_quality(&mut bytes, 95))
        .unwrap();
    B64Bytes::from(bytes)
}

/// A GIF of `width` × `height` px.
fn gif_of(width: u32, height: u32) -> B64Bytes {
    let image = RgbaImage::from_fn(width, height, |x, _| Rgba([(x % 256) as u8, 0, 0, 255]));
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Gif)
        .unwrap();
    B64Bytes::from(bytes)
}

fn sizes(fitted: &Fitted) -> (&str, (u32, u32), (u32, u32), bool) {
    (
        fitted.media_type,
        fitted.came,
        fitted.entered,
        fitted.reencoded,
    )
}

#[tokio::test]
async fn an_image_enters_as_it_came_within_the_limits_and_fitted_to_2000_px_beyond_them() {
    // Within both limits: the bytes it came with.
    let small = png_of(60, 40, 1);
    let fitted = fit(small.clone(), "image/png").await.unwrap();
    assert_eq!(fitted.data, small);
    assert_eq!(sizes(&fitted), ("image/png", (60, 40), (60, 40), false));
    // A side over 2,000 px: scaled to fit, keeping the aspect ratio; a
    // PNG stays PNG and a JPEG JPEG.
    let wide = fit(png_of(3_000, 30, 2), "image/png").await.unwrap();
    assert_eq!(sizes(&wide), ("image/png", (3_000, 30), (2_000, 20), true));
    let tall = fit(jpeg_of(30, 2_400), "image/jpeg").await.unwrap();
    assert_eq!(sizes(&tall), ("image/jpeg", (30, 2_400), (25, 2_000), true));
    assert!(tall.data.starts_with(b"\xff\xd8\xff"));
    // A GIF becomes its first frame as PNG, whatever its size.
    let gif = fit(gif_of(40, 30), "image/gif").await.unwrap();
    assert_eq!(sizes(&gif), ("image/png", (40, 30), (40, 30), true));
    assert!(gif.data.starts_with(b"\x89PNG"));
    // Fitting again gives the same bytes.
    assert_eq!(fit(png_of(3_000, 30, 2), "image/png").await.unwrap(), wide);
}

#[tokio::test]
async fn an_image_over_the_byte_limit_enters_as_jpeg_and_one_that_does_not_decode_not_at_all() {
    // Noise does not compress: as a PNG of 16 bits a channel, 8 bytes a
    // pixel, it is over 3,750,000 bytes. Half a megapixel keeps the test
    // near a second in an unoptimized build; fewer bytes a pixel would
    // take more pixels.
    let mut state: u32 = 1;
    let mut next = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 16) as u16
    };
    let noise = ImageBuffer::from_fn(700, 700, |_, _| Rgba([next(), next(), next(), 65_535]));
    let mut noisy = Vec::new();
    let stored = PngEncoder::new_with_quality(
        &mut noisy,
        image::codecs::png::CompressionType::Uncompressed,
        image::codecs::png::FilterType::NoFilter,
    );
    DynamicImage::ImageRgba16(noise)
        .write_with_encoder(stored)
        .unwrap();
    let noisy = B64Bytes::from(noisy);
    assert!(noisy.len() > MAX_BYTES);
    let fitted = fit(noisy, "image/png").await.unwrap();
    assert_eq!(sizes(&fitted), ("image/jpeg", (700, 700), (700, 700), true));
    assert!(fitted.data.len() <= MAX_BYTES);
    // Bytes that only start like a PNG decode to nothing.
    let broken = B64Bytes::from(b"\x89PNG\r\n\x1a\n\0\0\0\x01broken".to_vec());
    assert!(matches!(
        fit(broken, "image/png").await,
        Err(Unfit::Undecodable(_))
    ));
}
