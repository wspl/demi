//! The two-step decode of vendor payloads tagged by `type`
//! (`crates/provider-common/src/wire/mod.rs`).

use demi_provider_common::{
    tagged_wire,
    wire::{Tagged, decode_tagged},
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
    let start =
        decode_tagged::<Event>(r#"{"type":"start","index":2,"block":{"type":"text","text":"hi"}}"#);
    let expected = Start {
        index: 2,
        block: Tagged(Some(Block::Text(Text { text: "hi".into() }))),
    };
    assert_eq!(start.unwrap(), Some(Event::Start(expected)));
    let unknown_block =
        decode_tagged::<Event>(r#"{"type":"start","index":0,"block":{"type":"image"}}"#);
    let expected = Start {
        index: 0,
        block: Tagged(None),
    };
    assert_eq!(unknown_block.unwrap(), Some(Event::Start(expected)));
}
