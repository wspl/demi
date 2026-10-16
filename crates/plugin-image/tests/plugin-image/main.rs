//! The image plugin through the JSON loopback over an in-memory Demi, which
//! answers its file reads and its images: the words a failure is told in,
//! and an edit's images read from the conversation's Host. The whole path
//! through the backend, with a real device, is the backend's `images`
//! scenarios'.

mod support;

use std::io::Cursor;

use demi_plugin_interface::{ImageKind, ImageRefusal, MadeImage, PortRefusal};
use image::{ImageFormat, Rgb, RgbImage};

use crate::support::{Plugged, local};

/// A PNG of `width`×`height` px, as a vendor answers one.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    RgbImage::from_pixel(width, height, Rgb([200, 30, 30]))
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn made(bytes: Vec<u8>) -> MadeImage {
    MadeImage {
        bytes: bytes.into(),
        model: "gpt-image-2".into(),
        provider: "Codex".into(),
        vendor: "ChatGPT".into(),
        account: Some("alex@example.com".into()),
    }
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

/// Each refusal of the Providers service, and an answer that is no image,
/// is told on stderr in the design's words with exit status 1; a reset
/// on another day names the day, in the user's time zone.
/// Planted defect this catches: a refusal passed on as the port's own
/// text, which names neither the vendor's words nor what to do.
#[test]
fn a_failure_is_told_in_the_designs_words() {
    local(async {
        let plugged = Plugged::new();
        let refusals = [
            (
                ImageRefusal::UsedUp {
                    vendor: "ChatGPT".into(),
                    resets_at: Some("2030-01-01T13:00:00Z".parse().unwrap()),
                },
                "ChatGPT's usage limit is reached, resetting at Jan 1, 9:00 PM",
            ),
            (
                ImageRefusal::UsedUp { vendor: "Grok".into(), resets_at: None },
                "Grok's usage limit is reached",
            ),
            (
                ImageRefusal::Refused { vendor: "ChatGPT".into(), message: "Your request was rejected.".into() },
                "ChatGPT refused the request: Your request was rejected.",
            ),
            (
                ImageRefusal::Expired { vendor: "ChatGPT".into(), account: Some("alex@example.com".into()) },
                "the ChatGPT sign-in for alex@example.com has expired; sign in again in Settings › Providers",
            ),
            (
                ImageRefusal::Expired { vendor: "ChatGPT".into(), account: None },
                "the ChatGPT sign-in has expired; ask your administrator to sign in again",
            ),
            (
                ImageRefusal::Unanswered { vendor: "Grok".into(), cause: "no answer in 120 s".into() },
                "Grok did not answer: no answer in 120 s",
            ),
        ];
        for (refusal, words) in refusals {
            plugged.answer(move |_| Err(PortRefusal::Image { refusal: refusal.clone() }));
            let (code, stdout, stderr) = plugged.run(&["image", "generate", "a fox"]).await;
            assert_eq!((code, stdout.len(), stderr), (1, 0, format!("demi image generate: {words}\n")));
        }
    });
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
            let (code, stdout, stderr) = plugged.run(&["image", "generate", "a fox"]).await;
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
        let (code, _, stderr) = plugged.run(&["image", "generate", "a fox", "--transparent"]).await;
        assert_eq!(
            (code, stderr.as_str()),
            (
                0,
                "6×4 PNG from gpt-image-2 (ChatGPT)\ndemi image generate: warning: asked for a transparent background, got an opaque image\n"
            )
        );
        plugged.answer(|_| Ok(made(rgba_png(6, 4, 0))));
        let (code, _, stderr) = plugged.run(&["image", "generate", "a fox", "--transparent"]).await;
        assert_eq!((code, stderr.as_str()), (0, "6×4 PNG from gpt-image-2 (ChatGPT, alex@example.com)\n"));
    });
}

/// An edit's images are read in one read, each path as given, which the
/// port resolves on the job's Host, and sent with their kinds; a missing
/// file, one over 16 MiB and a stopped Host are named.
/// Planted defect this catches: an image sent without its kind, or a file
/// over the limit sent cut.
#[test]
fn an_edit_reads_its_images_beside_the_shell_and_names_one_it_cannot_send() {
    local(async {
        let plugged = Plugged::new();
        let fox = png(4, 3);
        let mut big = png(1, 1);
        big.resize(16 * 1024 * 1024 + 1, 0);
        plugged.host_files(Some(vec![
            ("/workspace/fox.png", fox.clone()),
            ("/photos/cat.jpg", b"\xff\xd8\xff\xe0 a jpeg".to_vec()),
            ("/workspace/big.png", big),
        ]));
        let answer = png(8, 8);
        plugged.answer(move |_| Ok(made(answer.clone())));
        let (code, stdout, stderr) = plugged.run(&["image", "edit", "rounder ears", "fox.png", "/photos/cat.jpg"]).await;
        assert_eq!((code, stderr.as_str()), (0, "8×8 PNG from gpt-image-2 (ChatGPT, alex@example.com)\n"));
        assert_eq!(stdout, png(8, 8));
        let asked = plugged.demi.images_asked.borrow().last().cloned().unwrap();
        let sent: Vec<(ImageKind, Vec<u8>)> =
            asked.images.iter().map(|image| (image.kind, image.bytes.to_vec())).collect();
        assert_eq!(
            sent,
            [(ImageKind::Png, fox), (ImageKind::Jpeg, b"\xff\xd8\xff\xe0 a jpeg".to_vec())]
        );

        for (path, reason) in [
            ("gone.png", "no such file"),
            ("big.png", "the file is 16777217 bytes; an image is at most 16 MiB"),
        ] {
            let (code, _, stderr) = plugged.run(&["image", "edit", "x", path]).await;
            assert_eq!((code, stderr), (1, format!("demi image edit: {path}: {reason}\n")));
        }
        plugged.host_files(None);
        let (code, _, stderr) = plugged.run(&["image", "edit", "x", "fox.png"]).await;
        assert_eq!((code, stderr.as_str()), (1, "demi image edit: the Host is not running\n"));
    });
}
