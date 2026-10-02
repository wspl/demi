// Whether the service is `Send`, which serving it needs, is decided through
// a conversation's browser and the channels that answer with it, deeper than
// the default 128 steps of the trait solver; the library sets the same limit.
#![recursion_limit = "256"]

use std::sync::Arc;

#[tokio::main]
async fn main() {
    let launch = demi_command_sdk::Launch::from_process();
    // Diagnostics go to standard error, which the runner drains into the
    // Host's log line by line.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .without_time()
        .with_target(false)
        .init();
    let chrome = demi_command_package_browser_chrome::driver::installation::Chrome::new(
        launch
            .resource(demi_command_package_browser_protocol::release::RESOURCE)
            .map(ToOwned::to_owned),
    );
    let service =
        demi_command_sdk::serve_stdio(Arc::new(demi_browser::DemiBrowser::new(chrome.clone())));
    // Profiles a service that ended without retiring its browsers left are
    // removed beside serving (`browser.md` § Native driver).
    let (result, ()) = tokio::join!(
        service,
        demi_command_package_browser_chrome::tabs::environment::sweep_orphans(&chrome)
    );
    if let Err(error) = result {
        eprintln!("demi-browser: {error}");
        std::process::exit(1);
    }
    // The executable owns synchronous Windows stdio workers; HTTP/2 has drained.
    std::process::exit(0);
}
