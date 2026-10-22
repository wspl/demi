//! What every OAuth login and refresh reads
//! (`crates/provider-common/src/wire/mod.rs`), and the ids Demi derives for the
//! OpenAI-shaped formats.

use std::time::Duration;

use demi_provider_common::{
    oauth::{Lifetime, OAuthSeconds, PollInterval},
    openai_request::{prompt_cache_key, short_hash},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
struct Answer {
    #[serde(default)]
    interval: PollInterval,
    #[serde(default)]
    expires_in: Lifetime,
}

fn answer(value: Value) -> Answer {
    serde_json::from_value(value).unwrap()
}

#[test]
fn oauth_durations_are_numbers_or_digits_and_unusable_ones_have_a_meaning() {
    for (value, seconds) in [
        (json!(5), 5.0),
        (json!("28800"), 28_800.0),
        (json!(" 1.5 "), 1.5),
        (json!(0), 0.0),
    ] {
        let decoded: OAuthSeconds = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded, OAuthSeconds(seconds), "{value}");
    }
    for value in [
        json!("1e3"),
        json!("soon"),
        json!(-1),
        json!(true),
        json!(null),
        json!("-5"),
    ] {
        assert!(
            serde_json::from_value::<OAuthSeconds>(value.clone()).is_err(),
            "{value}"
        );
    }
    // Without a usable interval the client polls every 5 seconds.
    assert_eq!(
        answer(json!({})).interval,
        PollInterval(Duration::from_secs(5))
    );
    assert_eq!(
        answer(json!({ "interval": "soon" })).interval,
        PollInterval(Duration::from_secs(5))
    );
    assert_eq!(
        answer(json!({ "interval": "0" })).interval,
        PollInterval(Duration::ZERO)
    );
    assert_eq!(
        answer(json!({ "interval": 3 })).interval,
        PollInterval(Duration::from_secs(3))
    );
    // A lifetime that is not above zero reads as absent.
    assert_eq!(
        answer(json!({ "expires_in": "600" })).expires_in,
        Lifetime(Some(Duration::from_secs(600)))
    );
    for value in [json!(0), json!("forever"), json!(-3)] {
        assert_eq!(
            answer(json!({ "expires_in": value })).expires_in,
            Lifetime(None)
        );
    }
}

#[test]
fn a_long_session_id_becomes_a_hashed_cache_key_counted_in_utf16_units() {
    assert_eq!(short_hash(""), "811c9dc5");
    assert_eq!(short_hash("a"), "e40c292c");
    let at_limit = "x".repeat(64);
    assert_eq!(prompt_cache_key(&at_limit), at_limit.as_str());
    let long = format!("chat-{}", "x".repeat(80));
    assert_eq!(prompt_cache_key(&long), "session_cc30bf2");
    // 33 emoji are 66 UTF-16 units though 33 characters.
    let emoji = "😀".repeat(33);
    assert_eq!(prompt_cache_key(&emoji), "session_21a3538");
    let chinese = "会话".repeat(40);
    assert_eq!(prompt_cache_key(&chinese), "session_1bc4bf95");
}
