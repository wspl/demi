//! An account's sign-in, which its CLI keeps in the account's directory
//! (`crates/provider-claude-code/src/account.rs`).
//!
//! - A sign-in the vendor refused marks the account signed out and fails with
//!   the authentication error, and the account starts no process until it signs
//!   in again.
//! - A sign-in an earlier release kept in the record goes to the account's
//!   directory with the first start, as the CLI's own files, and leaves the
//!   record.
//!
//! Each test costs a few milliseconds.

use demi_host_interface::Signal;
use demi_provider_claude_code::{CliWork, Handover};
use demi_provider_common::credentials::{CredentialPool as _, MemoryCredentialPool};
use demi_provider_common::{ErrorCode, Provider as _, ProviderEvent};
use demi_shared_types::AuthState;
use serde_json::{Value, json};

use crate::cli::*;

/// The lines Claude Code 2.1.301 prints for a turn whose sign-in the vendor
/// refused and the CLI could not renew: its made-up assistant line saying
/// `authentication_failed`, then a failed result, which names no status.
///
/// With `status`, the result names it, as a refusal of the call does.
fn refused(status: Option<u16>) -> Vec<Value> {
    let words = "Failed to authenticate: OAuth session expired and could not be refreshed";
    vec![
        json!({
            "type": "assistant",
            "error": "authentication_failed",
            "is_api_error_message": true,
            "message": { "model": "<synthetic>", "content": [{ "type": "text", "text": words }] },
        }),
        json!({
            "type": "result",
            "subtype": "success",
            "is_error": true,
            "api_error_status": status,
            "terminal_reason": "api_error",
            "result": words,
            "usage": { "input_tokens": 0, "output_tokens": 0 },
        }),
    ]
}

#[tokio::test(flavor = "local")]
async fn a_sign_in_the_vendor_refused_signs_the_account_out_and_fails_with_the_authentication_error()
{
    for status in [None, Some(401)] {
        let (provider, pool) = provider_with_pool().await;
        let account = pool.active().await.unwrap().unwrap();
        let (placement, mut starts) = ScriptedPlacement::new();
        let mut runtime = runtime_of(&provider, &placement);
        let (events, cli) = tokio::join!(
            all_events(runtime.run(request_without_tools(vec![user("hi")]))),
            async {
                let cli = starts.next().await;
                for line in refused(status) {
                    cli.say(line);
                }
                cli
            }
        );
        let [ProviderEvent::Error(failure)] = events.as_slice() else {
            panic!("the refused sign-in did not fail the turn: {events:?}");
        };
        let message = "Claude Code signed the account out. Sign in to it again";
        assert_eq!(
            (failure.message.as_str(), failure.code.clone()),
            (message, Some(ErrorCode::AuthExpired)),
            "{status:?}"
        );
        // The process is closed, SIGTERM first, and its directory goes.
        assert_eq!(cli.signals(), [Signal::Terminate]);
        assert!(cli.config_removed());
        // The account is signed out until a sign-in replaces it, which the
        // accounts list and the entry's status say.
        let meta = pool.meta(&account).await.unwrap().unwrap();
        assert!(meta.signed_out);
        assert!(provider.accounts().unwrap().list().await.unwrap()[0].signed_out);
        assert_eq!(
            provider.auth_status().await,
            AuthState::Unauthenticated {
                message: Some(message.into())
            }
        );
        // Its next request starts no process: the CLI emptied the sign-in.
        let events = all_events(runtime.run(request_without_tools(vec![user("again")]))).await;
        let [ProviderEvent::Error(again)] = events.as_slice() else {
            panic!("a signed-out account's request did not fail: {events:?}");
        };
        assert_eq!(again.code, Some(ErrorCode::AuthExpired));
        assert_eq!(placement.starts(), 1);
    }
}

#[tokio::test(flavor = "local")]
async fn an_earlier_releases_sign_in_goes_to_the_accounts_directory_once_and_leaves_the_record() {
    let pool = MemoryCredentialPool::new();
    let account = seed(
        &pool,
        Some(earlier_sign_in(TOKEN, REFRESH, "2026-09-24T16:00:00Z")),
    )
    .await;
    let provider = provider_of(&pool, &account);
    let (placement, mut starts) = ScriptedPlacement::new();
    let mut runtime = runtime_of(&provider, &placement);
    let (_, first) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("hi")]))),
        async {
            let cli = starts.next().await;
            cli.result(1, 1);
            cli
        }
    );
    // The CLI's own files, as Claude Code 2.1.301 writes them: its account,
    // then its sign-in, whose presence says the directory holds one, so it
    // is put in place last.
    let credentials = json!({ "claudeAiOauth": {
        "accessToken": TOKEN,
        "refreshToken": REFRESH,
        "expiresAt": 1_790_265_600_000_i64,
        "scopes": ["user:inference", "user:profile"],
        "subscriptionType": "max",
    } });
    let config = json!({ "oauthAccount": {
        "accountUuid": "account-1",
        "emailAddress": "alex@example.test",
    } });
    let CliWork::Account {
        account: named,
        handover: Some(Handover { files }),
    } = &first.work
    else {
        panic!("the first start handed nothing over: {:?}", first.work);
    };
    assert_eq!(named, &account);
    let written: Vec<(&str, Value)> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), serde_json::from_slice(bytes).unwrap()))
        .collect();
    assert_eq!(
        written,
        [(".claude.json", config), (".credentials.json", credentials)]
    );
    // The tokens moved: the record holds none, and keeps the account.
    assert_eq!(pool.document(&account).read().await.unwrap(), None);
    assert!(pool.meta(&account).await.unwrap().is_some());

    // The next process finds the sign-in in the directory.
    runtime.close().await;
    let (_, second) = tokio::join!(
        all_events(runtime.run(request_without_tools(vec![user("again")]))),
        async {
            let cli = starts.next().await;
            cli.result(1, 1);
            cli
        }
    );
    assert_eq!(
        second.work,
        CliWork::Account {
            account,
            handover: None
        }
    );
}
