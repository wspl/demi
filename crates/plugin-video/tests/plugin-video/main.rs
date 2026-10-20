//! The video plugin through the JSON loopback over an in-memory Demi, which
//! answers its file reads and its video's job.
//!
//! The tests cover the lines a job's states are told in, the MP4 checked before
//! a byte reaches stdout, the summary and its warnings, the frames read from
//! the conversation's Host, and the words a failure is told in. The whole path
//! through the backend is the backend's `videos` scenarios'.

mod support;

use demi_plugin_interface::{
    AskedVideo, ImageKind, MadeVideo, MediaRefusal, PortRefusal, VideoOption, VideoResolution, VideoStep,
};
use demi_shared_types::{Cost, Currency};

use crate::support::{Plugged, local};

/// One second of 64×36 px with a sound track.
const CLIP: &[u8] = include_bytes!("fixtures/clip.mp4");

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n frame";

fn asked(shape: &str, unsent: Vec<VideoOption>) -> AskedVideo {
    AskedVideo {
        job: "d97415a1".into(),
        model: "grok-imagine-video".into(),
        provider: "Grok".into(),
        vendor: "Grok".into(),
        account: Some("alex@example.com".into()),
        shape: shape.into(),
        unsent,
    }
}

fn made(bytes: &[u8]) -> Result<VideoStep, PortRefusal> {
    Ok(VideoStep::Made {
        video: MadeVideo {
            bytes: bytes.to_vec().into(),
            cost: Some(Cost { amount: 0.05, currency: Currency::Usd }),
        },
    })
}

/// The job is named at once, each change of its state is a line, and the
/// checked MP4 goes to stdout with a line that says what came back.
///
/// Planted defects this catches:
///
/// - the job named only once it ends, which leaves the agent no id of a paid
///   job if the call is stopped;
/// - a state that is not told;
/// - the length read from the vendor's word rather than the file.
#[test]
fn the_job_is_named_at_once_its_states_told_and_the_video_summed_up() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| Ok(asked("Grok", Vec::new())));
        plugged.steps([
            Ok(VideoStep::Queued),
            Ok(VideoStep::Running { progress: None }),
            Ok(VideoStep::Running { progress: Some(40) }),
            made(CLIP),
        ]);
        let (code, stdout, stderr) =
            plugged.run(&["video", "generate", "a paper boat", "--model", "grok-imagine-video"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(stdout, CLIP);
        assert_eq!(
            stderr,
            "demi video generate: asked grok-imagine-video (Grok, alex@example.com), job d97415a1\n\
             demi video generate: queued\n\
             demi video generate: running\n\
             demi video generate: running, 40%\n\
             1 s 64×36 MP4 with sound from grok-imagine-video (Grok, alex@example.com), $0.05\n"
        );
        let ask = &plugged.demi.videos_asked.borrow()[0];
        assert_eq!((ask.prompt.as_str(), ask.model.as_str()), ("a paper boat", "grok-imagine-video"));
    });
}

