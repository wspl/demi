//! The usage ledger's totals (`usage-and-quota.md` § Usage ledger): one
//! group per entry and model in the order each pair was first used, each
//! user's own, and on a shared instance every account's for an
//! administrator. The metered runtime that writes the rows is the usage
//! module's own test; here the rows are written to the ledger directly.

use demi_web_api::auth::Role;
use demi_web_api::error::ErrorCode;
use demi_web_api::settings::InstanceMode;
use demi_web_api::usage::{InstanceUsage, UsageTotals};
use reqwest::StatusCode;

use crate::support::Harness;

/// A ledger row of `email`'s, `order` milliseconds into the day.
fn row(harness: &Harness, email: &str, provider: &str, model: &str, input: i64, order: i64) {
    harness
        .control_database()
        .execute(
            "INSERT INTO usage_ledger (id, user_id, conversation_id, provider_id, model_id,
               input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, created_at)
             SELECT ?1, id, '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b', ?2, ?3, ?4, 10, 2, 1, ?5 FROM users WHERE email = ?6",
            rusqlite::params![uuid::Uuid::new_v4().to_string(), provider, model, input, 1_790_000_000_000_i64 + order, email],
        )
        .unwrap();
}

#[tokio::test]
async fn totals_group_by_entry_and_model_in_first_use_order_and_the_instance_view_is_for_administrators() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    harness.add_user("admin@example.test", "admin-pass-1", Role::Admin);
    harness.add_user("bob@example.test", "bob-pass-1", Role::User);
    row(&harness, "master@example.test", "entry-b", "model-2", 100, 1);
    row(&harness, "master@example.test", "entry-a", "model-1", 200, 2);
    row(&harness, "master@example.test", "entry-b", "model-2", 300, 3);
    row(&harness, "bob@example.test", "entry-a", "model-1", 50, 4);

    let own = backend.get("/api/usage", Some(&master)).await.json::<UsageTotals>();
    let groups: Vec<(&str, &str, u64, u64, u64)> = own
        .totals
        .iter()
        .map(|group| {
            (
                group.provider_id.as_str(),
                group.model_id.as_str(),
                group.requests,
                group.input_tokens,
                group.output_tokens,
            )
        })
        .collect();
    assert_eq!(
        groups,
        [("entry-b", "model-2", 2, 400, 20), ("entry-a", "model-1", 1, 200, 10)]
    );
    assert_eq!(
        (own.totals[0].cache_read_tokens, own.totals[0].cache_write_tokens),
        (4, 2)
    );

    let bob = backend.login("bob@example.test", "bob-pass-1").await;
    assert_eq!(
        backend
            .get("/api/usage", Some(&bob))
            .await
            .json::<UsageTotals>()
            .totals
            .len(),
        1
    );
    let refused = backend.get("/api/usage/instance", Some(&bob)).await;
    assert_eq!(refused.refusal(), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));
    let admin = backend.login("admin@example.test", "admin-pass-1").await;
    let instance = backend
        .get("/api/usage/instance", Some(&admin))
        .await
        .json::<InstanceUsage>();
    let requests: Vec<(&str, u64)> = instance
        .users
        .iter()
        .map(|user| {
            (
                user.email.as_str(),
                user.totals.iter().map(|group| group.requests).sum(),
            )
        })
        .collect();
    assert_eq!(
        requests,
        [
            ("master@example.test", 3),
            ("admin@example.test", 0),
            ("bob@example.test", 1)
        ]
    );
    backend.close().await;

    // An isolated instance has no instance view.
    let backend = harness.start_in_mode(InstanceMode::Isolated).await;
    let master = backend
        .login(crate::support::MASTER_EMAIL, crate::support::MASTER_PASSWORD)
        .await;
    let refused = backend.get("/api/usage/instance", Some(&master)).await;
    assert_eq!(refused.refusal(), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));
    backend.close().await;
}
