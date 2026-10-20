//! Whether an image's bytes are a whole picture: the one check `demi file view`
//! runs before it shows the model an image and `demi image` runs on a vendor's
//! answer before it writes it (`crates/command-package-file/src/files.rs`,
//! `images.md` § Commands). A picture cut short is refused, never shown or
//! converted as the decoder fills it in.

use std::io::Cursor;

use image::{DynamicImage, ImageError, ImageFormat, ImageReader, Limits};

/// Why bytes are not a whole picture.
#[derive(Debug, thiserror::Error)]
pub enum Undecodable {
    /// Not an image format the decoder reads.
    #[error("not an image this build decodes")]
    Unknown,
    /// Its data ends before its picture does, as a writer cut short leaves it.
    #[error("the image is cut short")]
    CutShort,
    #[error("the image does not decode: {0}")]
    Corrupt(ImageError),
    #[error("the image is too large to decode")]
    TooLarge,
}

/// `bytes` decoded whole as an image of `format`, taking at most
/// `max_alloc` bytes of memory. The JPEG decoder fills a scan whose data
/// ends early with gray rather than failing, so a JPEG is also walked
/// marker by marker to its end-of-image marker.
pub fn decode(bytes: &[u8], format: ImageFormat, max_alloc: u64) -> Result<DynamicImage, Undecodable> {
    if !format.reading_enabled() {
        return Err(Undecodable::Unknown);
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(max_alloc);
    reader.limits(limits);
    let image = reader.decode().map_err(|error| match error {
        ImageError::IoError(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Undecodable::CutShort,
        ImageError::Limits(_) => Undecodable::TooLarge,
        error => Undecodable::Corrupt(error),
    })?;
    if format == ImageFormat::Jpeg && !jpeg_reaches_its_end(bytes) {
        return Err(Undecodable::CutShort);
    }
    Ok(image)
}

/// Whether the JPEG `bytes` reaches its end-of-image marker, walked as the
/// format lays it out: each segment by its length, and a scan's coded data
/// up to the next marker that is not a restart marker or a stuffed 0xFF.
/// Bytes after the end-of-image marker are not read.
fn jpeg_reaches_its_end(bytes: &[u8]) -> bool {
    const START_OF_IMAGE: u8 = 0xD8;
    const END_OF_IMAGE: u8 = 0xD9;
    const START_OF_SCAN: u8 = 0xDA;
    let restart = |marker: u8| (0xD0..=0xD7).contains(&marker);
    if bytes.get(..2) != Some(&[0xFF, START_OF_IMAGE]) {
        return false;
    }
    let mut at = 2;
    loop {
        if bytes.get(at) != Some(&0xFF) {
            return false;
        }
        // Fill bytes: any number of 0xFF before a marker.
        while bytes.get(at) == Some(&0xFF) {
            at += 1;
        }
        let Some(&marker) = bytes.get(at) else {
            return false;
        };
        at += 1;
        if marker == END_OF_IMAGE {
            return true;
        }
        if restart(marker) || marker == 0x01 {
            continue;
        }
        let Some(&[high, low]) = bytes.get(at..at + 2) else {
            return false;
        };
        let length = usize::from(u16::from_be_bytes([high, low]));
        if length < 2 || at + length > bytes.len() {
            return false;
        }
        at += length;
        if marker == START_OF_SCAN {
            // The coded data runs to the next marker of its own.
            loop {
                let Some(offset) = bytes[at..].iter().position(|&byte| byte == 0xFF) else {
                    return false;
                };
                at += offset;
                match bytes.get(at + 1) {
                    None => return false,
                    Some(&next) if next == 0x00 || restart(next) => at += 2,
                    Some(_) => break,
                }
            }
        }
    }
}
