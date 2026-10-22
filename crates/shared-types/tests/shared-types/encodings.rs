//! The JSON forms every contract shares: bytes, times, identities, blob
//! references and completion ids.

use demi_shared_types::{CompletionId, Timestamp};
use serde_json::json;

#[test]
fn a_time_is_written_in_utc_with_three_fractional_digits() {
    let time = Timestamp::from_millisecond(1_790_000_000_000).unwrap();
    assert_eq!(
        serde_json::to_value(time).unwrap(),
        json!("2026-09-21T14:13:20.000Z")
    );
    let later = Timestamp::from_millisecond(1_790_000_000_500).unwrap();
    assert_eq!(later.to_string(), "2026-09-21T14:13:20.500Z");
    // The text of two times orders as the times do.
    assert!(time.to_string() < later.to_string());

    for accepted in [
        "2026-09-21T14:13:20Z",
        "2026-09-21T14:13:20.000Z",
        "2026-09-21T16:13:20+02:00",
    ] {
        let decoded: Timestamp = serde_json::from_value(json!(accepted)).unwrap();
        assert_eq!(decoded, time, "{accepted}");
    }
    for refused in [
        json!("2026-09-21T14:13:20.0001Z"),
        json!("2026-09-21"),
        json!("yesterday"),
        json!(1_790_000_000_000_i64),
    ] {
        assert!(
            serde_json::from_value::<Timestamp>(refused.clone()).is_err(),
            "{refused}"
        );
    }
    let precise = jiff::Timestamp::from_nanosecond(1_790_000_000_000_999_999).unwrap();
    assert_eq!(Timestamp::truncate(precise), time);
}

#[test]
fn a_completion_id_names_one_round_of_one_child() {
    let id = CompletionId {
        child: "child:1".parse().unwrap(),
        round: 1_790_000_000_000,
    };
    assert_eq!(id.to_string(), "subagent:child:1:1790000000000");
    assert_eq!(id.block_id().as_str(), "subagent:child:1:1790000000000");
    assert_eq!(
        "subagent:child:1:1790000000000"
            .parse::<CompletionId>()
            .unwrap(),
        id
    );
    for refused in [
        "subagent:child",
        "subagent::5",
        "subagent:child:",
        "subagent:child:x5",
        "subagent:child:+5",
        "agent:child:5",
        "subagent:child:9007199254740992",
    ] {
        assert!(refused.parse::<CompletionId>().is_err(), "{refused}");
    }
}
