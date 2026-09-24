//! The backend scenarios' scripted machine manager as a program, for suites
//! that start the backend executable, such as the browser-contract suite
//! (`scenarios.md` § Browser-contract suite): the backend reconciles with a
//! manager before it serves. The program prints the manager's socket path as
//! its first line and serves until its standard input closes or it is
//! terminated. It then ends, and every runner it started ends with it.

// The scenarios' own module; the program uses only part of it.
#[allow(dead_code)]
#[path = "../tests/backend/machines.rs"]
mod machines;

use std::io::Read as _;

use tokio::signal::unix::{SignalKind, signal};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let manager = machines::ScriptedManager::start();
    println!("{}", manager.socket().display());
    // The input closes when the process that started this one ends, however
    // it ends. A plain thread reads it: the runtime would wait for a blocking
    // task at its end, while the process ends this thread.
    let (closed, input_closed) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut input = std::io::stdin();
        let mut ignored = [0; 64];
        while matches!(input.read(&mut ignored), Ok(read) if read > 0) {}
        // Nothing waits for it once a signal ended the program first.
        let _ = closed.send(());
    });
    let mut terminate = signal(SignalKind::terminate()).expect("SIGTERM can be handled");
    tokio::select! {
        _ = input_closed => {}
        _ = terminate.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    // The manager's runners go with its state, once the runtime drops the
    // tasks that share it; dropping a runner kills its process.
    drop(manager);
}
