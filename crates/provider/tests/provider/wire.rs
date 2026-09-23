//! The two-step decode of vendor payloads tagged by `type`
//! (`providers.md` § Reading vendor input).

use demi_provider::{
    tagged_wire,
    wire::{ReportedString, Tagged, decode_tagged},
};
use serde::Deserialize;

#[derive(Debug, PartialEq, Deserialize)]
struct Start {
    index: u32,
    block: Tagged<Block>,
}

#[derive(Debug, PartialEq, Deserialize)]
struct Stop {}

tagged_wire! {
    #[derive(Debug, PartialEq)]
    enum Event {
        "start" => Start(Start),
        "stop" => Stop(Stop),
    }
}

#[derive(Debug, PartialEq, Deserialize)]
struct Text {
    text: String,
}

tagged_wire! {
    #[derive(Debug, PartialEq)]
    enum Block {
        "text" => Text(Text),
    }
}

#[test]
fn an_unregistered_tag_is_skipped_and_a_registered_one_decodes() {
    assert_eq!(decode_tagged::<Event>(r#"{"type":"ping"}"#).unwrap(), None);
    assert_eq!(
        decode_tagged::<Event>(r#"{"type":"stop","vendor_field":1}"#).unwrap(),
        Some(Event::Stop(Stop {}))
    );
    let start = decode_tagged::<Event>(r#"{"type":"start","index":2,"block":{"type":"text","text":"hi"}}"#);
    let expected = Start {
        index: 2,
        block: Tagged(Some(Block::Text(Text { text: "hi".into() }))),
    };
    assert_eq!(start.unwrap(), Some(Event::Start(expected)));
    let unknown_block = decode_tagged::<Event>(r#"{"type":"start","index":0,"block":{"type":"image"}}"#);
    let expected = Start { index: 0, block: Tagged(None) };
    assert_eq!(unknown_block.unwrap(), Some(Event::Start(expected)));
}

#[test]
fn a_malformed_registered_payload_is_an_error_that_names_its_field() {
    let missing = decode_tagged::<Event>(r#"{"type":"start","block":{"type":"text","text":"hi"}}"#).unwrap_err();
    assert!(missing.to_string().contains("index"), "{missing}");
    let nested = decode_tagged::<Event>(r#"{"type":"start","index":0,"block":{"type":"text","text":42}}"#).unwrap_err();
    assert_eq!(nested.path(), "block");
    assert!(nested.to_string().starts_with("block: text: invalid type"), "{nested}");
}

#[test]
fn a_payload_without_a_tag_or_that_is_not_json_is_an_error() {
    for text in [
        "{}",
        r#"{"type":5}"#,
        "not json",
        "",
        r#"{"type":"start","index":0,"block":{"text":"hi"}}"#,
    ] {
        assert!(decode_tagged::<Event>(text).is_err(), "{text}");
    }
}

#[test]
fn a_reported_field_reads_anything_but_a_string_as_absent_and_a_token_count_is_whole() {
    #[derive(Deserialize)]
    struct Report {
        #[serde(default)]
        message: ReportedString,
        #[serde(default)]
        tokens: Option<u64>,
    }
    let report: Report = serde_json::from_str(r#"{"message":{"nested":true},"tokens":null}"#).unwrap();
    assert_eq!((report.message.into_inner(), report.tokens), (None, None));
    let report: Report = serde_json::from_str(r#"{"message":"slow down","tokens":12}"#).unwrap();
    assert_eq!((report.message.into_inner(), report.tokens), (Some("slow down".into()), Some(12)));
    for refused in [r#"{"tokens":12.5}"#, r#"{"tokens":"10"}"#, r#"{"tokens":-1}"#] {
        assert!(serde_json::from_str::<Report>(refused).is_err(), "{refused}");
    }
}
