//! Settings › Web Search: the state says how the user is connected and
//! which way searches go, and the methods sign in and out and save and
//! remove the key, which one search checks first.

use demi_plugin_web::SavedKey;
use serde_json::json;

use crate::exa::{Exa, FakeExa, pages};
use crate::support::Plugged;

fn local<T>(test: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(tokio::task::LocalSet::new().run_until(test))
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
