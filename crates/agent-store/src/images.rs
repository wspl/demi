//! Images as they enter a transcript (`transcript.md` § Images in the
//! transcript): each is fitted once to what every provider accepts of one
//! image, at most 2,000 px on each side and 5,000,000 bytes as base64, and
//! kept light, since every request replays it. It never changes afterwards,
//! so every request sends the same bytes. The original stays where it came
//! from.

use std::io::Cursor;

use demi_shared_types::{B64Bytes, PixelSize};
use image::{
    DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits, Rgba, RgbaImage,
    metadata::Orientation,
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
    imageops::{self, FilterType},
};

/// The longest side of a fitted image, in pixels: the Anthropic API refuses
/// a request of more than 20 images when one side of any exceeds it.
const MAX_SIDE: u32 = 2_000;
/// The most bytes of a fitted image, 5,000,000 as base64: the strictest
/// limit a vendor documents for one image.
const MAX_BYTES: usize = 3_750_000;
/// The most bytes an image enters with as it is: every request replays it,
/// so one heavier enters as a JPEG when that is smaller.
const LIGHT_BYTES: usize = 500_000;
/// The most memory decoding one image may take.
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
/// The quality a scaled JPEG keeps.
const JPEG_QUALITY: u8 = 90;
/// The quality of an image that is heavier than [`LIGHT_BYTES`].
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

/// Fits `data` as the table of `transcript.md` § Images in the transcript
/// says: a PNG, JPEG or WebP within 2,000 px on each side and 500,000 bytes
/// enters as it came, once it decodes. Any other image is turned upright;
/// one over 2,000 px on a side is scaled to fit 2,000 × 2,000 px and, like a
/// GIF, encoded again, a JPEG as JPEG and anything else as PNG. One still
/// over 500,000 bytes enters as a JPEG of a lower quality, its transparency
/// flattened onto white, when that is smaller; one over 3,750,000 bytes
/// after that does not enter.
fn fit_now(data: &B64Bytes, media_type: &str) -> Result<Fitted, Unfit> {
    let (format, mut decoder) = decoder(data, media_type)?;
    let orientation = decoder.orientation()?;
    let came = decoder.dimensions();
    let mut image = DynamicImage::from_decoder(decoder)?;
    let as_it_came =
        format != ImageFormat::Gif && came.0.max(came.1) <= MAX_SIDE && data.len() <= LIGHT_BYTES;
    if as_it_came {
        return Ok(Fitted {
            data: data.clone(),
            media_type: format.to_mime_type(),
            came,
            entered: came,
            reencoded: false,
        });
    }
    // An encoded image carries no EXIF data, so its orientation is applied.
    image.apply_orientation(orientation);
    let scaled = image.width().max(image.height()) > MAX_SIDE;
    if scaled {
        image = image.resize(MAX_SIDE, MAX_SIDE, FilterType::Lanczos3);
    }
    // The image before the byte rule: encoded again when it was scaled or
    // is a GIF, otherwise as it came.
    let mut fitted = if scaled || format == ImageFormat::Gif {
        let (bytes, format) = if format == ImageFormat::Jpeg {
            (jpeg(&image, JPEG_QUALITY)?, ImageFormat::Jpeg)
        } else {
            (png(&image)?, ImageFormat::Png)
        };
        Fitted {
            data: B64Bytes::from(bytes),
            media_type: format.to_mime_type(),
            came,
            entered: (image.width(), image.height()),
            reencoded: true,
        }
    } else {
        Fitted {
            data: data.clone(),
            media_type: format.to_mime_type(),
            came,
            entered: came,
            reencoded: false,
        }
    };
    if fitted.data.len() > LIGHT_BYTES {
        let lighter = jpeg(&image, SMALLER_JPEG_QUALITY)?;
        if lighter.len() < fitted.data.len() {
            fitted = Fitted {
                data: B64Bytes::from(lighter),
                media_type: ImageFormat::Jpeg.to_mime_type(),
                came,
                entered: (image.width(), image.height()),
                reencoded: true,
            };
        }
    }
    if fitted.data.len() > MAX_BYTES {
        return Err(Unfit::TooLarge);
    }
    Ok(fitted)
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

/// The size `data`, an image of `media_type`, shows at, as a web browser
/// shows it: its pixels turned upright as its orientation says. Only its
/// header is read; none when that does not decode.
pub(crate) fn shown_size(data: &[u8], media_type: &str) -> Option<PixelSize> {
    let (_, mut decoder) = decoder(data, media_type).ok()?;
    let (width, height) = decoder.dimensions();
    let turned = matches!(
        decoder.orientation().ok()?,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    );
    Some(if turned {
        PixelSize {
            width: height,
            height: width,
        }
    } else {
        PixelSize { width, height }
    })
}

/// `image` as a JPEG of `quality`, its transparency flattened onto white,
/// since JPEG has no alpha channel: a transparent pixel would otherwise
/// show the color it hides, often black.
fn jpeg(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, Unfit> {
    let opaque = if image.color().has_alpha() {
        let mut white = RgbaImage::from_pixel(image.width(), image.height(), Rgba([255; 4]));
        imageops::overlay(&mut white, &image.to_rgba8(), 0, 0);
        DynamicImage::ImageRgba8(white).to_rgb8()
    } else {
        image.to_rgb8()
    };
    let mut bytes = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut bytes, quality);
    DynamicImage::ImageRgb8(opaque).write_with_encoder(encoder)?;
    Ok(bytes)
}

/// `image` as a PNG.
fn png(image: &DynamicImage) -> Result<Vec<u8>, Unfit> {
    let mut bytes = Vec::new();
    image.write_with_encoder(PngEncoder::new(&mut bytes))?;
    Ok(bytes)
}
