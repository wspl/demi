//! The pictures of the Host a reply shows, as the transcript names them
//! (`docs/decisions/0046-a-reply-keeps-the-pictures-it-shows.md`).

use demi_agent_transcript::{named_pictures, picture_path};

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