/// Each option the shape took no field for is a warning, then each the result
/// does not honour. One that matches, the 16:9 of 64×36, is not. An option not
/// sent is told once, with what the result shows of it.
///
/// Planted defects this catches:
///
/// - an unsent option left untold, so the agent believes the seed was used;
/// - a mismatch told as a failure, which would throw away a paid video.
#[test]
fn what_the_shape_does_not_take_and_the_result_does_not_honour_are_warnings() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| Ok(asked("Grok", vec![VideoOption::Seed])));
        plugged.steps([made(CLIP)]);
        let line = [
            "video", "generate", "a paper boat", "--model", "grok-imagine-video", "--duration", "5", "--aspect", "16:9",
            "--resolution", "480p", "--no-audio", "--seed", "7",
        ];
        let (code, stdout, stderr) = plugged.run(&line).await;
        assert_eq!((code, stdout.len()), (0, CLIP.len()), "{stderr}");
        let warnings: Vec<&str> = stderr.lines().filter(|line| line.contains("warning")).collect();
        assert_eq!(
            warnings,
            [
                "demi video generate: warning: Grok takes no --seed",
                "demi video generate: warning: asked for 5 s, got 1 s",
                "demi video generate: warning: asked for 480p, got 64×36",
                "demi video generate: warning: asked for no sound, got a video with sound",
            ]
        );
        let ask = plugged.demi.videos_asked.borrow()[0].clone();
        assert_eq!(
            (ask.duration, ask.resolution, ask.audio, ask.seed),
            (Some(5), Some(VideoResolution::Height(480)), Some(false), Some(7))
        );

        plugged.answer(|_| Ok(asked("OpenAI Videos", vec![VideoOption::Audio])));
        plugged.steps([made(CLIP)]);
        let (_, _, stderr) = plugged.run(&["video", "generate", "a boat", "--model", "m", "--no-audio"]).await;
        let warnings: Vec<&str> = stderr.lines().filter(|line| line.contains("warning")).collect();
        assert_eq!(warnings, ["demi video generate: warning: OpenAI Videos takes no --no-audio; the video has sound"]);
    });
}

/// `--resolution` takes any height or count of thousands across a vendor names,
/// as MiniMax's `768P` and `2K`, and sends it as read. One the result does not
/// honour is a warning in `--resolution`'s words, and a value of neither form
/// is refused before anything is asked.
///
/// Planted defects this catches:
///
/// - a closed list of resolutions, which refuses MiniMax's;
/// - a `2k` result never checked;
/// - a malformed value passed to the plugin.
// Two calls against a scripted Providers service and one parse: milliseconds.
#[test]
fn a_resolution_is_any_height_or_thousands_across() {
    local(async {
        let plugged = Plugged::new();
        for resolution in ["768P", "2k"] {
            plugged.answer(|_| Ok(asked("Hailuo", Vec::new())));
            plugged.steps([made(CLIP)]);
            let (code, _, stderr) =
                plugged.run(&["video", "generate", "a boat", "--model", "MiniMax-H3", "--resolution", resolution]).await;
            assert_eq!(code, 0, "{stderr}");
            let asked = resolution.to_lowercase();
            let warnings: Vec<&str> = stderr.lines().filter(|line| line.contains("warning")).collect();
            assert_eq!(warnings, [format!("demi video generate: warning: asked for {asked}, got 64×36")]);
        }
        let sent: Vec<_> = plugged.demi.videos_asked.borrow().iter().map(|ask| ask.resolution).collect();
        assert_eq!(sent, [Some(VideoResolution::Height(768)), Some(VideoResolution::K(2))]);

        let refused = plugged.refused(&["video", "generate", "a boat", "--model", "MiniMax-H3", "--resolution", "hd"]);
        assert!(refused.contains("\\\"hd\\\" is not a height such as 720p, or 2k or 4k"), "{refused}");
    });
}

/// A file that is not an MP4 fails the call and writes nothing; its job
/// was already named.
/// Planted defect this catches: the bytes written before the check, which
/// leaves a broken file behind `> clip.mp4`.
#[test]
fn a_file_that_is_not_an_mp4_fails_and_writes_nothing() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| Ok(asked("Grok", Vec::new())));
        plugged.steps([made(b"<html>Access denied</html>")]);
        let (code, stdout, stderr) = plugged.run(&["video", "generate", "a boat", "--model", "m"]).await;
        assert_eq!((code, stdout.len()), (1, 0));
        let last = stderr.lines().last().unwrap();
        assert!(
            last.starts_with("demi video generate: Grok returned a file that is not a video this reads: the file is not an MP4"),
            "{stderr}"
        );
    });
}

