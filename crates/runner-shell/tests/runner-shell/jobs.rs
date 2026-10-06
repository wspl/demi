//! A job's shell as the runner starts it (`runner.md` § Shell jobs,
//! § Cancellation and completion): a cancelled job reports the signal that
//! asked for its end, ends with nothing left running wherever it blocks,
//! while a sibling in the same process goes on, and a finished job keeps
//! the output of its process substitutions.

use std::time::Duration;

use demi_runner_process::{job_shell::ShellJob, process::ProcessInput};
use demi_runner_protocol::wire::{OutputStream, Signal};
use demi_runner_shell::{
    ShellRuntime,
    testing::{Job, Scope},
};
use tokio_util::sync::CancellationToken;

/// Waits for a shell job's complete readiness marker across output frames.
async fn wait_for_job_ready(job: &mut Job) {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut ready = Vec::new();
        while ready.len() < 5 {
            let chunk = job.output.recv().await.expect("job exited before ready");
            ready.extend(chunk.bytes);
        }
        assert_eq!(ready, b"ready");
    })
    .await
    .unwrap();
}

/// Each signal runs in a job of its own, and the jobs run together: each is a
/// login shell that reads the machine's profile first.
#[tokio::test]
async fn shell_cancellation_reports_the_requesting_signal() {
    let signals = [
        Some(Signal::Terminate),
        Some(Signal::Interrupt),
        Some(Signal::Hangup),
        Some(Signal::Quit),
        Some(Signal::Kill),
        None,
    ]
    .map(|signal| async move {
        let root = tempfile::tempdir().unwrap();
        let scope = Scope::new(CancellationToken::new(), None);
        let mut job = Job::start(
            "printf ready; sleep 60".into(),
            root.path().into(),
            root.path().into(),
            crate::home(root.path()),
            true,
            true,
            scope.clone(),
            &ShellRuntime::current(),
        )
        .await
        .unwrap();
        wait_for_job_ready(&mut job).await;
        assert!(job.signal(Signal::User1).is_err());
        assert!(!job.is_cancelled());
        if let Some(signal) = signal {
            job.signal(signal).unwrap();
            // Cleanup or repeated requests cannot replace the original cause.
            job.signal(Signal::Kill).unwrap();
        } else {
            job.cancel();
            job.signal(Signal::Terminate).unwrap();
        }
        let (exit, _) = tokio::time::timeout(Duration::from_secs(3), job.wait())
            .await
            .unwrap();
        let expected = signal.map_or("SIGKILL".to_owned(), |signal| signal.to_string());
        assert_eq!(exit.signal, Some(expected));
        // Bash's own exit code is its interruption's, not the job's.
        assert_eq!(exit.code, None);
        assert!(exit.error.is_none());
        assert_eq!(scope.tasks.len(), 0);
    });
    futures_util::future::join_all(signals).await;
}

