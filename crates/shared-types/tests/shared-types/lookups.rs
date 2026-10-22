//! The lookups the backend and the web app share: which files the page shows
//! and how, which media a model reads, how an attachment is named to the
//! model, and which context limits a model offers.

use demi_shared_types::{
    FileExtension, Model, applied_context_limit, context_limits, effective_context_window,
    is_blank, model_accepts_media_type, model_accepts_video, preview_media_type, trim,
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
