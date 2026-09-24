//! The credential pool, the refresh protocol and the account operations
//! (`providers.md` § Credential vault, § Token refresh).

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use demi_core::{LoginPending, Timestamp};
use demi_provider::{
    credentials::{
        AccountError, AccountKit, AccountLabel, AccountMeta, Accounts, AccountsCapability,
        AccountsError, AddAccount, CredentialPool, LoginError, MemoryCredentialPool, NewAccount,
        PoolError, RefreshGates, RenewError, SecretDecodeError, SecretDocument, SecretFault,
        SubscriptionAccounts, credential_id_for, read_secret, renew,
    },
    testing::FixedClock,
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
    }
}

async fn pool_with(id: &str, secret: &Tokens) -> MemoryCredentialPool {
    let pool = MemoryCredentialPool::new();
    pool.write(meta(id), secret.encode()).await.unwrap();
    pool
}

async fn stored(pool: &MemoryCredentialPool, id: &str) -> Tokens {
    read_secret::<Tokens>(&*pool.document(id)).await.unwrap().secret
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
    assert!(!third.is_finished(), "the third refresher ran beside the second");
    // A failed refresh releases its turn like a successful one: the turn
    // ends when its permit is dropped, however the refresh ended.
    drop(second_turn);
    drop(third.await.unwrap());
    assert_eq!(*order.lock().unwrap(), ["second", "third"]);
}

#[tokio::test]
async fn a_due_secret_is_refreshed_and_stored_over_the_version_read() {
    let pool = pool_with("a", &tokens("old", "r1")).await;
    let before = pool.document("a").read().await.unwrap().unwrap().version;
    let renewed = renew(
        &*pool.document("a"),
        |_: &Tokens| true,
        |secret: Tokens| async move {
            assert_eq!(secret.refresh, "r1");
            Ok::<_, String>(tokens("new", "r2"))
        },
    )
    .await
    .unwrap();
    assert_eq!(renewed, tokens("new", "r2"));
    let revision = pool.document("a").read().await.unwrap().unwrap();
    assert_eq!(revision.text, tokens("new", "r2").encode());
    assert_ne!(revision.version, before);
}

/// A vendor that records the refresh tokens it spends and yields once, so
/// that a concurrent refresher queues behind it.
fn vendor(spent: &Arc<Mutex<Vec<String>>>) -> impl Fn(Tokens) -> BoxFuture<'static, Result<Tokens, String>> {
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
        renew(&*one, due, vendor(&spent)),
        renew(&*two, due, vendor(&spent)),
    );
    assert_eq!((first.unwrap().access, second.unwrap().access), ("new".into(), "new".into()));
    assert_eq!(*spent.lock().unwrap(), ["r1"]);

    // A rule that still asks after the wait, as for new tokens that are due
    // as well, refreshes again, spending the token the earlier one stored.
    let pool = pool_with("a", &tokens("old", "r1")).await;
    let spent = Arc::new(Mutex::new(Vec::new()));
    let always = |_: &Tokens| true;
    let (one, two) = (pool.document("a"), pool.document("a"));
    let (first, second) = tokio::join!(
        renew(&*one, always, vendor(&spent)),
        renew(&*two, always, vendor(&spent)),
    );
    assert!(first.is_ok() && second.is_ok());
    assert_eq!(*spent.lock().unwrap(), ["r1", "after-r1"]);
    assert_eq!(stored(&pool, "a").await, tokens("new", "after-after-r1"));
}

#[tokio::test]
async fn a_refusal_uses_the_tokens_another_writer_stored_and_fails_when_nobody_did() {
    let pool = pool_with("a", &tokens("old", "r1")).await;
    let raced = renew(&*pool.document("a"), |_: &Tokens| true, |_| {
        let pool = pool.clone();
        async move {
            // Another worker refreshed first and spent the token.
            pool.write(meta("a"), tokens("winner", "rw").encode()).await.unwrap();
            Err("refresh token already used".to_owned())
        }
    })
    .await;
    assert_eq!(raced, Ok(tokens("winner", "rw")));

    let alone = renew(&*pool.document("a"), |_: &Tokens| true, |_| async {
        Err::<Tokens, _>("refresh token revoked".to_owned())
    })
    .await;
    assert_eq!(alone, Err(RenewError::Refresh("refresh token revoked".to_owned())));
}

