//! Joined cleanup for real-browser fixture assertions, including unwinding tests.
use demi_command_package_browser_chrome::driver::{numbers::TabNumbers, operation::Result};
use demi_command_package_browser_chrome::tabs::environment::{
    BrowserEnvironment, LaunchOptions, with_browser,
};
use demi_command_sdk::testing::counting_numbers;
use futures_util::FutureExt;
use std::{future::Future, path::PathBuf};
use tokio_util::sync::CancellationToken;

/// Serve deterministic browser fixtures, including failures before HTTP headers.
pub async fn with_fixture<F, W>(exercise: F)
where
    F: FnOnce(BrowserEnvironment, String) -> W,
    W: Future<Output = Result<()>>,
{
    let numbers = TabNumbers::new(counting_numbers(), "conversation".into());
    with_numbered_fixture(numbers, exercise).await;
}

/// [`with_fixture`] whose browser numbers its tabs from `numbers`, as one
/// conversation's browsers do one after another.
pub async fn with_numbered_fixture<F, W>(numbers: TabNumbers, exercise: F)
where
    F: FnOnce(BrowserEnvironment, String) -> W,
    W: Future<Output = Result<()>>,
{
    let executable = PathBuf::from(std::env::var_os("DEMI_TEST_CHROME").expect("DEMI_TEST_CHROME"));
    let server = crate::server::Server::start(include_str!("repairs.html")).await;
    let base = server.base.clone();
    let result = with_browser(
        LaunchOptions::pinned(
            executable,
            demi_command_protocol::CommandLocale {
                time_zone: "UTC".into(),
                languages: vec!["en-US".into()],
            },
        )
        .unwrap(),
        numbers,
        CancellationToken::new(),
        |browser| async move {
            Ok(
                std::panic::AssertUnwindSafe(async move { exercise(browser, base).await })
                    .catch_unwind()
                    .await,
            )
        },
    )
    .await;
    server.close().await;
    match result.expect("joined fixture retirement") {
        Ok(result) => assert!(result.is_ok(), "{result:?}"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