/// The frames and references are read from the conversation's Host beside the
/// shell, in their places. A last frame without a first, or sound asked both
/// ways, fails before anything is asked.
///
/// Planted defects this catches:
///
/// - a reference sent as the last frame when no last frame is given;
/// - a call with a last frame alone sent, which every vendor refuses after the
///   ask is paid for on some.
#[test]
fn frames_and_references_are_read_from_the_host_and_a_lone_last_frame_is_refused() {
    local(async {
        let plugged = Plugged::new();
        plugged.host_files(vec![("/workspace/first.png", PNG.to_vec()), ("/workspace/cat.png", PNG.to_vec())]);
        plugged.answer(|_| Ok(asked("Grok", Vec::new())));
        plugged.steps([made(CLIP)]);
        let line = ["video", "generate", "a cat", "--model", "m", "--image", "first.png", "--reference", "cat.png"];
        let (code, _, stderr) = plugged.run(&line).await;
        assert_eq!(code, 0, "{stderr}");
        let ask = plugged.demi.videos_asked.borrow()[0].clone();
        assert_eq!(ask.image.map(|image| image.kind), Some(ImageKind::Png));
        assert!(ask.last_frame.is_none());
        assert_eq!(ask.references.len(), 1);

        let (code, _, stderr) = plugged.run(&["video", "generate", "a cat", "--model", "m", "--last-frame", "cat.png"]).await;
        assert_eq!((code, stderr.as_str()), (1, "demi video generate: --last-frame needs --image, the first frame\n"));
        let (code, _, stderr) = plugged.run(&["video", "generate", "a cat", "--model", "m", "--audio", "--no-audio"]).await;
        assert_eq!(
            (code, stderr.as_str()),
            (1, "demi video generate: --audio and --no-audio ask for opposite things; give one\n")
        );
        assert_eq!(plugged.demi.videos_asked.borrow().len(), 1);
    });
}

/// A job's end that is not a video, and a refusal at the ask, are told in
/// the design's words with exit status 1; the job's id stays on the first
/// line.
/// Planted defect this catches: a job still running told without its id,
/// which leaves the agent nothing to name to the user.
#[test]
fn a_failure_is_told_in_the_designs_words() {
    local(async {
        let plugged = Plugged::new();
        let ended = [
            (
                MediaRefusal::JobFailed { vendor: "Grok".into(), message: "Video generation failed".into(), trace: Default::default() },
                "Grok failed the video: Video generation failed",
            ),
            (MediaRefusal::JobExpired { vendor: "Grok".into() }, "the job expired at Grok before it ran"),
            (
                MediaRefusal::StillRunning { vendor: "Grok".into(), job: "d97415a1".into(), seconds: 600 },
                "the job d97415a1 is still running at Grok after 10 minutes; it runs to its end there",
            ),
        ];
        for (refusal, words) in ended {
            plugged.answer(|_| Ok(asked("Grok", Vec::new())));
            plugged.steps([Ok(VideoStep::Running { progress: None }), Err(PortRefusal::Media { refusal })]);
            let (code, stdout, stderr) = plugged.run(&["video", "generate", "a boat", "--model", "m"]).await;
            assert_eq!((code, stdout.len()), (1, 0));
            assert_eq!(
                stderr,
                format!(
                    "demi video generate: asked grok-imagine-video (Grok, alex@example.com), job d97415a1\n\
                     demi video generate: running\n\
                     demi video generate: {words}\n"
                )
            );
        }
        let refused = [
            (
                MediaRefusal::NoProvider,
                "no provider makes videos; sign in to Grok, or add an xAI, Google, MiniMax, BytePlus ModelArk or Volcengine Ark key or a compatible API with a video shape, in Settings › Providers",
            ),
            (
                MediaRefusal::AllOff { providers: vec!["Grok".into()] },
                "video generation is off for Grok; turn it on in Settings › Providers",
            ),
            (MediaRefusal::UnknownModel { model: "m".into() }, "no provider makes \"m\"; demi video models lists them"),
        ];
        for (refusal, words) in refused {
            plugged.answer(move |_| Err(PortRefusal::Media { refusal: refusal.clone() }));
            let (code, _, stderr) = plugged.run(&["video", "generate", "a boat", "--model", "m"]).await;
            assert_eq!((code, stderr), (1, format!("demi video generate: {words}\n")));
        }
    });
}

