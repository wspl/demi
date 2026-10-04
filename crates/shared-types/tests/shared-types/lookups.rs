//! The lookups the backend and the web app share: which files the page shows
//! and how, which media a model reads, how an attachment is named to the
//! model, and which context limits a model offers.

use demi_shared_types::{
    Attachment, FileExtension, MODEL_MEDIA_TYPES, Model, ModelMediaKind, applied_context_limit,
    attachment_tag, context_limits, effective_context_window, file_extension_support, is_blank,
    model_accepts_media_type, model_accepts_video, preview_media_type, shows_in_place,
    model_media_type_for, trim,
};
use serde_json::Value;

/// The cases of `fixtures/file-types.json`, which the page's generated
/// lookups are checked with too.
fn file_type_cases(lookup: &str) -> Vec<(String, Value)> {
    let cases: Value = serde_json::from_str(include_str!("fixtures/file-types.json")).unwrap();
    cases[lookup]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| (case[0].as_str().unwrap().to_owned(), case[1].clone()))
        .collect()
}

#[test]
fn a_file_is_known_by_its_extension_whatever_its_case_or_separator() {
    // Among the cases: a leading dot names a hidden file, not an extension,
    // and only ASCII letters fold, so the Kelvin sign in `mar\u{212a}down`
    // names no type.
    for (path, expected) in file_type_cases("previewMediaType") {
        assert_eq!(preview_media_type(&path), expected.as_str(), "{path}");
    }
}

#[test]
fn the_page_shows_media_in_place_and_renders_markdown_from_its_text() {
    for (media_type, expected) in file_type_cases("showsInPlace") {
        assert_eq!(
            shows_in_place(&media_type),
            expected.as_bool().unwrap(),
            "{media_type}"
        );
    }
}

#[test]
fn every_medium_a_model_reads_is_served_for_the_page_to_show_in_place() {
    // A tool's images and videos, and a message's, are known by the
    // model-media table and served from their blobs by the file-type table:
    // a type the second does not show in place downloads, and a player then
    // plays it only where the user's browser guesses the format.
    for entry in MODEL_MEDIA_TYPES {
        assert!(shows_in_place(entry.media_type), "{}", entry.media_type);
    }
}

#[test]
fn each_model_medium_names_its_kind_and_the_extension_a_catalog_accepts_it_by() {
    let png = model_media_type_for("image/png").unwrap();
    assert_eq!(
        (png.kind, png.extension),
        (ModelMediaKind::Image, FileExtension::Png)
    );
    assert!(model_media_type_for("application/pdf").is_none());
}

fn model(accepted: Option<&[FileExtension]>) -> Model {
    Model {
        id: "m".into(),
        name: "M".into(),
        context_window: 1,
        output_limit: None,
        thinking: Vec::new(),
        accepted_extensions: accepted.map(<[FileExtension]>::to_vec),
    }
}

#[test]
fn a_model_reads_media_its_catalog_accepts_and_unknown_support_counts_as_no() {
    use FileExtension::*;
    let images = model(Some(&[Png, Jpg, Jpeg, Gif, Webp]));
    assert!(model_accepts_media_type(&images, "image/png"));
    assert!(!model_accepts_media_type(&images, "video/mp4"));
    assert!(!model_accepts_media_type(&images, "application/pdf"));
    assert!(!model_accepts_video(&images));

    let video = model(Some(&[Png, Mp4, Mov, Webm, M4v]));
    assert!(model_accepts_media_type(&video, "video/mp4"));
    assert!(model_accepts_media_type(&video, "video/quicktime"));
    assert!(model_accepts_video(&video));

    // A jpg-only catalog still accepts image/jpeg.
    assert!(model_accepts_media_type(&model(Some(&[Jpg])), "image/jpeg"));
    assert!(!model_accepts_media_type(&model(Some(&[])), "image/png"));
    assert!(!model_accepts_media_type(&model(None), "image/png"));
    assert!(!model_accepts_video(&model(None)));
}

#[test]
fn extension_support_tells_unknown_unsupported_and_supported_apart() {
    use FileExtension::*;
    assert_eq!(file_extension_support(None, Png), None);
    assert_eq!(file_extension_support(Some(&[]), Png), Some(false));
    assert_eq!(file_extension_support(Some(&[Png]), Png), Some(true));
    assert_eq!(file_extension_support(Some(&[Png]), Pdf), Some(false));
    assert_eq!(file_extension_support(Some(&[Jpeg]), Jpg), Some(true));
    assert_eq!(file_extension_support(Some(&[Jpg]), Jpeg), Some(true));
}

#[test]
fn an_attachment_is_one_self_closing_tag_with_escaped_attributes() {
    let attachment = Attachment {
        name: r#"notes "v2" <&>.md"#.into(),
        path: r#"/home/demi/.demi/attachments/c1/notes "v2" <&>.md"#.into(),
        media_type: "text/markdown".into(),
        size_bytes: 82,
        sha256: "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
            .parse()
            .unwrap(),
        snippet: Some("never rendered".into()),
    };
    assert_eq!(
        attachment_tag(&attachment),
        r#"<attachment name="notes &quot;v2&quot; &lt;&amp;&gt;.md" type="text/markdown" size="82" path="/home/demi/.demi/attachments/c1/notes &quot;v2&quot; &lt;&amp;&gt;.md"/>"#,
    );
}

#[test]
fn blank_text_is_white_space_as_javascript_trims_it() {
    assert!(is_blank(""));
    assert!(is_blank(" \t\r\n"));
    assert!(is_blank("\u{feff}\u{3000}\u{a0}"));
    assert!(!is_blank("\u{85}"));
    assert!(!is_blank(" a "));
}

#[test]
fn a_trim_removes_the_white_space_javascript_trims() {
    assert_eq!(trim("\u{feff}  New name \t\r\n\u{3000}"), "New name");
    assert_eq!(trim("\u{85}name\u{85}"), "\u{85}name\u{85}");
    assert_eq!(trim(" \t "), "");
}

/// The cases of `fixtures/context-limits.json`, which the page's generated
/// lookups are checked with too.
fn context_limit_cases(lookup: &str) -> Vec<Value> {
    let cases: Value = serde_json::from_str(include_str!("fixtures/context-limits.json")).unwrap();
    cases[lookup].as_array().unwrap().clone()
}

/// A case's number, or none for null.
fn tokens(value: &Value) -> Option<u32> {
    value.as_u64().map(|tokens| u32::try_from(tokens).unwrap())
}

#[test]
fn a_window_over_500k_offers_300k_and_200k_and_one_of_1m_also_500k() {
    // Among the cases: a window of exactly 500,000 tokens offers nothing,
    // nor does an unknown one.
    for case in context_limit_cases("contextLimits") {
        let window = tokens(&case[0]).unwrap();
        let expected: Vec<u32> = case[1]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(tokens)
            .collect();
        assert_eq!(
            context_limits(window).collect::<Vec<_>>(),
            expected,
            "{window}"
        );
    }
}

#[test]
fn a_stored_limit_applies_only_while_the_window_offers_it() {
    for case in context_limit_cases("appliedContextLimit") {
        let (window, limit) = (tokens(&case[0]).unwrap(), tokens(&case[1]));
        assert_eq!(
            applied_context_limit(window, limit),
            tokens(&case[2]),
            "{window} {limit:?}"
        );
        assert_eq!(
            effective_context_window(window, limit),
            tokens(&case[2]).unwrap_or(window)
        );
    }
}
