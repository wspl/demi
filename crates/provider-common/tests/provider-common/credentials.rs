//! The credential pool, the refresh protocol and the account operations
//! (`crates/backend-providers/src/vault/entries.rs`,
//! `crates/provider-common/src/credentials/refresh.rs`).

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use demi_provider_common::credentials::{
        AccountMeta, CredentialPool, MemoryCredentialPool, RefreshGates, SecretDocument, read_secret, renew,
    };
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};

/// A secret document of a made-up family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tokens {
    access: String,
    refresh: String,
}

impl SecretDocument for Tokens {}

fn tokens(access: &str, refresh: &str) -> Tokens {
    Tokens {
        access: access.into(),
        refresh: refresh.into(),
    }
}

const NOW: &str = "2026-09-18T14:00:00.000Z";

fn meta(id: &str) -> AccountMeta {
    AccountMeta {
        id: id.into(),
        label: format!("{id}@example.com"),
        detail: None,
        updated_at: NOW.parse().unwrap(),
        source: "test".into(),
        identity_key: Some(id.into()),
        signed_out: false,
    }
}

async fn pool_with(id: &str, secret: &Tokens) -> MemoryCredentialPool {
    let pool = MemoryCredentialPool::new();
    pool.write(meta(id), Some(secret.encode())).await.unwrap();
    pool
}

async fn stored(pool: &MemoryCredentialPool, id: &str) -> Tokens {
    read_secret::<Tokens>(&*pool.document(id))
        .await
        .unwrap()
        .secret
}

#[tokio::test]
async fn refresh_turns_of_one_account_run_one_at_a_time_in_arrival_order() {
    let gates = Arc::new(RefreshGates::new());
    let first = gates.turn("a").await;
    // Another account's turn does not wait.
    let other = tokio::time::timeout(Duration::from_secs(5), gates.turn("b")).await;
    assert!(other.is_ok(), "another account waited");

    let order = Arc::new(Mutex::new(Vec::new()));
    let waiter = |name: &'static str| {
        let gates = gates.clone();
        let order = order.clone();
        tokio::spawn(async move {
            let permit = gates.turn("a").await;
            order.lock().unwrap().push(name);
            permit
        })
    };
    let second = waiter("second");
    tokio::task::yield_now().await;
    // A waiter that gives up leaves its place to the ones behind it.
    let abandoned = waiter("abandoned");
    tokio::task::yield_now().await;
    abandoned.abort();
    let third = waiter("third");
    tokio::task::yield_now().await;

    drop(first);
    let second_turn = second.await.unwrap();
    tokio::task::yield_now().await;
    assert!(
        !third.is_finished(),
        "the third refresher ran beside the second"
    );
    // A failed refresh releases its turn like a successful one: the turn
    // ends when its permit is dropped, however the refresh ended.
    drop(second_turn);
    drop(third.await.unwrap());
    assert_eq!(*order.lock().unwrap(), ["second", "third"]);
}

/// A vendor that records the refresh tokens it spends and yields once, so
/// that a concurrent refresher queues behind it.
fn vendor(
    spent: &Arc<Mutex<Vec<String>>>,
) -> impl Fn(Tokens) -> BoxFuture<'static, Result<Tokens, String>> {
    let spent = spent.clone();
    move |secret: Tokens| {
        let spent = spent.clone();
        Box::pin(async move {
            spent.lock().unwrap().push(secret.refresh.clone());
            tokio::task::yield_now().await;
            Ok(tokens("new", &format!("after-{}", secret.refresh)))
        })
    }
}

#[tokio::test]
async fn a_refresher_that_waited_uses_the_tokens_it_finds_unless_its_rule_still_asks() {
    // Grok's rule: only the token that was found expiring needs a refresh.
    let pool = pool_with("a", &tokens("old", "r1")).await;
    let spent = Arc::new(Mutex::new(Vec::new()));
    let due = |secret: &Tokens| secret.access == "old";
    let (one, two) = (pool.document("a"), pool.document("a"));
    let (first, second) = tokio::join!(
        renew(one.into(), due, vendor(&spent)),
        renew(two.into(), due, vendor(&spent)),
    );
    assert_eq!(
        (first.unwrap().access, second.unwrap().access),
        ("new".into(), "new".into())
    );
    assert_eq!(*spent.lock().unwrap(), ["r1"]);

    // A rule that still asks after the wait, as for new tokens that are due
    // as well, refreshes again, spending the token the earlier one stored.
    let pool = pool_with("a", &tokens("old", "r1")).await;
    let spent = Arc::new(Mutex::new(Vec::new()));
    let always = |_: &Tokens| true;
    let (one, two) = (pool.document("a"), pool.document("a"));
    let (first, second) = tokio::join!(
        renew(one.into(), always, vendor(&spent)),
        renew(two.into(), always, vendor(&spent)),
    );
    assert!(first.is_ok() && second.is_ok());
    assert_eq!(*spent.lock().unwrap(), ["r1", "after-r1"]);
    assert_eq!(stored(&pool, "a").await, tokens("new", "after-after-r1"));
}
