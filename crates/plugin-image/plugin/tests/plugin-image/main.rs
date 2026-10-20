//! The image plugin through the JSON loopback over an in-memory Demi, which
//! answers its file reads and its images.
//!
//! The tests cover the words a failure is told in, and an edit's images read
//! from the conversation's Host. The whole path through the backend, with a
//! real device, is the backend's `images` scenarios'.

mod support;

use std::io::Cursor;

use demi_plugin_interface::{
    EntryStatus, ImageFacts, ImageKind, MediaRefusal, MediaUnlisted, MadeImage, MediaListing, MediaModel, MediaPrice, ModelEntry,
    PortRefusal, PriceRate, Unable,
};
use demi_shared_types::MediaKind;
use image::{GenericImageView as _, ImageFormat, Rgb, RgbImage};

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
                MediaRefusal::UsedUp {
                    vendor: "ChatGPT".into(),
                    resets_at: Some("2030-01-01T13:00:00Z".parse().unwrap()), trace: Default::default(),
                },
                "ChatGPT's usage limit is reached, resetting at Jan 1, 9:00 PM",
            ),
            (
                MediaRefusal::UsedUp { vendor: "Grok".into(), resets_at: None, trace: Default::default() },
                "Grok's usage limit is reached",
            ),
            (
                MediaRefusal::Refused { vendor: "ChatGPT".into(), message: "Your request was rejected.".into(), trace: Default::default() },
                "ChatGPT refused the request: Your request was rejected.",
            ),
            (
                MediaRefusal::Expired { vendor: "ChatGPT".into(), account: Some("alex@example.com".into()) },
                "the ChatGPT sign-in for alex@example.com has expired; sign in again in Settings › Providers",
            ),
            (
                MediaRefusal::Expired { vendor: "ChatGPT".into(), account: None },
                "the ChatGPT sign-in has expired; ask your administrator to sign in again",
            ),
            (
                MediaRefusal::Unanswered { vendor: "Grok".into(), cause: "no answer in 120 s".into(), trace: Default::default() },
                "Grok did not answer: no answer in 120 s",
            ),
            (
                MediaRefusal::AllOff { providers: vec!["ChatGPT".into(), "Grok".into()] },
                "image generation is off for ChatGPT and Grok; turn it on in Settings › Providers",
            ),
            (
                MediaRefusal::KeyRefused { vendor: "OpenAI".into(), manages: true, trace: Default::default() },
                "OpenAI refused the API key; check it in Settings › Providers",
            ),
            (
                MediaRefusal::KeyRefused { vendor: "xAI".into(), manages: false, trace: Default::default() },
                "xAI refused the API key; ask your administrator to check it",
            ),
            (
                MediaRefusal::NoProvider,
                "no provider makes images; sign in to ChatGPT or Grok, or add an OpenAI, xAI, Google, MiniMax, BytePlus ModelArk or Volcengine Ark key, in Settings › Providers",
            ),
            (
                MediaRefusal::Off { model: "gpt-image-2".into(), provider: "ChatGPT".into() },
                "\"gpt-image-2\" is ChatGPT's, whose image generation is off in Settings › Providers",
            ),
        ];
        for (refusal, words) in refusals {
            plugged.answer(move |_| Err(PortRefusal::Media { refusal: refusal.clone() }));
            let (code, stdout, stderr) = plugged.run(&["image", "generate", "a fox", "--model", "m"]).await;
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
        let (code, stdout, stderr) = plugged.run(&["image", "edit", "rounder ears", "fox.png", "/photos/cat.jpg", "--model", "m"]).await;
        assert_eq!((code, stderr.as_str()), (0, "8×8 PNG, asked of gpt-image-2 (ChatGPT, alex@example.com)\n"));
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
            let (code, _, stderr) = plugged.run(&["image", "edit", "x", path, "--model", "m"]).await;
            assert_eq!((code, stderr), (1, format!("demi image edit: {path}: {reason}\n")));
        }
        plugged.host_files(None);
        let (code, _, stderr) = plugged.run(&["image", "edit", "x", "fox.png", "--model", "m"]).await;
        assert_eq!((code, stderr.as_str()), (1, "demi image edit: the Host is not running\n"));
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

