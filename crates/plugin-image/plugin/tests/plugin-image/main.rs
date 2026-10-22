//! The image plugin through the JSON loopback over an in-memory Demi, which
//! answers its file reads and its images.
//!
//! The tests cover the image checked before a byte reaches stdout and the
//! format it is written in. The whole path through the backend, with a real
//! device, is the backend's `images` scenarios'.

mod support;

use std::io::Cursor;

use demi_plugin_interface::MadeImage;
use image::{GenericImageView as _, ImageFormat, Rgb, RgbImage};

use crate::support::{Plugged, local};

fn made(bytes: Vec<u8>) -> MadeImage {
    MadeImage {
        bytes: bytes.into(),
        model: "gpt-image-2".into(),
        provider: "ChatGPT".into(),
        vendor: "ChatGPT".into(),
        account: Some("alex@example.com".into()),
        // The ChatGPT backend's answer names no model.
        confirmed: false,
    }
}

/// `bytes` as Grok answers them, which confirms the model.
fn from_grok(bytes: Vec<u8>) -> MadeImage {
    MadeImage {
        bytes: bytes.into(),
        model: "grok-imagine-image".into(),
        provider: "Grok".into(),
        vendor: "Grok".into(),
        account: Some("alex@example.com".into()),
        confirmed: true,
    }
}

/// A JPEG of `width`×`height` px, as Grok answers one.
fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    RgbImage::from_pixel(width, height, Rgb([200, 30, 30]))
        .write_to(&mut bytes, ImageFormat::Jpeg)
        .unwrap();
    bytes.into_inner()
}

/// A PNG of `width`×`height` px with an alpha channel, each pixel's alpha
/// `alpha`.
fn rgba_png(width: u32, height: u32, alpha: u8) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(width, height, image::Rgba([200, 30, 30, alpha]))
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

/// The image is decoded whole before a byte reaches stdout: an answer that
/// is no image, or only a PNG's signature, fails and writes nothing.
/// Planted defect this catches: the image written once its header reads,
/// so a cut one reaches the file.
#[test]
fn an_image_that_does_not_decode_whole_fails_and_writes_nothing() {
    local(async {
        let plugged = Plugged::new();
        // Its header whole, its pixels cut short.
        let whole = rgba_png(64, 64, 128);
        let cut = whole[..whole.len() - 24].to_vec();
        for answer in [b"<html>busy</html>".to_vec(), b"\x89PNG\r\n\x1a\n".to_vec(), cut] {
            plugged.answer(move |_| Ok(made(answer.clone())));
            let (code, stdout, stderr) = plugged.run(&["image", "generate", "a fox", "--model", "m"]).await;
            assert_eq!(
                (code, stdout.len(), stderr.as_str()),
                (1, 0, "demi image generate: ChatGPT returned an image that could not be read\n")
            );
        }
    });
}

/// A transparent background is a transparent pixel, not an alpha channel:
/// an RGBA PNG whose every pixel is opaque warns, one with a clear pixel
/// does not, and an account the user may not see is not named.
/// Planted defect this catches: the check that reads the channel alone.
#[test]
fn an_opaque_image_with_an_alpha_channel_warns_and_an_unnamed_account_stays_unnamed() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| Ok(MadeImage { account: None, ..made(rgba_png(6, 4, 255)) }));
        let (code, _, stderr) = plugged.run(&["image", "generate", "a fox", "--transparent", "--model", "m"]).await;
        assert_eq!(
            (code, stderr.as_str()),
            (
                0,
                "6×4 PNG, asked of gpt-image-2 (ChatGPT)\ndemi image generate: warning: asked for a transparent background, got an opaque image\n"
            )
        );
        plugged.answer(|_| Ok(made(rgba_png(6, 4, 0))));
        let (code, _, stderr) = plugged.run(&["image", "generate", "a fox", "--transparent", "--model", "m"]).await;
        assert_eq!((code, stderr.as_str()), (0, "6×4 PNG, asked of gpt-image-2 (ChatGPT, alex@example.com)\n"));
    });
}

/// The bytes written are the format asked for, PNG by default, whatever the
/// vendor sent.
///
/// - Grok's JPEG is written as a PNG, and a PNG as JPEG or WebP on request.
/// - A JPEG asked for with a transparent background is sent as asked and warns,
///   as a JPEG holds no transparency.
///
/// Planted defect this catches: the vendor's bytes passed on as they came, so a
/// file named fox.png holds a JPEG.
#[test]
fn the_image_is_written_in_the_format_asked_for_whatever_the_vendor_sent() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| Ok(from_grok(jpeg(8, 6))));
        let (code, stdout, stderr) = plugged.run(&["image", "generate", "a fox", "--model", "m"]).await;
        assert_eq!((code, stderr.as_str()), (0, "8×6 PNG from grok-imagine-image (Grok, alex@example.com)\n"));
        assert_eq!(image::guess_format(&stdout).unwrap(), ImageFormat::Png);
        assert_eq!(image::load_from_memory(&stdout).unwrap().dimensions(), (8, 6));

        plugged.answer(|_| Ok(made(rgba_png(6, 4, 0))));
        for (format, written, name) in [("jpeg", ImageFormat::Jpeg, "JPEG"), ("webp", ImageFormat::WebP, "WebP")] {
            let (code, stdout, stderr) =
                plugged.run(&["image", "generate", "a fox", "--transparent", "--format", format, "--model", "m"]).await;
            assert_eq!(image::guess_format(&stdout).unwrap(), written);
            let said = format!("6×4 {name}, asked of gpt-image-2 (ChatGPT, alex@example.com)\n");
            let warned = "demi image generate: warning: asked for a transparent background, got an opaque image\n";
            let expected = if written == ImageFormat::Jpeg { format!("{said}{warned}") } else { said };
            assert_eq!((code, stderr), (0, expected));
        }
        // The two transparent asks went to the vendor as asked.
        assert!(plugged.demi.images_asked.borrow().iter().skip(1).all(|asked| asked.transparent));
    });
}

/// A vendor's JPEG cut short, which the JPEG decoder would fill with gray,
/// fails and writes nothing rather than becoming a PNG of a half picture.
/// Planted defect this catches: the conversion that trusts the decoder.
#[test]
fn a_cut_jpeg_from_the_vendor_is_not_converted() {
    local(async {
        let plugged = Plugged::new();
        let mut bytes = Cursor::new(Vec::new());
        RgbImage::from_fn(64, 48, |x, y| Rgb([(x * 4) as u8, (y * 5) as u8, (x ^ y) as u8]))
            .write_to(&mut bytes, ImageFormat::Jpeg)
            .unwrap();
        let whole = bytes.into_inner();
        let cut = whole[..whole.len() * 2 / 3].to_vec();
        plugged.answer(move |_| Ok(from_grok(cut.clone())));
        let (code, stdout, stderr) = plugged.run(&["image", "generate", "a fox", "--model", "m"]).await;
        assert_eq!(
            (code, stdout.len(), stderr.as_str()),
            (1, 0, "demi image generate: Grok returned an image that could not be read\n")
        );
    });
}
