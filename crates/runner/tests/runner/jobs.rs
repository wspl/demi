//! Shell jobs as the backend runs them (`runner.md` § Shell jobs): large
//! output through functions and compound pipelines drains, and a kill ends a
//! job that blocks in a utility.

use std::{collections::BTreeMap, time::Duration};

use demi_runner_protocol::wire::{Inbound, Outbound, OutputStream, Signal};

use crate::{Host, start_job};

#[tokio::test]
async fn functions_and_compound_pipelines_drain_large_output_and_here_documents() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        std::fs::write(host.home().join("input"), vec![b'x'; 262_144]).unwrap();
        start_job(
            &mut host,
            "pipeline",
            "producer() { cat input; }; value=$(producer | cat | cat); printf '%s\\n' \"${#value}\"; { producer; } | wc -c; (producer) | wc -c; cat <<EOF | wc -c\n$value\nEOF\ncat <<< \"$value\" | wc -c",
        )
        .await;
        let mut output = Vec::new();
        loop {
            match host.frame().await {
                Outbound::JobOutput { stream, bytes, .. } => {
                    assert_eq!(stream, OutputStream::Stdout, "{:?}", bytes.0);
                    output.extend(bytes.0);
                }
                Outbound::JobExit {
                    exit_code, signal, ..
                } => {
                    assert_eq!(exit_code, Some(0), "{signal:?}");
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        let output = String::from_utf8(output).unwrap();
        assert_eq!(
            output.split_whitespace().collect::<Vec<_>>(),
            ["262144", "262144", "262144", "262145", "262145"]
        );
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_terminates_a_blocking_native_builtin() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        start_job(&mut host, "job", "printf ready; sleep 60").await;
        assert!(matches!(host.frame().await, Outbound::JobOutput { .. }));
        host.send(Inbound::JobKill {
            job_id: "job".into(),
            signal: Some(Signal::Kill),
        })
        .await;
        loop {
            if let Outbound::JobExit { job_id, .. } = host.frame().await {
                assert_eq!(job_id, "job");
                break;
            }
        }
        host.close().await;
    })
    .await
    .unwrap();
}

/// Waits until something is at `path`.
async fn until_exists(path: &std::path::Path) {
    while !path.exists() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A job outlives its connection (`runner.md` § Command lifetime): one that
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
        assert_eq!((ended.exit_code, ended.unreached), (Some(3), false));
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

/// A connection that stays away past the grace stops the runner's jobs as a
/// stop does, and the next hello lists them ended for that reason
/// (`runner.md` § Command lifetime). The test's runner keeps its jobs 200 ms
/// instead of 10 minutes.
#[tokio::test]
async fn a_job_kept_past_the_grace_is_stopped_and_listed_as_unreached() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let env = BTreeMap::from([("DEMI_UNREACHED_GRACE_MS".to_owned(), "200".to_owned())]);
        let mut host = Host::start(env).await.online().await;
        start_job(&mut host, "long", "sh -c 'echo $$ > pid; exec sleep 60'").await;
        let pid = host.home().join("pid");
        until_exists(&pid).await;
        let pid = std::fs::read_to_string(&pid).unwrap().trim().to_owned();
        let alive = |pid: &str| {
            std::process::Command::new("kill")
                .args(["-0", pid])
                .status()
                .unwrap()
                .success()
        };

        host.cut().await;
        while alive(&pid) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        host.reconnected().await;

        let (_, kept) = host.kept();
        let [job] = kept.as_slice() else {
            panic!("{kept:?}");
        };
        assert_eq!(job.job_id, "long");
        assert!(
            job.ended.as_ref().is_some_and(|ended| ended.unreached),
            "{job:?}"
        );
        host.close().await;
    })
    .await
    .unwrap();
}
