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
    let mut browser = demi_browser::DemiBrowser::new();
    if let Some(data) = launch.data() {
        browser = browser.with_data_directory(data.to_owned());
    }
    let browser = Arc::new(browser);
    let chrome = browser.chrome().clone();
    let service = demi_command_sdk::serve_stdio(browser);
    // Profiles a service that ended without retiring its browsers left are
    // removed beside serving (`browser.md` § Native driver); a sweep still
    // under way when serving ends is given up.
    let sweep = demi_command_package_browser_chrome::tabs::environment::sweep_orphans(&chrome);
    tokio::pin!(service, sweep);
    let result = tokio::select! {
        result = &mut service => result,
        () = &mut sweep => service.await,
    };
    if let Err(error) = result {
        eprintln!("demi-browser: {error}");
        std::process::exit(1);
    }
    // The executable owns synchronous Windows stdio workers; HTTP/2 has drained.
    std::process::exit(0);
}