/// A model the vendor does not confirm is named as asked of, and one it
/// confirms as the one the image came from.
/// Planted defect this catches: ChatGPT's request read as a confirmed model.
#[test]
fn a_model_chatgpt_cannot_confirm_is_named_as_asked_of() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| Ok(made(png(1254, 1254))));
        let (_, _, stderr) = plugged.run(&["image", "generate", "a fox", "--model", "m"]).await;
        assert_eq!(stderr, "1254×1254 PNG, asked of gpt-image-2 (ChatGPT, alex@example.com)\n");
        plugged.answer(|_| Ok(from_grok(png(4, 4))));
        let (_, _, stderr) = plugged.run(&["image", "generate", "a fox", "--model", "m"]).await;
        assert_eq!(stderr, "4×4 PNG from grok-imagine-image (Grok, alex@example.com)\n");
    });
}

/// A shape the vendor refuses fails with the shape asked for and the
/// shapes the vendor named, in its order.
/// Planted defect this catches: the refusal told without the shapes, or
/// with a list of Demi's own.
#[test]
fn a_refused_shape_names_the_shapes_the_vendor_takes() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| {
            Err(PortRefusal::Media {
                refusal: MediaRefusal::Shape {
                    vendor: "Grok".into(),
                    takes: vec!["1:1".into(), "3:4".into(), "19.5:9".into()], trace: Default::default(),
                },
            })
        });
        let (code, stdout, stderr) = plugged.run(&["image", "generate", "an otter", "--aspect", "4:5", "--model", "m"]).await;
        assert_eq!(
            (code, stdout.len(), stderr.as_str()),
            (1, 0, "demi image generate: Grok takes no 4:5 (it takes 1:1, 3:4, 19.5:9)\n")
        );
    });
}

/// A call without `--model` fails before it asks for anything: the agent
/// always names the model. Planted defect this catches: a call without a
/// model sent on, so an entry's own choice makes the image.
#[test]
fn a_call_without_a_model_fails_and_asks_for_nothing() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| Ok(made(png(4, 4))));
        for (args, command) in [
            (&["image", "generate", "a fox"][..], "generate"),
            (&["image", "edit", "rounder", "fox.png"][..], "edit"),
        ] {
            let (code, stdout, stderr) = plugged.run(args).await;
            assert_eq!(
                (code, stdout.len(), stderr),
                (1, 0, format!("demi image {command}: --model is required; demi image models lists them\n"))
            );
        }
        assert!(plugged.demi.images_asked.borrow().is_empty());
    });
}

fn entry(provider: &str, subscription: bool, account: Option<&str>, confirmed: bool, status: EntryStatus) -> ModelEntry {
    ModelEntry {
        provider: provider.into(),
        provider_id: provider.to_lowercase(),
        subscription,
        account: account.map(str::to_owned),
        confirmed,
        status,
    }
}

fn model(id: &str, vendor: Option<&str>, released: Option<&str>, name: Option<&str>, entries: Vec<ModelEntry>) -> MediaModel {
    MediaModel {
        id: id.into(),
        vendor: vendor.map(str::to_owned),
        vendor_id: vendor.map(str::to_lowercase),
        name: name.map(str::to_owned),
        description: None,
        released: released.map(str::to_owned),
        alias_of: None,
        aliases: Vec::new(),
        image: None,
        video: None,
        audio: None,
        price: None,
        entries,
    }
}

