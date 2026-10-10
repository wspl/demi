//! A declared command runs as a builtin of the job's shell: each invocation
//! goes to the handler the job supplies, with the job's context, its root,
//! its arguments and its standard input, and its output and exit code come
//! back to the script (`runner.md` § Shell jobs). The programs the job
//! starts find the job's context.

use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use demi_command_protocol::{Completion, LocalInvocation};
use demi_command_sdk::{Handler, InvocationContext, ServiceError};
use demi_runner_process::{
    command_client::{RAW, RawCommand},
    job_shell::{JobCommands, JobShell, JobStart},
};
use demi_runner_protocol::wire::OutputStream;
use demi_runner_shell::ShellRuntime;
use tokio_util::sync::CancellationToken;

/// Answers each invocation with its standard input on standard output and
/// exit code 7, and keeps what it was asked.
#[derive(Default)]
struct Recorded(Mutex<Vec<(String, RawCommand)>>);

impl Handler for Recorded {
    type Metadata = LocalInvocation;

    fn operations(&self) -> Vec<String> {
        vec![RAW.into()]
    }

    fn invoke(
        &self,
        mut context: InvocationContext<LocalInvocation>,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let command = serde_json::from_value(context.request.args.clone());
        let asked = command.map(|command| {
            self.0
                .lock()
                .unwrap()
                .push((context.request.operation.clone(), command));
        });
        Box::pin(async move {
            asked?;
            while let Some(bytes) = context.input.next().await? {
                context.output.stdout(bytes).await?;
            }
            Ok(Completion {
                exit_code: 7,
                error: None,
            })
        })
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_declared_command_reaches_the_jobs_handler() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let handler = Arc::new(Recorded::default());
        let context = "0123456789abcdef0123456789abcdef";
        let mut job = ShellRuntime::current()
            .start(JobStart {
                script: "/usr/bin/env; printf body | fixture --flag; echo \" $?\"".into(),
                cwd: root.path().into(),
                env: crate::home(root.path()),
                live: false,
                cancellation: CancellationToken::new(),
                commands: Some(JobCommands {
                    context: context.into(),
                    roots: vec!["fixture".into()],
                    handler: handler.clone(),
                }),
                edits: None,
            })
            .await
            .unwrap();
        let mut stdout = Vec::new();
        while let Some(chunk) = job.output().recv().await {
            if chunk.stream == OutputStream::Stdout {
                stdout.extend(chunk.bytes);
            }
        }
        let exit = job.wait().await;
        assert_eq!(exit.code, Some(0), "{:?}", exit.error);
        let stdout = String::from_utf8(stdout).unwrap();
        let (environment, answered) = stdout.trim_end().rsplit_once('\n').unwrap();
        let environment: BTreeMap<_, _> = environment
            .lines()
            .filter_map(|line| line.split_once('='))
            .collect();
        assert_eq!(environment.get("DEMI_CONTEXT_ID"), Some(&context));
        assert_eq!(answered, "body 7");
        let asked = handler.0.lock().unwrap();
        let [(operation, command)] = asked.as_slice() else {
            panic!("one invocation: {} of them", asked.len());
        };
        assert_eq!(operation, RAW);
        assert_eq!(
            (
                command.context.as_str(),
                command.root.as_str(),
                command.argv.as_slice(),
                command.live
            ),
            (context, "fixture", ["--flag".to_owned()].as_slice(), false)
        );
    })
    .await
    .unwrap();
}
