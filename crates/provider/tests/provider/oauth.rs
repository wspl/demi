//! What every OAuth login and refresh reads (`providers.md` § Reading vendor
//! input), and the ids Demi derives for the OpenAI-shaped formats.

use std::time::Duration;

use demi_provider::{
    oauth::{Lifetime, OAuthSeconds, PollInterval, jwt_claims},
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
    for (value, seconds) in [(json!(5), 5.0), (json!("28800"), 28_800.0), (json!(" 1.5 "), 1.5), (json!(0), 0.0)] {
        let decoded: OAuthSeconds = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded, OAuthSeconds(seconds), "{value}");
    }
    for value in [json!("1e3"), json!("soon"), json!(-1), json!(true), json!(null), json!("-5")] {
        assert!(serde_json::from_value::<OAuthSeconds>(value.clone()).is_err(), "{value}");
    }
    // Without a usable interval the client polls every 5 seconds.
    assert_eq!(answer(json!({})).interval, PollInterval(Duration::from_secs(5)));
    assert_eq!(answer(json!({ "interval": "soon" })).interval, PollInterval(Duration::from_secs(5)));
    assert_eq!(answer(json!({ "interval": "0" })).interval, PollInterval(Duration::ZERO));
    assert_eq!(answer(json!({ "interval": 3 })).interval, PollInterval(Duration::from_secs(3)));
    // A lifetime that is not above zero reads as absent.
    assert_eq!(answer(json!({ "expires_in": "600" })).expires_in, Lifetime(Some(Duration::from_secs(600))));
    for value in [json!(0), json!("forever"), json!(-3)] {
        assert_eq!(answer(json!({ "expires_in": value })).expires_in, Lifetime(None));
    }
}

/// A token whose payload is `claims`, in base64url without padding.
fn token(claims: &Value) -> String {
    use base64::Engine;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
    format!("header.{payload}.signature")
}

#[test]
fn a_tokens_claims_are_read_without_its_signature() {
    let claims: Option<Value> = jwt_claims(&token(&json!({ "sub": "user-1", "exp": 1_700_000_000 })));
    assert_eq!(claims, Some(json!({ "sub": "user-1", "exp": 1_700_000_000 })));
    // A payload that needs padding, with text beyond ASCII.
    let claims: Option<Value> = jwt_claims(&token(&json!({ "email": "zoé@example.com" })));
    assert_eq!(claims, Some(json!({ "email": "zoé@example.com" })));
    let padded = format!("header.{}==.signature", token(&json!({ "a": 1 })).split('.').nth(1).unwrap());
    assert_eq!(jwt_claims::<Value>(&padded), Some(json!({ "a": 1 })));
    for bad in ["header.payload", "", "a..c", "header.!!!!.signature", "a.b.c.d", "header.bm9wZQ.sig"] {
        assert_eq!(jwt_claims::<Value>(bad), None, "{bad}");
    }
    // The caller's type decides: a signature-less decode can yield any JSON.
    assert_eq!(jwt_claims::<Value>(&token(&json!(42))), Some(json!(42)));
    #[derive(Debug, Deserialize)]
    struct Claims {
        #[expect(dead_code, reason = "decoded to show that a number is not these claims")]
        sub: String,
    }
    assert!(jwt_claims::<Claims>(&token(&json!(42))).is_none());
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
