//! The instance's settings and each user's preferences (`web-api.md` §
//! User preferences).

use demi_web_api::error::ErrorCode;
use demi_web_api::settings::{InstanceMode, Settings, UserPreferences};
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD, Session, TestBackend};

const PREFERENCES: &str = "/api/settings/preferences";

async fn saved(backend: &TestBackend, session: &Session) -> Value {
    let answer = backend.get(PREFERENCES, Some(session)).await;
    assert_eq!(answer.status, StatusCode::OK);
    serde_json::to_value(answer.json::<UserPreferences>()).unwrap()
}

#[tokio::test]
async fn the_instance_mode_is_read_back_as_configured() {
    for mode in [InstanceMode::Shared, InstanceMode::Isolated] {
        let harness = Harness::new().with_mode(mode);
        let (backend, master) = harness.start_set_up().await;
        assert_eq!(backend.get("/api/settings", Some(&master)).await.json::<Settings>(), Settings { mode });
        let anonymous = backend.get("/api/settings", None).await;
        assert_eq!(anonymous.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated));
        backend.close().await;
    }
}

#[tokio::test]
async fn preference_patches_merge_field_by_field_refuse_what_is_invalid_and_survive_a_restart() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let device = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    assert_eq!(saved(&backend, &master).await, json!({ "preferences": { "appearance": {}, "shortcuts": {} } }));

    let last_model = json!({
        "providerId": "codex-account",
        "modelId": "chosen-model",
        "thinkingEffort": "high",
        "serviceTierId": null,
    });
    let (theme, shortcut, font, model) = tokio::join!(
        backend.patch(PREFERENCES, &master, json!({ "appearance": { "theme": "dark" } })),
        backend.patch(PREFERENCES, &device, json!({ "shortcuts": { "new": "⌘⇧N" } })),
        backend.patch(PREFERENCES, &device, json!({ "appearance": { "fontSize": 17 } })),
        backend.patch(PREFERENCES, &device, json!({ "lastModel": last_model })),
    );
    for answer in [theme, shortcut, font, model] {
        assert_eq!(answer.status, StatusCode::OK);
    }
    let expected = json!({ "preferences": {
        "appearance": { "theme": "dark", "fontSize": 17 },
        "shortcuts": { "new": "⌘⇧N" },
        "lastModel": last_model,
    } });
    assert_eq!(saved(&backend, &device).await, expected);

    for refused in [
        json!({ "appearance": { "fontSize": 100 } }),
        json!({ "appearance": { "theme": "sepia" } }),
        json!({ "appearance": null }),
        json!({ "shortcuts": { "arbitrary": "x" } }),
        json!({ "shortcuts": { "new": "x".repeat(65) } }),
        json!({ "lastModel": { "providerId": "", "modelId": "chosen-model", "thinkingEffort": null, "serviceTierId": null } }),
        json!({ "lastModel": { "providerId": "codex-account", "modelId": "chosen-model" } }),
        json!({ "remember": true }),
    ] {
        let answer = backend.patch(PREFERENCES, &device, refused.clone()).await;
        assert_eq!(answer.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody), "{refused}");
    }

    backend.close().await;
    let backend = harness.start().await;
    assert_eq!(saved(&backend, &master).await, expected);
    // A null shortcut removes that override and leaves the rest.
    let removed = backend.patch(PREFERENCES, &master, json!({ "shortcuts": { "new": null } })).await;
    assert_eq!(
        serde_json::to_value(removed.json::<UserPreferences>()).unwrap(),
        json!({ "preferences": {
            "appearance": { "theme": "dark", "fontSize": 17 },
            "shortcuts": {},
            "lastModel": last_model,
        } })
    );
    backend.close().await;
}

#[tokio::test]
async fn a_reported_locale_is_checked_and_kept_in_canonical_form() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    for (locale, field) in [
        (json!({ "timeZone": "Mars/Olympus_Mons", "languages": ["en"] }), "locale.timeZone"),
        (json!({ "timeZone": "UTC", "languages": ["en", "en_US"] }), "locale.languages[1]"),
        (json!({ "timeZone": "UTC", "languages": [] }), "languages"),
        (json!({ "timeZone": "UTC", "languages": vec!["en"; 17] }), "languages"),
        (json!({ "timeZone": "UTC" }), "languages"),
    ] {
        let answer = backend.patch(PREFERENCES, &master, json!({ "locale": locale })).await;
        let error = answer.error();
        assert_eq!((answer.status, error.code), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody), "{locale}");
        assert!(error.message.contains(field), "{locale}: {}", error.message);
    }

    let reported = json!({ "timeZone": "asia/shanghai", "languages": ["zh-cn", "EN", "zh-CN", "iw"] });
    let answer = backend.patch(PREFERENCES, &master, json!({ "locale": reported })).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        serde_json::to_value(answer.json::<UserPreferences>().preferences.locale).unwrap(),
        json!({ "timeZone": "Asia/Shanghai", "languages": ["zh-CN", "en", "he"] })
    );
    backend.close().await;
}
