use std::sync::Arc;

// Invocations are rare and wait on the network and the disk
// (`concurrency.md` § demi-claude-code and the command-sdk).
#[tokio::main(flavor = "current_thread")]
async fn main() {
    // A panic leaves its report on standard error (`builds-and-releases.md`
    // § Build profiles).
    demi_shared_cli::install_panic_hook();
    demi_command_sdk::Launch::from_process();
    let result =
        demi_command_sdk::serve_stdio(Arc::new(demi_claude_code::DemiClaude::default())).await;
    if let Err(error) = result {
        eprintln!("demi-claude-code: {error}");
        std::process::exit(1);
    }
    // The executable owns synchronous Windows stdio workers; HTTP/2 has drained.
    std::process::exit(0);
}