/// The list is a table (`crates/shared-types/src/output.rs`).
///
/// - It has one row per model with the entry a call takes now, and `-` where
///   nothing is known.
/// - Columns are aligned by the width a terminal gives them, a CJK name
///   counting two.
/// - What is no data, an entry whose models could not be listed or one that
///   does not confirm the model, goes to stderr.
/// - One model is labelled lines with its prices and its entries as tables, and
///   `--json` prints the same.
///
/// Planted defects this catches:
///
/// - a CJK name that pushes its row out of line;
/// - the entry a call takes named from one used up;
/// - a note on stdout;
/// - a fact or a status left out of the page.
#[test]
fn the_models_are_a_table_one_model_is_labelled_lines_and_json_prints_the_same() {
    local(async {
        let plugged = Plugged::new();
        let used_up = EntryStatus::UsedUp { until: Some("2026-10-17T18:00:00Z".parse().unwrap()) };
        let mut gpt = model(
            "gpt-image-2",
            Some("OpenAI"),
            Some("2026-04-21"),
            Some("GPT Image 2"),
            vec![
                entry("ChatGPT", true, Some("alex@example.com"), false, used_up),
                entry("OpenAI", false, None, true, EntryStatus::Now),
                entry("Proxy", false, None, true, EntryStatus::Off),
            ],
        );
        gpt.description = Some("State-of-the-art image generation".into());
        gpt.aliases = vec!["chatgpt-image-latest".into()];
        gpt.image = Some(ImageFacts {
            sizes: Some(vec!["1024x1024".into(), "1536x1024".into()]),
            edits: Some(true),
            max_input_images: Some(16),
            transparent: Some(true),
            ..ImageFacts::default()
        });
        gpt.price = Some(MediaPrice {
            rates: vec![PriceRate {
                unit: "millionInputTokens".into(),
                modality: Some("text".into()),
                amount: 5.0,
                currency: "USD".into(),
                size: None,
                quality: None,
                resolution: None,
                duration: None,
            }, PriceRate {
                unit: "image".into(),
                modality: None,
                amount: 0.167,
                currency: "USD".into(),
                size: Some("1024x1024".into()),
                quality: Some("high".into()),
                resolution: None,
                duration: None,
            }],
            page: Some("https://openai.com/api/pricing".into()),
        });
        plugged.demi.media_models.borrow_mut().insert(MediaKind::Image, MediaListing {
            models: vec![
                model("seedream-5-0", Some("BytePlus"), Some("2026-09-01"), Some("即梦 5.0"), vec![entry("火山", false, None, true, EntryStatus::Now)]),
                gpt.clone(),
                model("own", None, None, None, vec![entry("Proxy", false, None, true, EntryStatus::NotSignedIn)]),
            ],
            unlisted: vec![MediaUnlisted {
                provider: "Gemini".into(),
                refusal: MediaRefusal::KeyRefused { vendor: "Google".into(), manages: true, trace: Default::default() },
            }],
            none: None,
        });
        let (code, stdout, stderr) = plugged.run(&["image", "models"]).await;
        assert_eq!(code, 0);
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "ID            VENDOR    RELEASED    ENTRY   NAME\n\
             seedream-5-0  BytePlus  2026-09-01  火山    即梦 5.0\n\
             gpt-image-2   OpenAI    2026-04-21  OpenAI  GPT Image 2\n\
             own           -         -           -       -\n"
        );
        assert_eq!(
            stderr,
            "demi image models: warning: Gemini's models could not be listed: Google refused the API key; check it in Settings › Providers\n\
             demi image models: note: ChatGPT does not confirm which model answers a call\n"
        );

        let (code, stdout, _) = plugged.run(&["image", "models", "gpt-image-2"]).await;
        assert_eq!(code, 0);
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "ID: gpt-image-2\nVendor: OpenAI\nReleased: 2026-04-21\nEntry: OpenAI\nName: GPT Image 2\nAlias of: -\n\
             Aliases: chatgpt-image-latest\nSizes: 1024x1024, 1536x1024\nCustom size: -\nAspect ratios: -\nResolutions: -\n\
             Edits: yes\nMax input images: 16\nTransparent background: yes\nPrice page: https://openai.com/api/pricing\n\
             Prices:\n\
             \x20 UNIT                MODALITY  AMOUNT  CURRENCY  SIZE       QUALITY  RESOLUTION  DURATION (S)\n\
             \x20 millionInputTokens  text      5       USD       -          -        -           -\n\
             \x20 image               -         0.167   USD       1024x1024  high     -           -\n\
             Entries:\n\
             \x20 ENTRY    KIND          ACCOUNT           PRICE     STATUS\n\
             \x20 ChatGPT  subscription  alex@example.com  included  used up until 2026-10-17T18:00:00Z\n\
             \x20 OpenAI   api-key       -                 listed    now\n\
             \x20 Proxy    api-key       -                 listed    Image Generation off\n\
             Description:\n  State-of-the-art image generation\n"
        );
        let (code, stdout, _) = plugged.run(&["image", "models", "gpt-image-2", "--json"]).await;
        assert_eq!(code, 0);
        let shown: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(shown["entry"], "OpenAI");
        assert_eq!(shown["image"]["maxInputImages"], 16);
        assert_eq!(shown["price"]["rates"][0]["modality"], "text");
        assert_eq!(shown["price"]["rates"][1]["amount"], 0.167);
        assert_eq!(shown["entries"][0]["status"], serde_json::json!({ "status": "usedUp", "until": "2026-10-17T18:00:00.000Z" }));
        let (code, stdout, _) = plugged.run(&["image", "models", "--json"]).await;
        assert_eq!(code, 0);
        let listed: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        let ids: Vec<&str> = listed.as_array().unwrap().iter().map(|model| model["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["seedream-5-0", "gpt-image-2", "own"]);

        // An alias finds the model it names, as a call by it would.
        let (code, by_alias, _) = plugged.run(&["image", "models", "chatgpt-image-latest", "--json"]).await;
        assert_eq!(code, 0);
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&by_alias).unwrap()["id"], "gpt-image-2");

        let (code, _, stderr) = plugged.run(&["image", "models", "nope"]).await;
        assert_eq!((code, stderr.as_str()), (1, "demi image models: no provider makes \"nope\"; demi image models lists them\n"));

        // An empty list is its header alone, and why on stderr.
        plugged.demi.media_models.borrow_mut().insert(MediaKind::Image, MediaListing {
            none: Some(MediaRefusal::AllOff { providers: vec!["ChatGPT".into()] }),
            ..MediaListing::default()
        });
        let (code, stdout, stderr) = plugged.run(&["image", "models"]).await;
        assert_eq!(
            (code, String::from_utf8(stdout).unwrap().as_str(), stderr.as_str()),
            (0, "ID  VENDOR  RELEASED  ENTRY  NAME\n", "demi image models: note: image generation is off for ChatGPT; turn it on in Settings › Providers\n")
        );
    });
}

