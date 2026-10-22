// Whether the service is `Send`, which serving it needs, is decided through
// a conversation's browser and the channels that answer with it, deeper than
// the default 128 steps of the trait solver; the library sets the same limit.
#![recursion_limit = "256"]

use std::sync::Arc;

#[tokio::main]
async fn main() {
    // A panic leaves its report on standard error
    // (`crates/xtask/src/native/build.rs`).
    demi_shared_cli::install_panic_hook();
    demi_command_sdk::Launch::from_process();
    // Events go to standard error as lines of the log, which the runner keeps
    // as they are.
    demi_command_sdk::log_to_stderr("browser");
    let browser = Arc::new(demi_browser::DemiBrowser::new());
    let chrome = browser.chrome().clone();
    let service = demi_command_sdk::serve_stdio(browser);
    // Profiles a service that ended without retiring its browsers left are
    // removed beside serving; a sweep still
    // under way when serving ends is given up.
    let sweep = demi_command_package_browser_chrome::tabs::environment::sweep_orphans(&chrome);
    tokio::pin!(service, sweep);
    let result = tokio::select! {
        result = &mut service => result,
        () = &mut sweep => service.await,
    };
    if let Err(error) = result {
        tracing::error!(event = "browser_service.exited", %error, "the service ended with an error");
        std::process::exit(1);
    }
    // The executable owns synchronous Windows stdio workers; HTTP/2 has drained.
    std::process::exit(0);
}
