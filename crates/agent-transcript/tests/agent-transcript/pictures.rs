//! The pictures of the Host a reply shows, as the transcript names them when
//! the reply's text is complete
//! (`docs/decisions/0046-a-reply-keeps-the-pictures-it-shows.md`).

use std::{rc::Rc, sync::Arc};

use demi_agent_store::testing::test_model;
use demi_agent_transcript::{TranscriptLog, named_pictures, picture_path, testing::SequentialIds};
use demi_conversation_socket_protocol::TranscriptPatch;
use demi_shared_types::{Block, PictureCopy, ReplyPicture, Timestamp};

// Cost: pure, under a millisecond.

struct Fixed;

impl demi_shared_types::Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp::UNIX_EPOCH
    }
}

/// Which image targets of a reply are pictures of the Host, and the path each
/// names, as the page's reading of a target is (`filePath.ts`): the backend
/// copies exactly the pictures the page shows as copies.
#[test]
fn an_image_target_names_a_picture_of_the_host_as_the_page_reads_it() {
    let cases: &[(&str, Option<&str>)] = &[
        ("out/chart.png", Some("out/chart.png")),
        ("./out/chart.png", Some("out/chart.png")),
        ("chart.png", Some("chart.png")),
        ("/Users/alex/out/chart.png", Some("/Users/alex/out/chart.png")),
        ("~/out/chart.png", Some("~/out/chart.png")),
        ("C:\\work\\chart.png", Some("C:\\work\\chart.png")),
        ("file:///work/chart.png", Some("/work/chart.png")),
        ("out/my%20chart.png?raw=1#top", Some("out/my chart.png")),
        ("https://example.com/chart.png", None),
        ("data:image/png;base64,AAAA", None),
        ("mailto:a@example.com", None),
        ("#top", None),
        ("chart", None),
        ("", None),
    ];
    for (target, path) in cases {
        assert_eq!(picture_path(target).as_deref(), *path, "{target}");
    }
}

/// The targets of the images the page and the backend read alike, in
/// `fixtures/picture-targets.json`, which the page's own test reads too
/// (`packages/web-ui/src/markdown/__tests__/render.test.ts`): the target a
/// reply keeps a picture by is the one the page looks its copy up by.
#[test]
fn the_pictures_a_reply_names_are_the_ones_the_page_looks_up() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!("fixtures/picture-targets.json")).unwrap();
    for case in cases {
        let markdown = case["markdown"].as_str().unwrap();
        let named: Vec<String> = named_pictures(markdown).into_iter().map(|(target, _)| target).collect();
        let expected: Vec<String> = case["pictures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|target| target.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(named, expected, "{markdown}");
    }
}

fn log_with_reply(text: &str) -> TranscriptLog {
    let mut log = TranscriptLog::new(Vec::new(), Rc::new(SequentialIds::new("b")), Arc::new(Fixed));
    log.append_text(&test_model(), text);
    log.take_patches();
    log
}

/// The step that completes a reply names its pictures: one patch carries the
/// complete mark and the pictures, so a page never sees one without the
/// other. A text that shows no picture of the Host keeps nothing.
#[test]
fn the_step_that_completes_a_reply_names_its_pictures() {
    let mut log = log_with_reply("![chart](out/chart.png) ![again](out/chart.png) ![web](https://example.com/x.png)");
    assert!(log.complete_tail_text());
    let batch = log.take_patches().expect("completing is a change");
    let [TranscriptPatch::ReplaceBlock { value: Block::Text(text), .. }]: [TranscriptPatch; 1] =
        batch.patches.try_into().expect("one patch")
    else {
        panic!("a replaced text")
    };
    assert!(text.forkable);
    assert_eq!(
        text.pictures,
        Some(vec![ReplyPicture { target: "out/chart.png".into(), copy: PictureCopy::Copying }])
    );

    let mut plain = log_with_reply("Nothing to show.");
    assert!(plain.complete_tail_text());
    let Block::Text(text) = &plain.blocks()[0] else { panic!("a text") };
    assert_eq!(text.pictures, None);
}
