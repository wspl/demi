//! A plugin's OAuth sign-in at the edge (`plugins.md` § OAuth sign-in): the
//! vendor sends the browser back to the plugin's callback, a session route,
//! which completes only a sign-in the caller started and answers a page the
//! user reads; the user's pages learn the plugin's new state, and no secret
//! reaches them.

use demi_backend_plugins::testing::{Authority, AuthorizationServer};
use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::json;

use crate::support::{Answer, Harness};

const PLUGIN: &str = "fixture-sign-in";

fn page_text(answer: &Answer) -> String {
    assert_eq!(answer.headers["content-type"], "text/html; charset=utf-8");
    String::from_utf8(answer.body.clone()).unwrap()
}

#[tokio::test]
async fn the_callback_signs_in_only_the_user_who_started_and_no_secret_reaches_a_page() {
    let authority = Authority::new();
    let server = AuthorizationServer::start(&authority).await;
    let harness = Harness::new().with_sign_in_fixture(server.issuer(), authority.root());
    let (backend, master) = harness.start_set_up().await;
    harness.add_user("bob@example.test", "bob's password", Role::User);
    let bob = backend.login("bob@example.test", "bob's password").await;
    let mut page = backend.sync(&master).await;
    let state = page.snapshot().await;
    assert_eq!(
        state.plugin_states[PLUGIN],
        json!({ "signedIn": false, "keyEnding": null })
    );

    let started = backend
        .post(&format!("/api/plugins/{PLUGIN}/calls/sign_in"), Some(&master), json!({}))
        .await;
    assert_eq!(started.status, StatusCode::OK);
    let url = started.json::<serde_json::Value>()["url"]
        .as_str()
        .unwrap()
        .to_owned();
    let back = server.approve(&url).await;
    let callback = format!("{}?{}", back.path(), back.query().unwrap());
    assert_eq!(back.path(), format!("/api/plugins/{PLUGIN}/oauth/callback"));
    assert!(back.as_str().starts_with(&backend.url), "{back}");

    // Without a session, or with another user's, the sign-in is not
    // completed.
    assert_eq!(backend.get(&callback, None).await.status, StatusCode::UNAUTHORIZED);
    let other = backend.get(&callback, Some(&bob)).await;
    assert_eq!(other.status, StatusCode::BAD_REQUEST);
    assert!(page_text(&other).contains("started by another account"));

    let signed_in = backend.get(&callback, Some(&master)).await;
    assert_eq!(signed_in.status, StatusCode::OK);
    assert!(
        page_text(&signed_in).contains("Signed in to Example. You can close this tab."),
        "{}",
        page_text(&signed_in)
    );
    let changes = page
        .until(|event| matches!(event, SyncEvent::Plugin { plugin, .. } if plugin == PLUGIN))
        .await;
    let Some(SyncEvent::Plugin { state, .. }) = changes.last() else {
        unreachable!("the wait ends at the plugin's state")
    };
    assert_eq!(state["signedIn"], json!(true));

    // A state is used once; Bob is not signed in.
    let again = backend.get(&callback, Some(&master)).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    let bobs = backend.sync(&bob).await.snapshot().await;
    assert_eq!(bobs.plugin_states[PLUGIN]["signedIn"], json!(false));

    // A key the plugin keeps shows only as its last four characters; the
    // product state holds neither it nor a token.
    let saved = backend
        .post(
            &format!("/api/plugins/{PLUGIN}/calls/save_key"),
            Some(&master),
            json!({ "key": "sk-live-abcd1234" }),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK);
    let product = serde_json::to_string(&backend.sync(&master).await.snapshot().await).unwrap();
    assert!(product.contains(r#""keyEnding":"1234""#), "{product}");
    for secret in ["sk-live-abcd1234", "access-", "refresh-", "client-"] {
        assert!(!product.contains(secret), "the product state holds {secret}");
    }

    // An unknown plugin has no callback.
    let unknown = backend
        .get("/api/plugins/nothing/oauth/callback?state=x&code=y", Some(&master))
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    backend.close().await;
}
