use std::sync::Arc;

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() != Some("--command-service") {
        eprintln!("Usage: demi-commands --command-service");
        std::process::exit(2);
    }
    let result =
        demi_command_service::serve_stdio(Arc::new(demi_builtin_package::DemiCommands::default()))
            .await;
    if let Err(error) = result {
        eprintln!("demi-commands: {error}");
        std::process::exit(1);
    }
    // The executable owns synchronous Windows stdio workers; HTTP/2 has drained.
    std::process::exit(0);
}
