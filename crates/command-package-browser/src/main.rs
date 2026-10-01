use std::sync::Arc;

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() != Some("--command-service") {
        eprintln!("Usage: demi-browser --command-service");
        std::process::exit(2);
    }
    // Diagnostics go to standard error, which the runner drains into the
    // Host's log line by line.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .without_time()
        .with_target(false)
        .init();
    let browsers = demi_browser_driver::installation::BrowserDirectories::host();
    let service = demi_command_sdk::serve_stdio(Arc::new(demi_browser::DemiBrowser::new(
        browsers.clone(),
    )));
    // Profiles a service that ended without retiring its browsers left are
    // removed beside serving (`browser.md` § Native driver).
    let (result, ()) = tokio::join!(
        service,
        demi_browser_tabs::environment::sweep_orphans(&browsers)
    );
    if let Err(error) = result {
        eprintln!("demi-browser: {error}");
        std::process::exit(1);
    }
    // The executable owns synchronous Windows stdio workers; HTTP/2 has drained.
    std::process::exit(0);
}
