use std::sync::Arc;

// Invocations are rare and wait on the network and the disk.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    // A panic leaves its report on standard error
    // (`crates/xtask/src/native/build.rs`).
    demi_shared_cli::install_panic_hook();
    demi_command_sdk::Launch::from_process();
    // Events go to standard error as lines of the log, which the runner keeps
    // as they are.
    demi_command_sdk::log_to_stderr("claude_code");
    let result =
        demi_command_sdk::serve_stdio(Arc::new(demi_claude_code::DemiClaude::default())).await;
    if let Err(error) = result {
        tracing::error!(event = "claude_code_service.exited", %error, "the service ended with an error");
        std::process::exit(1);
    }
    // The executable owns synchronous Windows stdio workers; HTTP/2 has drained.
    std::process::exit(0);
}
