//! The video plugin through the JSON loopback over an in-memory Demi, which
//! answers its file reads and its video's job.
//!
//! The tests cover the resolution a call may name and the MP4 checked before a
//! byte reaches stdout. The whole path through the backend is the backend's
//! `videos` scenarios'.

mod support;

use demi_plugin_interface::{
    AskedVideo, MadeVideo, PortRefusal, VideoOption, VideoResolution, VideoStep,
};
use demi_shared_types::{Cost, Currency};

use crate::support::{Plugged, local};

/// One second of 64×36 px with a sound track.
const CLIP: &[u8] = include_bytes!("fixtures/clip.mp4");

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
