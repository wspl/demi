//! Settings › Web Search: the state says how the user is connected and
//! which way searches go, and the methods sign in and out and save and
//! remove the key, which one search checks first.

use base64::Engine;
use demi_plugin_interface::PluginError;
use demi_plugin_web::{Account, InUse, SavedKey, WebState};
use serde_json::json;

use crate::exa::{Exa, FakeExa, pages};
use crate::support::Plugged;

fn local<T>(test: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(tokio::task::LocalSet::new().run_until(test))
}

/// A token whose claims name `email`, as Exa's may.
fn token_naming(email: &str) -> String {
    let encode = |value: serde_json::Value| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string())
    };
    format!(
        "{}.{}.signature",
        encode(json!({ "alg": "RS256" })),
        encode(json!({ "sub": "user-1", "email": email }))
    )
}

#[test]
fn the_state_follows_the_sign_in_and_the_key_and_names_the_way_in_use() {
    local(async {
        let plugged = Plugged::new(FakeExa::new(Exa {
            pages: pages(),
            key: Some("exa-example-abcd"),
            ..Exa::default()
        }));
        assert_eq!(
            plugged.state().await,
            WebState { account: None, key: None, in_use: InUse::Anonymous }
        );
        let started = plugged.call("sign_in", json!({})).await.unwrap();
        assert_eq!(started, json!({ "url": "https://auth.example.test/authorize?state=test" }));

        plugged.sign_in(&token_naming("alex@example.com"));
        let account = Some(Account { email: Some("alex@example.com".into()) });
        assert_eq!(
            plugged.state().await,
            WebState { account: account.clone(), key: None, in_use: InUse::Account }
        );

        // A key Exa refuses is not saved.
        let refused = plugged.call("save_key", json!({ "key": "wrong-key" })).await;
        assert!(
            matches!(&refused, Err(PluginError::Refused { reason, .. }) if reason == "key_refused"),
            "{refused:?}"
        );
        assert_eq!(plugged.saved_key(), None);

        let changes = plugged.demi.changes();
        plugged.call("save_key", json!({ "key": " exa-example-abcd\n" })).await.unwrap();
        assert_eq!(plugged.saved_key().as_deref(), Some("exa-example-abcd"));
        assert!(plugged.demi.changes() > changes);
        let key = Some(SavedKey { last_four: "abcd".into(), unchecked: None });
        assert_eq!(
            plugged.state().await,
            WebState { account: account.clone(), key: key.clone(), in_use: InUse::Key }
        );

        plugged.call("remove_key", json!({})).await.unwrap();
        assert_eq!(plugged.saved_key(), None);
        assert_eq!(plugged.state().await.in_use, InUse::Account);

        plugged.call("sign_out", json!({})).await.unwrap();
        assert_eq!(*plugged.demi.access_token.borrow(), None);
        assert_eq!(
            plugged.state().await,
            WebState { account: None, key: None, in_use: InUse::Anonymous }
        );
        // A sign-out is no expired sign-in: the next search is anonymous.
        let (code, _, stderr) = plugged.run(&["web", "search", "rolldown 1.0"]).await;
        assert_eq!((code, stderr.as_str()), (0, ""));
    });
}

/// A key Exa took but could not search with, at its rate limit or out of
/// credit, is saved with the reason it is not checked, until a search with
/// it succeeds.
#[test]
fn a_key_the_check_could_not_search_with_is_saved_as_not_checked() {
    local(async {
        let plugged = Plugged::new(FakeExa::new(Exa {
            pages: pages(),
            key: Some("exa-example-abcd"),
            refuse: Some((429, vec![("Retry-After", "20")])),
            ..Exa::default()
        }));
        plugged.call("save_key", json!({ "key": "exa-example-abcd" })).await.unwrap();
        assert_eq!(plugged.saved_key().as_deref(), Some("exa-example-abcd"));
        let unchecked = Some("Exa’s rate limit was reached".to_owned());
        assert_eq!(
            plugged.state().await.key,
            Some(SavedKey { last_four: "abcd".into(), unchecked: unchecked.clone() })
        );
        // A search that fails leaves it unchecked; one that succeeds checks it.
        assert_eq!(plugged.run(&["web", "search", "rolldown 1.0"]).await.0, 1);
        assert_eq!(plugged.state().await.key.unwrap().unchecked, unchecked);
        plugged.exa.0.borrow_mut().refuse = None;
        let changes = plugged.demi.changes();
        assert_eq!(plugged.run(&["web", "search", "rolldown 1.0"]).await.0, 0);
        assert_eq!(plugged.state().await.key.unwrap().unchecked, None);
        assert!(plugged.demi.changes() > changes);
    });
}
