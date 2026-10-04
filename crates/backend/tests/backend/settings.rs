//! The instance's settings, each user's preferences and each user's
//! subagent settings (`web-api.md` § User preferences, § Subagents).

use demi_provider_common::testing::MockVendor;
use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::settings::{InstanceMode, Settings, UserPreferences};
use demi_web_api_protocol::state::SyncEvent;
use demi_web_api_protocol::subagents::{ProfileAnswer, SubagentSettings};
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{entry, leveled};
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
        assert_eq!(
            backend
                .get("/api/settings", Some(&master))
                .await
                .json::<Settings>(),
            Settings { mode }
        );
        let anonymous = backend.get("/api/settings", None).await;
        assert_eq!(
            anonymous.refusal(),
            (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated)
        );
        backend.close().await;
    }
}

#[tokio::test]
async fn preference_patches_merge_field_by_field_refuse_what_is_invalid_and_survive_a_restart() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let device = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    assert_eq!(
        saved(&backend, &master).await,
        json!({ "preferences": { "appearance": {}, "shortcuts": {} } })
    );

    let last_model = json!({
        "providerId": "codex-account",
        "modelId": "chosen-model",
        "thinkingEffort": "high",
        "serviceTierId": null,
    });
    let project_host = json!({ "kind": "device", "deviceId": "laptop" });
    let (theme, shortcut, font, model, host) = tokio::join!(
        backend.patch(
            PREFERENCES,
            &master,
            json!({ "appearance": { "theme": "dark" } })
        ),
        backend.patch(
            PREFERENCES,
            &device,
            json!({ "shortcuts": { "new": "⌘⇧N" } })
        ),
        backend.patch(
            PREFERENCES,
            &device,
            json!({ "appearance": { "fontSize": 17 } })
        ),
        backend.patch(PREFERENCES, &device, json!({ "lastModel": last_model })),
        backend.patch(
            PREFERENCES,
            &master,
            json!({ "lastProjectHost": project_host })
        ),
    );
    for answer in [theme, shortcut, font, model, host] {
        assert_eq!(answer.status, StatusCode::OK);
    }
    let expected = json!({ "preferences": {
        "appearance": { "theme": "dark", "fontSize": 17 },
        "shortcuts": { "new": "⌘⇧N" },
        "lastModel": last_model,
        "lastProjectHost": project_host,
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
        json!({ "lastProjectHost": { "kind": "laptop" } }),
        json!({ "contextLimit": { "providerId": "codex-account", "modelId": "chosen-model", "tokens": 400000 } }),
        json!({ "remember": true }),
    ] {
        let answer = backend.patch(PREFERENCES, &device, refused.clone()).await;
        assert_eq!(
            answer.refusal(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
            "{refused}"
        );
    }

    backend.close().await;
    let backend = harness.start().await;
    assert_eq!(saved(&backend, &master).await, expected);
    // A null shortcut removes that override and leaves the rest.
    let removed = backend
        .patch(
            PREFERENCES,
            &master,
            json!({ "shortcuts": { "new": null } }),
        )
        .await;
    assert_eq!(
        serde_json::to_value(removed.json::<UserPreferences>()).unwrap(),
        json!({ "preferences": {
            "appearance": { "theme": "dark", "fontSize": 17 },
            "shortcuts": {},
            "lastModel": last_model,
            "lastProjectHost": project_host,
        } })
    );
    backend.close().await;
}

#[tokio::test]
async fn a_reported_locale_is_checked_and_kept_in_canonical_form() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    for (locale, field) in [
        (
            json!({ "timeZone": "Mars/Olympus_Mons", "languages": ["en"] }),
            "locale.timeZone",
        ),
        (
            json!({ "timeZone": "UTC", "languages": ["en", "en_US"] }),
            "locale.languages[1]",
        ),
        (json!({ "timeZone": "UTC", "languages": [] }), "languages"),
        (
            json!({ "timeZone": "UTC", "languages": vec!["en"; 17] }),
            "languages",
        ),
        (json!({ "timeZone": "UTC" }), "languages"),
    ] {
        let answer = backend
            .patch(PREFERENCES, &master, json!({ "locale": locale }))
            .await;
        let error = answer.error();
        assert_eq!(
            (answer.status, error.code),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
            "{locale}"
        );
        assert!(error.message.contains(field), "{locale}: {}", error.message);
    }

    let reported =
        json!({ "timeZone": "asia/shanghai", "languages": ["zh-cn", "EN", "zh-CN", "iw"] });
    let answer = backend
        .patch(PREFERENCES, &master, json!({ "locale": reported }))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        serde_json::to_value(answer.json::<UserPreferences>().preferences.locale).unwrap(),
        json!({ "timeZone": "Asia/Shanghai", "languages": ["zh-CN", "en", "he"] })
    );
    backend.close().await;
}

const PROFILES: &str = "/api/subagents/profiles";

/// The subagent settings the next `subagents` message of `page` carries.
async fn synced(page: &mut crate::support::SyncChannel) -> SubagentSettings {
    let events = page
        .until(|event| matches!(event, SyncEvent::Subagents { .. }))
        .await;
    match events.last() {
        Some(SyncEvent::Subagents { subagents }) => subagents.clone(),
        other => panic!("no subagents message: {other:?}"),
    }
}

