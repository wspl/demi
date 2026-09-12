use std::sync::Arc;

#[cfg(unix)]
fn stdio() -> std::io::Result<impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> {
    use std::os::fd::AsFd;
    use tokio::net::unix::pipe::{Receiver, Sender};
    Ok(tokio::io::join(
        Receiver::from_owned_fd(std::io::stdin().as_fd().try_clone_to_owned()?)?,
        Sender::from_owned_fd(std::io::stdout().as_fd().try_clone_to_owned()?)?,
    ))
}

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() != Some("--command-service") {
        eprintln!("Usage: demi-commands --command-service");
        std::process::exit(2);
    }
    let result = match stdio() {
        Ok(io) => {
            demi_command_service::serve(io, Arc::new(demi_builtin_package::DemiCommands::default()))
                .await
        }
        Err(error) => {
            eprintln!("demi-commands: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = result {
        eprintln!("demi-commands: {error}");
        // A fatal native handler fault cannot be repaired within a resident process.
        std::process::exit(1);
    }
}
