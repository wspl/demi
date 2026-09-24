//! Test support (`crates-and-packages.md` § command-service): the programs a
//! test finds beside itself, and a command service's binary started and
//! driven with a client.

use std::{
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
};

use tokio::{
    process::{Child, ChildStdin, ChildStdout, Command},
    task::JoinHandle,
};

use crate::{Client, ServiceError};

/// A program Cargo built into the target directory this test runs from,
/// such as a workspace crate's executable.
pub fn built_program(name: &str) -> PathBuf {
    let executable = std::env::current_exe().expect("the test knows its executable");
    let directory = executable
        .parent()
        .and_then(Path::parent)
        .expect("a test runs from the target directory's deps");
    directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

type Transport = tokio::io::Join<ChildStdout, ChildStdin>;

/// A command service's process, serving this test's client over its
/// standard input and output; its standard error is the test's. Dropping it
/// kills the process.
pub struct ServiceProcess {
    client: Client,
    child: Child,
    connection: JoinHandle<Result<(), h2::Error>>,
}

impl ServiceProcess {
    /// Starts `program` with `args` and connects to it.
    pub async fn start(program: impl AsRef<Path>, args: &[&str]) -> Result<Self, ServiceError> {
        let mut child = Command::new(program.as_ref())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = child.stdout.take().expect("the service's stdout is piped");
        let stdin = child.stdin.take().expect("the service's stdin is piped");
        let transport: Transport = tokio::io::join(stdout, stdin);
        let (client, connection) = Client::connect(transport).await?;
        Ok(Self {
            client,
            child,
            connection: tokio::spawn(connection),
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The process's ID, while it runs.
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// Asks the service to shut down, and waits for its connection to end and
    /// its process to exit.
    pub async fn shutdown(mut self) -> Result<ExitStatus, ServiceError> {
        self.client.shutdown().await?;
        let status = self.child.wait().await?;
        drop(self.client);
        self.connection.await??;
        Ok(status)
    }
}