/// An entry `--provider` names that cannot make the model says why and
/// which can, and the name reaches the Providers service.
/// Planted defect this catches: the reason or the entries that can left
/// out of the words; `--provider` not passed on.
#[test]
fn an_entry_named_that_cannot_make_the_model_says_why_and_which_can() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|ask| match ask.provider.as_deref() {
            Some("Proxy") => Err(PortRefusal::Media {
                refusal: MediaRefusal::Cannot {
                    provider: "Proxy".into(),
                    model: ask.model.clone(),
                    why: Unable::Off,
                    can: vec!["ChatGPT".into(), "OpenAI".into()],
                },
            }),
            _ => Err(PortRefusal::Media {
                refusal: MediaRefusal::Cannot {
                    provider: "Grok".into(),
                    model: ask.model.clone(),
                    why: Unable::NotOffered,
                    can: Vec::new(),
                },
            }),
        });
        let (code, _, stderr) = plugged.run(&["image", "generate", "a fox", "--model", "gpt-image-2", "--provider", "Proxy"]).await;
        assert_eq!(
            (code, stderr.as_str()),
            (1, "demi image generate: Proxy cannot make \"gpt-image-2\": its image generation is off in Settings › Providers; ChatGPT and OpenAI can\n")
        );
        let (_, _, stderr) = plugged.run(&["image", "generate", "a fox", "--model", "gpt-image-2", "--provider", "Grok"]).await;
        assert_eq!(
            stderr,
            "demi image generate: Grok cannot make \"gpt-image-2\": it does not make \"gpt-image-2\"; no other provider can make it now\n"
        );
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
