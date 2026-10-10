use std::sync::Arc;

#[tokio::main]
async fn main() {
    // A panic leaves its report on standard error (`builds-and-releases.md`
    // § Build profiles).
    demi_shared_cli::install_panic_hook();
    demi_command_sdk::Launch::from_process();
    // Diagnostics go to standard error, which the runner drains into the
    // Host's log line by line.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .without_time()
        .with_target(false)
        .init();
    if let Err(error) =
        demi_command_sdk::serve_stdio(Arc::new(demi_file::DemiFile::default())).await
    {
        eprintln!("demi-file: {error}");
        std::process::exit(1);
    }
    // The executable owns synchronous Windows stdio workers; HTTP/2 has drained.
    std::process::exit(0);
}
