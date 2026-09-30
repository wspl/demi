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
