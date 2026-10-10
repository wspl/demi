//! Images as they enter a transcript (`runtime.md` § Images in the
//! transcript): each is fitted once to what every provider accepts of one
//! image, at most 2,000 px on each side and 5,000,000 bytes as base64, and it
//! never changes afterwards, so every request sends the same bytes. The
//! original stays where it came from.

use std::io::Cursor;

use demi_shared_types::B64Bytes;
use image::{
    DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits,
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
pub struct Fitted {
    pub data: B64Bytes,
    pub media_type: &'static str,
    /// Its size in pixels as it came.
    pub came: (u32, u32),
    /// Its size in pixels as it entered.
    pub entered: (u32, u32),
    /// Whether it entered re-encoded, as other bytes than it came with.
    pub reencoded: bool,
}

/// Why an image cannot enter a transcript.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unfit {
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
pub async fn fit(data: B64Bytes, media_type: &str) -> Result<Fitted, Unfit> {
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
    let (format, mut decoder) = decoder(data, media_type)?;
    let orientation = decoder.orientation()?;
    let came = decoder.dimensions();
    let mut image = DynamicImage::from_decoder(decoder)?;
    let as_it_came =
        format != ImageFormat::Gif && came.0.max(came.1) <= MAX_SIDE && data.len() <= MAX_BYTES;
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

/// A decoder of `data`, an image of `media_type`, that has read its header
/// and decodes the rest within [`MAX_DECODE_BYTES`].
fn decoder<'a>(
    data: &'a [u8],
    media_type: &str,
) -> Result<(ImageFormat, impl ImageDecoder + 'a), Unfit> {
    let format = ImageFormat::from_mime_type(media_type)
        .ok_or_else(|| Unfit::Undecodable(format!("{media_type} is not an image type")))?;
    let mut reader = ImageReader::with_format(Cursor::new(data), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    Ok((format, reader.into_decoder()?))
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