/// One video model is labelled lines with what a video model takes, its
/// prices per second by resolution, and the entries in the order a call
/// tries them; the list names the entry a call takes now.
/// Planted defect this catches: a video model shown with the image facts,
/// which leaves its durations and resolutions out.
#[test]
fn a_video_model_shows_its_durations_resolutions_frames_and_prices_per_second() {
    use demi_plugin_interface::{EntryStatus, MediaListing, MediaModel, MediaPrice, ModelEntry, PriceRate, VideoFacts};
    use demi_shared_types::MediaKind;
    local(async {
        let plugged = Plugged::new();
        let rate = |resolution: &str, amount: f64| PriceRate {
            unit: "second".into(),
            modality: None,
            amount,
            currency: "USD".into(),
            size: None,
            quality: None,
            resolution: Some(resolution.into()),
            duration: None,
        };
        let model = MediaModel {
            id: "grok-imagine-video".into(),
            vendor: Some("xAI".into()),
            vendor_id: Some("xai".into()),
            name: Some("Grok Imagine Video".into()),
            description: None,
            released: Some("2026-01-28".into()),
            alias_of: None,
            aliases: Vec::new(),
            image: None,
            audio: None,
            video: Some(VideoFacts {
                durations: None,
                min_duration: Some(1.0),
                max_duration: Some(15.0),
                aspect_ratios: Some(vec!["16:9".into(), "9:16".into()]),
                resolutions: Some(vec!["480p".into(), "720p".into()]),
                first_frame: Some(true),
                last_frame: Some(false),
                max_reference_images: Some(0),
                audio: Some(true),
            }),
            price: Some(MediaPrice {
                rates: vec![rate("480p", 0.05), rate("720p", 0.07)],
                page: Some("https://docs.x.ai/developers/models".into()),
            }),
            entries: vec![ModelEntry {
                provider: "Grok".into(),
                provider_id: "grok".into(),
                subscription: true,
                account: Some("alex@example.com".into()),
                confirmed: true,
                status: EntryStatus::Now,
            }],
        };
        plugged
            .demi
            .media_models
            .borrow_mut()
            .insert(MediaKind::Video, MediaListing { models: vec![model], ..MediaListing::default() });
        let (code, stdout, stderr) = plugged.run(&["video", "models", "grok-imagine-video"]).await;
        assert_eq!(code, 0, "{stderr}");
        let shown = String::from_utf8(stdout).unwrap();
        assert!(
            shown.contains(
                "Durations: 1 to 15 s\nAspect ratios: 16:9, 9:16\nResolutions: 480p, 720p\n\
                 First frame: yes\nLast frame: no\nMax reference images: 0\nSound: yes\n"
            ),
            "{shown}"
        );
        let prices: Vec<Vec<&str>> = shown
            .lines()
            .filter(|line| line.trim_start().starts_with("second"))
            .map(|line| line.split_whitespace().collect())
            .collect();
        assert_eq!(prices, [["second", "-", "0.05", "USD", "-", "-", "480p", "-"], ["second", "-", "0.07", "USD", "-", "-", "720p", "-"]]);
        assert!(shown.contains("Grok   subscription  alex@example.com  included  now"), "{shown}");
        let (_, stdout, _) = plugged.run(&["video", "models"]).await;
        let listed = String::from_utf8(stdout).unwrap();
        assert_eq!(listed.lines().nth(1).map(|line| line.split_whitespace().collect::<Vec<_>>()),
            Some(vec!["grok-imagine-video", "xAI", "2026-01-28", "Grok", "Grok", "Imagine", "Video"]));
    });
}
