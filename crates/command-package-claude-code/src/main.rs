use std::sync::Arc;

// Invocations are rare and wait on the network and the disk
// (`concurrency.md` § demi-claude-code and the command-sdk).
#[tokio::main(flavor = "current_thread")]
async fn main() {
    if std::env::args().nth(1).as_deref() != Some("--command-service") {
        eprintln!("Usage: demi-claude-code --command-service");
        std::process::exit(2);
    }
    let result =
        demi_command_sdk::serve_stdio(Arc::new(demi_claude_code::DemiClaude::default())).await;
    if let Err(error) = result {
        eprintln!("demi-claude-code: {error}");
        std::process::exit(1);
    }
    // The executable owns synchronous Windows stdio workers; HTTP/2 has drained.
    std::process::exit(0);
}