#[tokio::test]
async fn a_refreshed_secret_whose_replace_loses_gives_way_to_the_winner() {
    let pool = pool_with("a", &tokens("old", "r1")).await;
    let renewed = renew(&*pool.document("a"), |_: &Tokens| true, |_| {
        let pool = pool.clone();
        async move {
            pool.write(meta("a"), tokens("winner", "rw").encode()).await.unwrap();
            Ok::<_, String>(tokens("loser", "rl"))
        }
    })
    .await;
    assert_eq!(renewed, Ok(tokens("winner", "rw")));
    assert_eq!(stored(&pool, "a").await, tokens("winner", "rw"));
}

#[tokio::test]
async fn a_corrupt_document_is_refused_by_its_path_and_never_quotes_a_value() {
    let pool = MemoryCredentialPool::new();
    let cases = [
        (r#"{"access":"sk-secret-1","refresh":7}"#, "refresh", SecretFault::Shape),
        (r#"{"access":"sk-secret-1"}"#, ".", SecretFault::Shape),
        (r#"{"access":"sk-secret-1","refresh":"r","extra":"sk-secret-2"}"#, "extra", SecretFault::Shape),
        (r#"{"access":"sk-secret-1""#, ".", SecretFault::Syntax),
        (r#"{"access":"sk-secret-1","refresh":"r"} trailing"#, ".", SecretFault::Syntax),
    ];
    for (text, path, fault) in cases {
        pool.write(meta("a"), text.into()).await.unwrap();
        let error = read_secret::<Tokens>(&*pool.document("a")).await.unwrap_err();
        let expected = SecretDecodeError {
            path: path.into(),
            fault,
        };
        assert_eq!(error, AccountError::Invalid(expected), "{text}");
        assert!(!error.to_string().contains("sk-secret"), "{error}");
    }
    let missing = read_secret::<Tokens>(&*pool.document("absent")).await;
    assert_eq!(missing, Err(AccountError::Missing));
}

#[tokio::test]
async fn a_memory_pool_keeps_accounts_by_id_with_a_versioned_document_and_an_active_one() {
    let pool = MemoryCredentialPool::new();
    pool.write(meta("b"), "{}".into()).await.unwrap();
    pool.write(meta("a"), "{}".into()).await.unwrap();
    let ids: Vec<String> = pool.list().await.unwrap().into_iter().map(|account| account.id).collect();
    assert_eq!(ids, ["a", "b"]);

    let first = pool.document("a").read().await.unwrap().unwrap();
    let second = pool.document("a").read().await.unwrap().unwrap();
    assert!(pool.document("a").replace("two".into(), first.version).await.unwrap());
    assert!(!pool.document("a").replace("lost".into(), second.version).await.unwrap());
    assert_eq!(pool.document("a").read().await.unwrap().unwrap().text, "two");
    assert!(pool.document("absent").read().await.unwrap().is_none());

    assert_eq!(pool.active().await.unwrap(), None);
    assert_eq!(pool.set_active("zz").await, Err(PoolError::NotFound("zz".into())));
    pool.set_active("b").await.unwrap();
    assert_eq!(pool.active().await.unwrap().as_deref(), Some("b"));
    pool.remove("b").await.unwrap();
    assert_eq!(pool.active().await.unwrap(), None);
    let entries: Vec<(String, String)> = pool
        .entries()
        .into_iter()
        .map(|(meta, secret)| (meta.id, secret))
        .collect();
    assert_eq!(entries, [("a".to_owned(), "two".to_owned())]);
}

#[test]
fn an_account_id_is_derived_from_its_identity_or_else_its_label() {
    assert_eq!(credential_id_for(Some("acct-1"), "a@example.com"), "cred-ba36a4edd92d37c6");
    assert_eq!(credential_id_for(None, "label only"), "cred-db98004c5bc389e4");
    assert_eq!(credential_id_for(Some(""), "label only"), "cred-db98004c5bc389e4");
}

/// A family whose device login hands out scripted accounts, or never ends.
struct Kit {
    logins: Mutex<VecDeque<NewAccount>>,
}

fn account(identity: &str, secret: &str) -> NewAccount {
    NewAccount {
        secret: secret.into(),
        label: AccountLabel {
            label: format!("{identity}@example.com"),
            detail: Some("device".into()),
            identity_key: Some(identity.into()),
        },
    }
}

impl AccountKit for Kit {
    fn capability(&self) -> AccountsCapability {
        AccountsCapability {
            login: true,
            add: false,
        }
    }

    fn login<'a>(
        &'a self,
        pending: &'a (dyn Fn(LoginPending) + Send + Sync),
    ) -> Option<BoxFuture<'a, Result<NewAccount, LoginError>>> {
        let next = self.logins.lock().unwrap().pop_front();
        Some(Box::pin(async move {
            pending(LoginPending {
                verification_url: "https://vendor.example/device".into(),
                user_code: Some("ABCD-1234".into()),
                expires_at: None,
            });
            match next {
                Some(account) => Ok(account),
                // The user never confirms.
                None => std::future::pending::<Result<NewAccount, LoginError>>().await,
            }
        }))
    }

    fn add(&self, _input: AddAccount) -> Option<Result<NewAccount, AccountsError>> {
        None
    }
}

#[tokio::test]
async fn a_login_stores_its_account_by_identity_and_the_first_one_becomes_active() {
    let pool = MemoryCredentialPool::new();
    let kit = Kit {
        logins: Mutex::new(VecDeque::from([
            account("acct-1", "first secret"),
            account("acct-1", "second secret"),
            account("acct-2", "other secret"),
        ])),
    };
    let clock = Arc::new(FixedClock(NOW.parse::<Timestamp>().unwrap()));
    let accounts = Accounts::new(Arc::new(pool.clone()), kit, clock);
    assert_eq!(accounts.capability(), AccountsCapability { login: true, add: false });

    let shown = Arc::new(Mutex::new(Vec::new()));
    let report = {
        let shown = shown.clone();
        move |pending: LoginPending| shown.lock().unwrap().push(pending.user_code)
    };
    let first = accounts.login(&report).await.unwrap();
    assert_eq!(first.id, "cred-ba36a4edd92d37c6");
    let now: Timestamp = NOW.parse().unwrap();
    assert_eq!((first.label.as_str(), first.updated_at), ("acct-1@example.com", Some(now)));
    assert_eq!(*shown.lock().unwrap(), [Some("ABCD-1234".to_owned())]);
    assert_eq!(accounts.active().await.unwrap().as_deref(), Some(first.id.as_str()));

    // Logging in again with the same account replaces its record.
    let again = accounts.login(&report).await.unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(pool.document(&first.id).read().await.unwrap().unwrap().text, "second secret");
    // Another account is added beside it and does not take over.
    let other = accounts.login(&report).await.unwrap();
    assert_ne!(other.id, first.id);
    assert_eq!(accounts.list().await.unwrap().len(), 2);
    assert_eq!(accounts.active().await.unwrap().as_deref(), Some(first.id.as_str()));
    assert_eq!(pool.meta(&other.id).await.unwrap().unwrap().source, "login:device");

    // The active account cannot be removed; another one can.
    assert_eq!(accounts.remove(&first.id).await, Err(AccountsError::Active));
    accounts.remove(&other.id).await.unwrap();
    assert_eq!(accounts.remove("cred-absent").await, Err(AccountsError::NotFound("cred-absent".into())));
    assert_eq!(accounts.set_active("cred-absent").await, Err(AccountsError::NotFound("cred-absent".into())));
    let listed: Vec<String> = accounts.list().await.unwrap().into_iter().map(|info| info.id).collect();
    assert_eq!(listed, [first.id]);
    let setup_token = AddAccount::SetupToken("sk-ant-oat01-x".to_owned().try_into().unwrap());
    assert_eq!(accounts.add(setup_token).await, Err(AccountsError::Unsupported));
}

#[tokio::test(start_paused = true)]
async fn dropping_a_login_cancels_it_and_stores_nothing() {
    let pool = MemoryCredentialPool::new();
    let kit = Kit {
        logins: Mutex::new(VecDeque::new()),
    };
    let clock = Arc::new(FixedClock(NOW.parse::<Timestamp>().unwrap()));
    let accounts = Accounts::new(Arc::new(pool.clone()), kit, clock);
    let report = |_: LoginPending| {};
    let waiting = tokio::time::timeout(Duration::from_millis(20), accounts.login(&report)).await;
    assert!(waiting.is_err(), "the login ended without the user");
    assert!(pool.list().await.unwrap().is_empty());
}
