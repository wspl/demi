//! Failure codes, the HTTP failure record and its standard reading.

use demi_provider_common::{
    ErrorCode, retry_at,
};
use demi_shared_types::Timestamp;

fn time(text: &str) -> Timestamp {
    text.parse().unwrap()
}

#[test]
fn a_vendor_failure_classifies_by_whole_words_and_falls_back_to_the_vendor_code() {
    let cases = [
        (
            None,
            "maximum context length",
            Some(ErrorCode::ContextLengthExceeded),
        ),
        (
            None,
            "max_tokens: 64000 > 32000 tokens",
            Some(ErrorCode::ContextLengthExceeded),
        ),
        (
            Some("request_too_large"),
            "Request exceeds the maximum size",
            Some(ErrorCode::RequestTooLarge),
        ),
        (
            None,
            "payload too large",
            Some(ErrorCode::RequestTooLarge),
        ),
        (
            Some("invalid_request_error"),
            "Too many images in request",
            Some(ErrorCode::RequestTooLarge),
        ),
        (
            Some("invalid_request_error"),
            "image dimensions exceed max allowed size for many-image requests",
            Some(ErrorCode::RequestTooLarge),
        ),
        (
            None,
            "the input is too large",
            Some(ErrorCode::ContextLengthExceeded),
        ),
        (None, "rate limit reached", Some(ErrorCode::RateLimit)),
        (
            Some("rate_limit_error"),
            "Number of requests exceeded",
            Some(ErrorCode::RateLimit),
        ),
        // Used-up credits are a usage limit, which no wait fixes; a quota
        // throttles, as Gemini's per-minute one does.
        (None, "insufficient balance", Some(ErrorCode::UsageLimit)),
        (
            Some("RESOURCE_EXHAUSTED"),
            "Quota exceeded for metric generate_content_requests per minute",
            Some(ErrorCode::RateLimit),
        ),
        (
            Some("billing_error"),
            "Your credit balance is too low",
            Some(ErrorCode::UsageLimit),
        ),
        (
            None,
            "You ran out of credits",
            Some(ErrorCode::UsageLimit),
        ),
        // A vendor's own usage-limit code is its provider's to read; its
        // words alone decide nothing.
        (
            Some("usage_limit_reached"),
            "The usage limit has been reached",
            Some(ErrorCode::Vendor("usage_limit_reached".into())),
        ),
        (None, "invalid api key", Some(ErrorCode::AuthExpired)),
        (
            Some("authentication_error"),
            "invalid x-api-key",
            Some(ErrorCode::AuthExpired),
        ),
        (None, "service unavailable", Some(ErrorCode::Overloaded)),
        (
            Some("overloaded_error"),
            "Overloaded",
            Some(ErrorCode::Overloaded),
        ),
        (
            Some("server_error"),
            "backend failed",
            Some(ErrorCode::Overloaded),
        ),
        (
            Some("internal-error"),
            "backend failed",
            Some(ErrorCode::Overloaded),
        ),
        (
            Some("api_error"),
            "Internal server error",
            Some(ErrorCode::Overloaded),
        ),
        (
            None,
            "Codex SSE response headers timed out after 20000ms",
            Some(ErrorCode::Overloaded),
        ),
        (None, "connect timeout", Some(ErrorCode::Overloaded)),
        (None, "socket hang up", Some(ErrorCode::Overloaded)),
        (None, "read ECONNRESET", Some(ErrorCode::Overloaded)),
        // A vendor code stands when no word decides.
        (
            Some("invalid_request_error"),
            "Invalid prompt_cache_key",
            Some(ErrorCode::Vendor("invalid_request_error".into())),
        ),
        (
            Some("custom"),
            "something else",
            Some(ErrorCode::Vendor("custom".into())),
        ),
        // Words inside other words and a bare "limit" decide nothing.
        (
            Some("invalid_request_error"),
            "Could not generate the schema",
            Some(ErrorCode::Vendor("invalid_request_error".into())),
        ),
        (
            Some("invalid_request_error"),
            "tools: exceeds the limit of 128",
            Some(ErrorCode::Vendor("invalid_request_error".into())),
        ),
        (None, "iterate over the unlimited list", None),
        // `usage` and `usage limit` decide nothing: the Claude Code CLI's
        // throttled request says it is not one.
        (
            Some("invalid_request_error"),
            "Invalid usage of the tools parameter",
            Some(ErrorCode::Vendor("invalid_request_error".into())),
        ),
        (None, "usage limit exceeded for this month", None),
        (
            None,
            "Server is temporarily limiting requests (not your usage limit)",
            None,
        ),
        (Some(""), "nothing to see", None),
    ];
    for (code, message, expected) in cases {
        assert_eq!(
            ErrorCode::classify(code, message),
            expected,
            "{code:?} {message}"
        );
    }
}

#[test]
fn retry_after_names_seconds_after_receipt_or_an_http_date_and_nothing_else() {
    let received = time("2026-09-18T14:00:00.000Z");
    let named = [
        ("120", "2026-09-18T14:02:00.000Z"),
        (" 120\t", "2026-09-18T14:02:00.000Z"),
        ("0", "2026-09-18T14:00:00.000Z"),
        ("1.5", "2026-09-18T14:00:01.500Z"),
        ("1.2349", "2026-09-18T14:00:01.234Z"),
        ("Tue, 22 Sep 2026 07:37:39 GMT", "2026-09-22T07:37:39.000Z"),
        (
            "Tuesday, 22-Sep-26 07:37:39 GMT",
            "2026-09-22T07:37:39.000Z",
        ),
        ("Tue Sep 22 07:37:39 2026", "2026-09-22T07:37:39.000Z"),
    ];
    for (value, moment) in named {
        assert_eq!(retry_at(value, received), Some(time(moment)), "{value}");
    }
    for value in [
        "1e3",
        "0x10",
        "2026-09-22T07:37:39Z",
        "-5",
        "+5",
        "soon",
        "",
        "1.",
        ".5",
        "99999999999999999999",
    ] {
        assert_eq!(retry_at(value, received), None, "{value}");
    }
}
