use std::sync::Arc;

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() != Some("--command-service") {
        eprintln!("Usage: demi-file --command-service");
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
    if let Err(error) = demi_command_service::serve_stdio(Arc::new(demi_file::DemiFile::default())).await {
        eprintln!("demi-file: {error}");
        std::process::exit(1);
    }
    // The executable owns synchronous Windows stdio workers; HTTP/2 has drained.
    std::process::exit(0);
}