/// Waits until the job of `scope` blocks or loops: one of its units waits
/// inside an interruptible read, write or sleep, or the job checks for
/// cancellation another hundred times, as a loop does at every step.
async fn blocked_or_looping(scope: &Scope, script: &str) {
    let activity = scope.activity();
    let checks = activity.checks();
    tokio::time::timeout(Duration::from_secs(60), async {
        while activity.waiting() == 0 && activity.checks() < checks + 100 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{script} neither blocks nor loops"));
}

/// A job that blocks, in any of the ways below, ends with `SIGKILL` and no
/// work left when it is cancelled there. The blocked jobs run at once beside
/// a sibling job, which prints the runner's process as its `$$`, waits in
/// `read` through every cancellation and then finishes as it would alone.
/// About 4 s: each way of blocking needs a job of its own, and each job is a
/// login shell that reads the machine's profile first (about 0.4 s in the
/// Linux container, with nvm), so the jobs start together.
///
/// The jobs run on a shell runtime of their own, as in the runner, which the
/// test shuts down without waiting: a unit that ignores cancellation fails
/// the test when its job does not end, rather than holding the test's
/// runtime, which would wait for it at shutdown.
#[tokio::test]
async fn jobs_share_the_runner_process_and_cancellation_is_isolated() {
    // Left undropped when the test fails, since dropping waits for its units.
    let shell = std::mem::ManuallyDrop::new(ShellRuntime::build().unwrap());
    let runtime = &ShellRuntime::new(&shell);
    let root = tempfile::tempdir().unwrap();
    let mut sibling = Job::start(
        "printf '%s' $$; read go; printf done".into(),
        root.path().into(),
        root.path().into(),
        crate::home(root.path()),
        false,
        true,
        Scope::new(CancellationToken::new(), None),
        runtime,
    )
    .await
    .unwrap();
    let process = std::process::id().to_string();
    let mut printed = Vec::new();
    while printed.len() < process.len() {
        let chunk = tokio::time::timeout(Duration::from_secs(60), sibling.output.recv())
            .await
            .unwrap()
            .expect("the sibling prints its process");
        assert!(matches!(chunk.stream, OutputStream::Stdout));
        printed.extend(chunk.bytes);
    }
    assert_eq!(String::from_utf8(printed).unwrap(), process);
    let blocked = [
        "while :; do :; done",
        "printf line | sed ':again; b again'",
        "jq -n 'def forever: forever; forever'",
        "cat",
        "tee file",
        "wc -c",
        "head -c 99999",
        "tail -c +1",
        "od -j 99999",
        "(sleep 60) & wait",
        "cat <(sleep 60)",
        "touch file; tail -f -s 60 file",
    ]
    .map(|script| async move {
        let root = tempfile::tempdir().unwrap();
        let scope = Scope::new(CancellationToken::new(), None);
        let mut job = Job::start(
            format!("printf ready; {script}"),
            root.path().into(),
            root.path().into(),
            crate::home(root.path()),
            true,
            true,
            scope.clone(),
            runtime,
        )
        .await
        .unwrap();
        wait_for_job_ready(&mut job).await;
        blocked_or_looping(&scope, script).await;
        job.cancel();
        let (exit, _) = tokio::time::timeout(Duration::from_secs(3), job.wait())
            .await
            .expect(script);
        assert_eq!(exit.signal.as_deref(), Some("SIGKILL"), "{script}");
        assert_eq!(scope.tasks.len(), 0, "{script}");
    });
    futures_util::future::join_all(blocked).await;
    sibling
        .input
        .send(ProcessInput::Bytes(bytes::Bytes::from_static(b"go\n")))
        .await
        .unwrap();
    sibling.input.send(ProcessInput::End).await.unwrap();
    let mut rest = Vec::new();
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(3), sibling.output.recv())
        .await
        .unwrap()
    {
        assert!(matches!(chunk.stream, OutputStream::Stdout));
        rest.extend(chunk.bytes);
    }
    assert_eq!(rest, b"done");
    let (exit, _) = sibling.wait().await;
    assert_eq!(exit.code, Some(0), "{:?}", exit.error);
    std::mem::ManuallyDrop::into_inner(shell).shutdown_background();
}

/// Each utility runs in a job of its own, and the jobs run together: each is
/// a login shell that reads the machine's profile first.
#[cfg(unix)]
#[tokio::test]
async fn cancellation_reaps_external_programs_started_by_native_utilities() {
    let scripts = [
        "/bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60'",
        "printf x | xargs /bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60'",
        "find . -maxdepth 0 -exec /bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60' ';'",
        "find . -maxdepth 0 -exec /bin/sh -c 'echo $$ > child.pid; exec /bin/sleep 60' sh '{}' +",
        "printf x | sed 'e echo $$ > child.pid; exec /bin/sleep 60'",
    ]
    .map(|script| async move {
        let root = tempfile::tempdir().unwrap();
        let scope = Scope::new(CancellationToken::new(), None);
        let mut job = Job::start(
            script.into(),
            root.path().into(),
            root.path().into(),
            crate::home(root.path()),
            false,
            true,
            scope.clone(),
            &ShellRuntime::current(),
        )
        .await
        .unwrap();
        let pid = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(root.path().join("child.pid")).await
                    && let Ok(pid) = text.trim().parse::<i32>()
                {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect(script);
        job.cancel();
        let (exit, _) = tokio::time::timeout(Duration::from_secs(3), job.wait())
            .await
            .expect(script);
        assert_eq!(exit.signal.as_deref(), Some("SIGKILL"));
        assert_eq!(scope.tasks.len(), 0);
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "child {pid} survived: {script}"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    });
    futures_util::future::join_all(scripts).await;
}

#[tokio::test]
async fn job_completion_preserves_process_substitution_output() {
    let root = tempfile::tempdir().unwrap();
    let content = vec![b'x'; 256 * 1024];
    std::fs::write(root.path().join("input"), &content).unwrap();
    let mut job = Job::start(
        "cat input | tee >(sleep 0.1; cat > copied) > /dev/null".into(),
        root.path().into(),
        root.path().into(),
        crate::home(root.path()),
        false,
        true,
        Scope::new(CancellationToken::new(), None),
        &ShellRuntime::current(),
    )
    .await
    .unwrap();
    let (exit, _) = tokio::time::timeout(Duration::from_secs(5), job.wait())
        .await
        .unwrap();
    assert_eq!(exit.code, Some(0), "{:?}", exit.error);
    assert_eq!(std::fs::read(root.path().join("copied")).unwrap(), content);
}