#[tokio::test]
async fn a_users_subagent_profiles_are_checked_written_and_reach_every_page_of_the_user() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let body = json!({
        "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
        "baseUrl": vendor.url("/v1"), "models": [leveled("m", Some("priority"))]
    });
    let provider = entry(&backend, &master, body).await;
    let mut page = backend.sync(&master).await;
    page.snapshot().await;

    // A model that names no effort is stored with the model's first.
    let explore = json!({
        "name": "explore", "description": "Finds code; it changes nothing.",
        "model": { "providerId": provider, "modelId": "m", "thinkingEffort": null, "serviceTierId": null },
        "instructions": "Report, never edit.", "canSpawn": false
    });
    let created = backend.post(PROFILES, Some(&master), explore.clone()).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let profile = created.json::<ProfileAnswer>().profile;
    assert_eq!(
        serde_json::to_value(&profile).unwrap(),
        json!({
            "id": profile.id, "name": "explore", "description": "Finds code; it changes nothing.",
            "model": { "providerId": provider, "modelId": "m", "thinkingEffort": "low", "serviceTierId": null },
            "instructions": "Report, never edit.", "canSpawn": false, "enabled": true
        })
    );
    assert_eq!(synced(&mut page).await.profiles, [profile.clone()]);

    // Each body is checked before anything is written.
    let with = |field: &str, value: Value| {
        let mut body = explore.clone();
        body["name"] = json!("other");
        body[field] = value;
        body
    };
    let refusals = [
        (explore.clone(), StatusCode::CONFLICT, ErrorCode::ProfileExists),
        (with("name", json!("default")), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        (with("name", json!("Explore")), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        (with("name", json!("1st")), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        (with("name", json!("a".repeat(41))), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        (with("description", json!("two\nlines")), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        (with("description", json!(" ")), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        (with("instructions", json!("  ")), StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        (
            with("model", json!({ "providerId": provider, "modelId": "m", "thinkingEffort": "max", "serviceTierId": null })),
            StatusCode::CONFLICT,
            ErrorCode::SettingUnavailable,
        ),
        (
            with("model", json!({ "providerId": provider, "modelId": "x", "thinkingEffort": null, "serviceTierId": null })),
            StatusCode::NOT_FOUND,
            ErrorCode::ModelNotFound,
        ),
        (
            with("model", json!({ "providerId": "gone", "modelId": "m", "thinkingEffort": null, "serviceTierId": null })),
            StatusCode::NOT_FOUND,
            ErrorCode::ProviderNotFound,
        ),
    ];
    for (body, status, code) in refusals {
        let refused = backend.post(PROFILES, Some(&master), body.clone()).await;
        assert_eq!(refused.refusal(), (status, code), "{body}");
    }

    // A patch changes the fields it names; null returns to the parent's.
    let path = format!("{PROFILES}/{}", profile.id);
    let patched = backend
        .patch(
            &path,
            &master,
            json!({ "name": "finder", "model": null, "instructions": null, "canSpawn": true }),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK);
    let finder = patched.json::<ProfileAnswer>().profile;
    assert_eq!(
        (finder.name.as_str(), &finder.model, &finder.instructions, finder.can_spawn),
        ("finder", &None, &None, true)
    );
    assert_eq!(finder.description, profile.description);
    assert_eq!(synced(&mut page).await.profiles, [finder.clone()]);
    let reviewer = backend
        .post(PROFILES, Some(&master), with("name", json!("reviewer")))
        .await
        .json::<ProfileAnswer>()
        .profile;
    synced(&mut page).await;
    let taken = backend.patch(&path, &master, json!({ "name": "reviewer" })).await;
    assert_eq!(taken.refusal(), (StatusCode::CONFLICT, ErrorCode::ProfileExists));

    // With its entry gone the reviewer is unavailable, and is still turned
    // off and on as it is.
    let deleted = backend
        .delete(&format!("/api/providers/{provider}"), &master)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let reviewer_path = format!("{PROFILES}/{}", reviewer.id);
    for enabled in [false, true] {
        let switched = backend
            .patch(&reviewer_path, &master, json!({ "enabled": enabled }))
            .await;
        assert_eq!(switched.status, StatusCode::OK);
        assert_eq!(switched.json::<ProfileAnswer>().profile.enabled, enabled);
    }

    // The switch, in the product state.
    let off = backend.put("/api/subagents", &master, json!({ "enabled": false })).await;
    assert_eq!(off.status, StatusCode::NO_CONTENT);
    let settings = page
        .until(|event| matches!(event, SyncEvent::Subagents { subagents } if !subagents.enabled))
        .await;
    assert!(!settings.is_empty());

    // Another user sees and reaches none of them.
    harness.add_user("ana@example.test", "correct horse battery", Role::User);
    let ana = backend.login("ana@example.test", "correct horse battery").await;
    let foreign = backend.patch(&path, &ana, json!({ "enabled": false })).await;
    assert_eq!(foreign.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ProfileNotFound));
    let mut ana_page = backend.sync(&ana).await;
    let state = ana_page.snapshot().await;
    assert_eq!(
        state.subagents,
        SubagentSettings {
            enabled: true,
            profiles: Vec::new()
        }
    );

    let removed = backend.delete(&path, &master).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let again = backend.delete(&path, &master).await;
    assert_eq!(again.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ProfileNotFound));
    let mut fresh = backend.sync(&master).await;
    let state = fresh.snapshot().await;
    assert!(!state.subagents.enabled);
    assert_eq!(state.subagents.profiles.len(), 1);
    assert_eq!(state.subagents.profiles[0].name, "reviewer");
    backend.close().await;
}
