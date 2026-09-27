//! Images as they enter a transcript (`runtime.md` § Images in the
//! transcript): each is fitted once to what every provider accepts of one
//! image, at most 2,000 px on each side and 5,000,000 bytes as base64, and it
//! never changes afterwards, so every request sends the same bytes. The
//! original stays where it came from.

use std::io::Cursor;

use demi_core::B64Bytes;
use image::{
    DynamicImage, ImageDecoder as _, ImageError, ImageFormat, ImageReader, Limits,
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
    imageops::FilterType,
};

/// The longest side of a fitted image, in pixels: the Anthropic API refuses
/// a request of more than 20 images when one side of any exceeds it.
const MAX_SIDE: u32 = 2_000;
/// The most bytes of a fitted image, 5,000,000 as base64: the strictest
/// limit a vendor documents for one image.
const MAX_BYTES: usize = 3_750_000;
/// The most memory decoding one image may take.
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
/// The quality a scaled JPEG keeps.
const JPEG_QUALITY: u8 = 90;
/// The quality of an image that is still over the byte limit.
const SMALLER_JPEG_QUALITY: u8 = 85;

/// An image as it enters a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fitted {
    pub(crate) data: B64Bytes,
    pub(crate) media_type: &'static str,
    /// Its size in pixels as it came.
    pub(crate) came: (u32, u32),
    /// Its size in pixels as it entered.
    pub(crate) entered: (u32, u32),
    /// Whether it entered re-encoded, as other bytes than it came with.
    pub(crate) reencoded: bool,
}

/// Why an image cannot enter a transcript.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Unfit {
    #[error("it could not be decoded ({0})")]
    Undecodable(String),
    #[error("decoding it would take more than 256 MiB")]
    TooLargeToDecode,
    #[error("even as a JPEG of quality 85 it is over 3,750,000 bytes")]
    TooLarge,
}

impl From<ImageError> for Unfit {
    fn from(error: ImageError) -> Self {
        match error {
            ImageError::Limits(_) => Self::TooLargeToDecode,
            error => Self::Undecodable(error.to_string()),
        }
    }
}

/// Fits `data`, an image of `media_type`, on the blocking pool: decoding and
/// encoding would hold the shard's thread for as long as they take.
pub(crate) async fn fit(data: B64Bytes, media_type: &str) -> Result<Fitted, Unfit> {
    let media_type = media_type.to_owned();
    match tokio::task::spawn_blocking(move || fit_now(&data, &media_type)).await {
        Ok(fitted) => fitted,
        // A decoder that panicked on its input decoded nothing.
        Err(failed) => Err(Unfit::Undecodable(failed.to_string())),
    }
}

/// Fits `data` as the table of `runtime.md` § Images in the transcript
/// says: a PNG, JPEG or WebP within both limits enters as it came, once it
/// decodes. Any other image is turned upright; one over 2,000 px on a side
/// is scaled to fit 2,000 × 2,000 px and, like a GIF, encoded again, a JPEG
/// as JPEG and anything else as PNG. An image still over the byte limit
/// enters as a JPEG of a lower quality, or not at all.
fn fit_now(data: &B64Bytes, media_type: &str) -> Result<Fitted, Unfit> {
    let format = ImageFormat::from_mime_type(media_type)
        .ok_or_else(|| Unfit::Undecodable(format!("{media_type} is not an image type")))?;
    let mut reader = ImageReader::with_format(Cursor::new(data.as_bytes()), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let came = decoder.dimensions();
    let mut image = DynamicImage::from_decoder(decoder)?;
    let as_it_came = format != ImageFormat::Gif
        && came.0.max(came.1) <= MAX_SIDE
        && data.len() <= MAX_BYTES;
    if as_it_came {
        return Ok(Fitted {
            data: data.clone(),
            media_type: format.to_mime_type(),
            came,
            entered: came,
            reencoded: false,
        });
    }
    // The encoded image carries no EXIF data, so its orientation is applied.
    image.apply_orientation(orientation);
    let scaled = image.width().max(image.height()) > MAX_SIDE;
    if scaled {
        image = image.resize(MAX_SIDE, MAX_SIDE, FilterType::Lanczos3);
    }
    let entered = (image.width(), image.height());
    let fitted = |bytes: Vec<u8>, format: ImageFormat| Fitted {
        data: B64Bytes::from(bytes),
        media_type: format.to_mime_type(),
        came,
        entered,
        reencoded: true,
    };
    if scaled || format == ImageFormat::Gif {
        let (bytes, format) = if format == ImageFormat::Jpeg {
            (jpeg(&image, JPEG_QUALITY)?, ImageFormat::Jpeg)
        } else {
            (png(&image)?, ImageFormat::Png)
        };
        if bytes.len() <= MAX_BYTES {
            return Ok(fitted(bytes, format));
        }
    }
    let bytes = jpeg(&image, SMALLER_JPEG_QUALITY)?;
    if bytes.len() > MAX_BYTES {
        return Err(Unfit::TooLarge);
    }
    Ok(fitted(bytes, ImageFormat::Jpeg))
}

/// `image` as a JPEG of `quality`, without an alpha channel, which JPEG has
/// no place for.
fn jpeg(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, Unfit> {
    let mut bytes = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut bytes, quality);
    DynamicImage::ImageRgb8(image.to_rgb8()).write_with_encoder(encoder)?;
    Ok(bytes)
}

/// `image` as a PNG.
fn png(image: &DynamicImage) -> Result<Vec<u8>, Unfit> {
    let mut bytes = Vec::new();
    image.write_with_encoder(PngEncoder::new(&mut bytes))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use image::{ImageBuffer, Rgb, RgbImage, Rgba, RgbaImage};

    use super::*;
    use crate::testing::png as png_of;

    /// A JPEG of `width` × `height` px.
    fn jpeg_of(width: u32, height: u32) -> B64Bytes {
        let image = RgbImage::from_fn(width, height, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 7]));
        B64Bytes::from(jpeg(&DynamicImage::ImageRgb8(image), 95).unwrap())
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
        (fitted.media_type, fitted.came, fitted.entered, fitted.reencoded)
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
        DynamicImage::ImageRgba16(noise).write_with_encoder(stored).unwrap();
        let noisy = B64Bytes::from(noisy);
        assert!(noisy.len() > MAX_BYTES);
        let fitted = fit(noisy, "image/png").await.unwrap();
        assert_eq!(sizes(&fitted), ("image/jpeg", (700, 700), (700, 700), true));
        assert!(fitted.data.len() <= MAX_BYTES);
        // Bytes that only start like a PNG decode to nothing.
        let broken = B64Bytes::from(b"\x89PNG\r\n\x1a\n\0\0\0\x01broken".to_vec());
        assert!(matches!(fit(broken, "image/png").await, Err(Unfit::Undecodable(_))));
    }
}
