//! Jobs over the runner link: their input, stops and output across a lost
//! connection.

use std::{collections::BTreeMap, time::Duration};

use demi_runner_protocol::wire::{
        self, Inbound, Outbound, WireBytes,
    };


use crate::{Host, start_job};

struct Ended {
    output: Vec<u8>,
    exit_code: Option<i32>,
}

/// Reads frames until the exit of `job`.
async fn until_exit(host: &mut Host, job: &str) -> Ended {
    let mut ended = Ended {
        output: Vec::new(),
        exit_code: None,
    };
    loop {
        match host.frame().await {
            Outbound::JobOutput { job_id, bytes, .. } if job_id == job => {
                ended.output.extend(bytes.0);
            }
            Outbound::JobExit {
                job_id, exit_code, ..
            } if job_id == job => {
                ended.exit_code = exit_code;
                return ended;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

/// Waits until something is at `path`.
async fn until_exists(path: &std::path::Path) {
    while !path.exists() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A job outlives its connection (`crates/runner/src/connection/kept.rs`): one that
/// ends while the runner is away is listed ended in the next hello, by the
/// same instance, with what it printed meanwhile counted, and its exit
/// follows the hello's answer. About a second: a login shell starts.
#[tokio::test]
async fn a_job_that_ends_while_the_connection_is_lost_is_listed_and_its_exit_follows() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        let (instance, kept) = host.kept();
        assert!(kept.is_empty(), "{kept:?}");
        start_job(
            &mut host,
            "kept",
            "printf a; while [ ! -f go ]; do sleep 0.05; done; printf bc; touch done; exit 3",
        )
        .await;
        assert!(matches!(host.frame().await, Outbound::JobOutput { .. }));

        host.cut().await;
        std::fs::write(host.home().join("go"), "").unwrap();
        until_exists(&host.home().join("done")).await;
        host.reconnected().await;

        let (again, kept) = host.kept();
        assert_eq!(again, instance, "the same runner kept its jobs");
        let [job] = kept.as_slice() else {
            panic!("{kept:?}");
        };
        assert_eq!(job.job_id, "kept");
        let ended = job.ended.as_ref().expect("the job ended");
        assert_eq!(ended.exit_code, Some(3));
        assert_eq!(job.output.stdout_bytes, 3);
        let mut host = host.online().await;
        loop {
            if let Outbound::JobExit {
                job_id,
                exit_code,
                output,
                ..
            } = host.frame().await
            {
                assert_eq!((job_id.as_str(), exit_code), ("kept", Some(3)));
                assert_eq!(output.map(|output| output.stdout_bytes), Some(3));
                break;
            }
        }
        // A runner that starts anew draws another instance and keeps none.
        host.restart().await;
        let (other, kept) = host.kept();
        assert_ne!(other, instance);
        assert!(kept.is_empty(), "{kept:?}");
        host.close().await;
    })
    .await
    .unwrap();
}

/// A connection loss stops nothing, however long it lasts. Only the backend's
/// refusal of the runner, as for a revoked device, stops its jobs
/// (`crates/runner/src/connection/kept.rs`).
///
/// The runner's attempts to connect again while it stays away mark the time
/// that passes, so the test waits for them, not for time.
///
/// Cost: about seven seconds. A login shell starts, the runner tries twice in
/// vain and is back within two, and the refused runner takes about five to end.
#[tokio::test]
async fn a_job_runs_on_through_a_connection_loss_until_the_backend_refuses_the_runner() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        // The shell creates `pid` before it writes the number, so the job
        // renames it into place: once `pid` exists, it holds the number.
        start_job(
            &mut host,
            "long",
            "sh -c 'echo $$ > pid.new; mv pid.new pid; exec sleep 60'",
        )
        .await;
        let pid = host.home().join("pid");
        until_exists(&pid).await;
        let pid = std::fs::read_to_string(&pid).unwrap().trim().to_owned();
        let alive = |pid: &str| {
            std::process::Command::new("kill")
                .args(["-0", pid])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success()
        };

        // The job goes on while the runner tries in vain to connect again,
        // and is still running when the runner is back.
        host.cut().await;
        host.drop_next().await;
        host.drop_next().await;
        host.reconnected().await;
        let (_, kept) = host.kept();
        let [job] = kept.as_slice() else {
            panic!("{kept:?}");
        };
        assert_eq!((job.job_id.as_str(), &job.ended), ("long", &None));
        assert!(alive(&pid), "the job {pid:?} runs on");

        // The backend refuses the runner: the runner stops its jobs.
        host.send(Inbound::HelloError {
            code: demi_runner_protocol::wire::HelloErrorCode::UnknownDevice,
            reason: "unknown device".into(),
        })
        .await;
        while alive(&pid) || host.process.running() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}


/// Input that arrives, and ends, right after the start of its command, before
/// the command reads a byte, reaches it: the input follows the start on the
/// command's stream, so the runner has the command when the input comes.
#[tokio::test]
async fn input_that_arrives_and_ends_with_the_start_reaches_the_command() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        // The command reads nothing for a while, so all of its input is here
        // long before it reads.
        start_job(&mut host, "late", "sleep 0.5; cat").await;
        for chunk in ["one ", "two ", "three"] {
            host.send(Inbound::JobStdin {
                job_id: "late".into(),
                bytes: WireBytes(chunk.as_bytes().to_vec()),
            })
            .await;
        }
        host.send(Inbound::JobStdinEnd {
            job_id: "late".into(),
        })
        .await;
        let ended = until_exit(&mut host, "late").await;
        assert_eq!(ended.output, b"one two three");
        assert_eq!(ended.exit_code, Some(0));
        host.close().await;
    })
    .await
    .unwrap();
}

