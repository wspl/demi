#![cfg(unix)]

use bytes::Bytes;
use demi_runner::process::{ChildProcess, OutputStream, ProcessInput, SpawnOptions};
use std::{collections::BTreeMap, time::Duration};

fn options(command: &str, args: &[&str]) -> SpawnOptions {
    SpawnOptions {
        command: command.into(),
        args: args.iter().map(|arg| (*arg).into()).collect(),
        cwd: std::env::temp_dir(),
        env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        process_group: true,
    }
}

#[tokio::test]
async fn child_streams_binary_before_stdin_eof_and_reaps() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut child = ChildProcess::spawn(options("/bin/cat", &[])).await.unwrap();
        child
            .input
            .send(ProcessInput::Bytes(Bytes::from_static(&[0, 255, 128, 10])))
            .await
            .unwrap();
        let chunk = child.output.recv().await.unwrap();
        assert!(matches!(chunk.stream, OutputStream::Stdout));
        assert_eq!(chunk.bytes.as_ref(), [0, 255, 128, 10]);
        child.input.send(ProcessInput::End).await.unwrap();
        assert!(child.output.recv().await.is_none());
        let exit = child.wait().await;
        assert_eq!(exit.code, Some(0));
        assert!(exit.error.is_none(), "{:?}", exit.error);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_interrupts_output_backpressure() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut child = ChildProcess::spawn(options("/usr/bin/yes", &[]))
            .await
            .unwrap();
        child.output.recv().await.unwrap();
        // Leave the bounded output queue full while the owner receives cancellation.
        tokio::time::sleep(Duration::from_millis(50)).await;
        child.cancel();
        let exit = child.wait().await;
        assert_eq!(exit.signal.as_deref(), Some("SIGKILL"));
        assert!(exit.error.is_none(), "{:?}", exit.error);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_kills_descendant_process_group() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut child = ChildProcess::spawn(options(
            "/bin/sh",
            &["-c", "sleep 30 & printf '%s\\n' $!; wait"],
        ))
        .await
        .unwrap();
        let output = child.output.recv().await.unwrap();
        let descendant: i32 = std::str::from_utf8(&output.bytes)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        child.cancel();
        child.wait().await;
        loop {
            // PID came from this test's child and is checked read-only.
            if unsafe { libc::kill(descendant, 0) } != 0 {
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::ESRCH)
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn spawn_failure_distinguishes_missing_cwd_from_missing_executable() {
    let failure = ChildProcess::spawn(options("/definitely-not-a-demi-test-program", &[]))
        .await
        .err()
        .unwrap();
    assert_eq!(failure.kind, "executable_not_found");
    let mut request = options("/bin/true", &[]);
    request.cwd = std::env::temp_dir().join("definitely-not-a-demi-test-cwd");
    let failure = ChildProcess::spawn(request).await.err().unwrap();
    assert_eq!(failure.kind, "cwd_unusable");
}
