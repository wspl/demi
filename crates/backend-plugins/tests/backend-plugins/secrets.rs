//! A plugin's secrets: a value it keeps for a
//! user, written by versioned compare-and-set and stored sealed, bound to
//! its plugin, its user and its name.

use demi_backend_database::plugin_secrets::SecretRow;
use demi_backend_plugins::{PageCallError, Registry};
use demi_plugin_interface::{PluginError, PortMessage, PortRefusal};
use serde_json::{Value, json};

use crate::support::{Backend, PortProbe, call, refusal, within};

async fn backend() -> Backend {
    let registry = Registry::new(
        vec![
            PortProbe::new("probe", None).boxed(),
            PortProbe::new("probe-two", None).boxed(),
        ],
        |_| true,
    )
    .unwrap();
    Backend::start(registry, Vec::new()).await
}

#[tokio::test(flavor = "local")]
async fn a_secret_round_trips_sealed_and_a_write_at_a_stale_revision_conflicts() {
    within(async {
        let backend = backend().await;
        let alice = backend.user("alice").await;
        let plugins = backend.plugins(&alice);
        let write = |value: &str, revision: Value| {
            call(
                &plugins,
                "probe",
                "write_secret",
                json!({ "name": "api-key", "value": value, "revision": revision }),
            )
        };
        let read = || call(&plugins, "probe", "read_secret", json!({ "name": "api-key" }));

        assert_eq!(read().await.unwrap(), Value::Null);
        assert_eq!(
            write("sk-first-1234", Value::Null).await.unwrap(),
            json!({ "revision": 1 })
        );
        assert_eq!(
            read().await.unwrap(),
            json!({ "value": "sk-first-1234", "revision": 1 })
        );
        // At rest the value is sealed.
        let row = SecretRow {
            user: alice.clone(),
            plugin: "probe".into(),
            name: "api-key".into(),
        };
        let stored = backend.control.plugin_secret(row).await.unwrap().unwrap();
        assert!(!stored.sealed.windows(4).any(|window| window == b"1234"));

        // A write that read no value, or an older revision, comes second.
        assert_eq!(
            refusal(write("sk-other", Value::Null).await),
            PortRefusal::Conflict
        );
        assert_eq!(
            write("sk-second-5678", json!(1)).await.unwrap(),
            json!({ "revision": 2 })
        );
        assert_eq!(refusal(write("sk-other", json!(1)).await), PortRefusal::Conflict);
        let remove = |revision: u64| {
            call(
                &plugins,
                "probe",
                "remove_secret",
                json!({ "name": "api-key", "revision": revision }),
            )
        };
        assert_eq!(refusal(remove(1).await), PortRefusal::Conflict);
        assert_eq!(
            read().await.unwrap(),
            json!({ "value": "sk-second-5678", "revision": 2 })
        );
        remove(2).await.unwrap();
        assert_eq!(read().await.unwrap(), Value::Null);

        // Another user's plugin, and another plugin of the user's, have
        // secrets of their own.
        write("sk-alice", Value::Null).await.unwrap();
        let bob = backend.user("bob").await;
        let bobs = backend.plugins(&bob);
        assert_eq!(
            call(&bobs, "probe", "read_secret", json!({ "name": "api-key" }))
                .await
                .unwrap(),
            Value::Null
        );
        assert_eq!(
            call(&plugins, "probe-two", "read_secret", json!({ "name": "api-key" }))
                .await
                .unwrap(),
            Value::Null
        );
    })
    .await;
}

#[tokio::test(flavor = "local")]
async fn a_sealed_secret_copied_to_another_plugin_user_or_name_does_not_open() {
    within(async {
        let backend = backend().await;
        let alice = backend.user("alice").await;
        let bob = backend.user("bob").await;
        call(
            &backend.plugins(&alice),
            "probe",
            "write_secret",
            json!({ "name": "api-key", "value": "sk-alice-1234" }),
        )
        .await
        .unwrap();
        // Behind the service's back, the sealed value is copied into the
        // rows of another user, another plugin and another name.
        for (user, plugin, name) in [
            ("bob", "probe", "api-key"),
            ("alice", "probe-two", "api-key"),
            ("alice", "probe", "other-key"),
        ] {
            demi_backend_database::control::testing::execute(
                &backend.control,
                "INSERT INTO plugin_secrets (user_id, plugin, name, sealed, revision)
                 SELECT ?1, ?2, ?3, sealed, 1 FROM plugin_secrets
                 WHERE user_id = 'alice' AND plugin = 'probe' AND name = 'api-key'",
                vec![user.into(), plugin.into(), name.into()],
            )
            .await;
        }
        for (user, plugin, name) in [
            (&bob, "probe", "api-key"),
            (&alice, "probe-two", "api-key"),
            (&alice, "probe", "other-key"),
        ] {
            let read = call(
                &backend.plugins(user),
                plugin,
                "read_secret",
                json!({ "name": name }),
            )
            .await;
            let Err(PageCallError::Plugin(PluginError::Failed { message })) = read else {
                panic!("the copy at {user}/{plugin}/{name} opened: {read:?}");
            };
            assert!(message.contains("does not open"), "{message}");
        }
        // The original still opens.
        assert_eq!(
            call(
                &backend.plugins(&alice),
                "probe",
                "read_secret",
                json!({ "name": "api-key" })
            )
            .await
            .unwrap(),
            json!({ "value": "sk-alice-1234", "revision": 1 })
        );
    })
    .await;
}

/// A plugin, in process or over a wire, cannot name the secret the host
/// keeps a sign-in's tokens as.
#[test]
fn a_plugin_cannot_name_the_secret_of_its_sign_in() {
    let message = json!({ "type": "read_secret", "name": "oauth:tokens" });
    assert!(serde_json::from_value::<PortMessage>(message).is_err());
}