/// A command that reads none of its input holds only the writer of that input:
/// the link reads no more of the command's stream, and the command's stop, a
/// request on a stream of its own, reaches it all the same.
#[tokio::test]
async fn a_stop_passes_input_the_command_does_not_read() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        start_job(&mut host, "deaf", "sleep 30").await;
        let outgoing = host.socket.outgoing.clone();
        let writer = tokio::spawn(async move {
            for _ in 0..400 {
                let input = wire::encode(&Inbound::JobStdin {
                    job_id: "deaf".into(),
                    bytes: WireBytes(vec![b'x'; wire::STDIN_CHUNK_BYTES]),
                })
                .unwrap();
                if outgoing.send(input).await.is_err() {
                    return;
                }
            }
        });
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!writer.is_finished(), "the command's stream holds its writer");
        host.send(Inbound::JobKill {
            job_id: "deaf".into(),
            signal: Some(wire::Signal::Kill),
        })
        .await;
        let ended = until_exit(&mut host, "deaf").await;
        assert_eq!(ended.exit_code, None);
        writer.abort();
        host.close().await;
    })
    .await
    .unwrap();
}

/// A stop is a request on a stream of its own, which can reach the runner
/// before the start it follows: the runner keeps it for the start.
#[tokio::test]
async fn a_stop_that_arrives_before_its_start_stops_the_command_when_it_starts() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        host.send(Inbound::JobKill {
            job_id: "early".into(),
            signal: Some(wire::Signal::Kill),
        })
        .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        start_job(&mut host, "early", "sleep 30").await;
        let ended = until_exit(&mut host, "early").await;
        assert_eq!(ended.exit_code, None);
        host.close().await;
    })
    .await
    .unwrap();
}

/// A chunk of live input over the limit breaks the protocol: the connection
/// ends, rather than the command waiting for input that never comes.
#[tokio::test]
async fn a_chunk_of_input_over_the_limit_ends_the_connection() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        start_job(&mut host, "big", "cat").await;
        host.send(Inbound::JobStdin {
            job_id: "big".into(),
            bytes: WireBytes(vec![b'x'; wire::STDIN_CHUNK_BYTES + 1]),
        })
        .await;
        host.ended().await;
        host.reconnected().await;
        let log = host.process.output();
        assert!(log.contains("live stdin chunk of 65537 bytes exceeds 65536 bytes"), "{log}");
        host.close().await;
    })
    .await
    .unwrap();
}
