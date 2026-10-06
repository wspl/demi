//! A medium's size in pixels, read from its header (`runtime.md` § Media).
//! The videos are fixtures of a few kilobytes that ffmpeg made from its test
//! pattern: an MP4 of 64 × 36 px with its `moov` box first and one with it
//! last, a QuickTime movie of 40 × 30 px and a WebM of 48 × 64 px.

use bytes::Bytes;
use demi_agent_store::{pixels::pixel_size, testing::png};
use demi_shared_types::PixelSize;
use image::{DynamicImage, ImageEncoder as _, RgbImage, codecs::jpeg::JpegEncoder};

fn size(width: u32, height: u32) -> Option<PixelSize> {
    Some(PixelSize { width, height })
}

/// A JPEG of `width` × `height` px whose EXIF orientation is `orientation`.
fn jpeg_oriented(width: u32, height: u32, orientation: u16) -> Bytes {
    // Big-endian TIFF with one IFD entry: Orientation (0x0112), a SHORT.
    let mut exif = b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
    exif.extend_from_slice(&orientation.to_be_bytes());
    exif.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let mut bytes = Vec::new();
    let mut encoder = JpegEncoder::new(&mut bytes);
    encoder.set_exif_metadata(exif).unwrap();
    let image = DynamicImage::ImageRgb8(RgbImage::new(width, height));
    image.write_with_encoder(encoder).unwrap();
    Bytes::from(bytes)
}

#[test]
fn an_images_size_is_read_from_its_header_as_a_browser_shows_it() {
    let shot = png(60, 40, 1).into_bytes();
    assert_eq!(pixel_size(shot.clone(), "image/png"), size(60, 40));
    // The header alone is enough: a record reads the file's opening.
    assert_eq!(pixel_size(shot.slice(..64), "image/png"), size(60, 40));
    // A photo taken on its side shows turned upright.
    assert_eq!(
        pixel_size(jpeg_oriented(30, 20, 6), "image/jpeg"),
        size(20, 30)
    );
    assert_eq!(
        pixel_size(jpeg_oriented(30, 20, 3), "image/jpeg"),
        size(30, 20)
    );
    // Bytes that only start like an image, and a file that is no medium.
    assert_eq!(pixel_size(shot.slice(..12), "image/png"), None);
    assert_eq!(pixel_size(shot, "application/pdf"), None);
}

#[test]
fn a_videos_size_is_read_from_its_containers_header() {
    let mp4 = Bytes::from_static(include_bytes!("fixtures/clip.mp4"));
    let moov_last = Bytes::from_static(include_bytes!("fixtures/moov-last.mp4"));
    let mov = Bytes::from_static(include_bytes!("fixtures/clip.mov"));
    let webm = Bytes::from_static(include_bytes!("fixtures/clip.webm"));
    assert_eq!(pixel_size(mp4.clone(), "video/mp4"), size(64, 36));
    assert_eq!(pixel_size(moov_last.clone(), "video/mp4"), size(64, 36));
    assert_eq!(pixel_size(mov, "video/quicktime"), size(40, 30));
    assert_eq!(pixel_size(webm, "video/webm"), size(48, 64));
    // The opening of an MP4 holds its header when the `moov` box comes
    // first, and not when it comes after the frames.
    assert_eq!(pixel_size(mp4.slice(..900), "video/mp4"), size(64, 36));
    assert_eq!(pixel_size(moov_last.slice(..900), "video/mp4"), None);
}
