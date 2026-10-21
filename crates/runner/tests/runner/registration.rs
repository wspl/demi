//! The runner's reconnects (`crates/runner/src/connection/mod.rs`).

use std::{collections::BTreeMap, time::Duration};

use demi_runner_protocol::wire::{Inbound, Outbound};

use crate::Host;

/// A connection that was online and then drops without a close, as a network
/// that goes away does, is retried after the first delay however long the
/// backend stayed away before it. Its error reaches the Host log each time,
/// though it is the same error as the failures before
/// (`crates/runner/src/host_log.rs`).
///
/// Cost: about 5 s, since four failed connections grow the delay to 4 s before
/// the runner goes online.
#[tokio::test]
async fn an_online_connection_that_fails_starts_its_retries_and_its_log_over() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await;
        let mut waited = Duration::ZERO;
        for _ in 0..4 {
            host.cut().await;
            let cut = tokio::time::Instant::now();
            host.reconnected().await;
            waited = cut.elapsed();
        }
        let grown = waited;
        for _ in 0..2 {
            host.welcome(&[crate::CONVERSATION]).await;
            // The runner's job count says it took the welcome, which the cut
            // would otherwise race.
            assert!(matches!(
                host.frame_with_counts().await,
                Outbound::JobCount { .. }
            ));
            host.cut().await;
            let cut = tokio::time::Instant::now();
            host.reconnected().await;
            let waited = cut.elapsed();
            assert!(
                waited < grown / 2,
                "reconnected {waited:?} after an online connection failed, {grown:?} before it"
            );
        }
        host.welcome(&[crate::CONVERSATION]).await;
        // The failures before the runner was first online are one line, and
        // each online connection's failure is one more.
        let failures = logged_failures(&mut host, 3).await;
        assert_eq!(failures.len(), 3, "{failures:?}");
        host.close().await;
    })
    .await
    .unwrap();
}

/// The Host log's `connection ended` lines, read once it holds the
/// runner's `online` line of its `onlines`th connection, which comes after
/// every failure before it.
async fn logged_failures(host: &mut Host, onlines: usize) -> Vec<String> {
    loop {
        host.send(Inbound::LogRead {
            id: "log".into(),
            since: None,
            limit: 1000,
            source: Some("runner".into()),
        })
        .await;
        let Outbound::LogLines { lines, .. } = host.frame().await else {
            panic!("expected log_lines");
        };
        if lines.iter().filter(|line| line.text == "online").count() >= onlines {
            return lines
                .into_iter()
                .filter(|line| line.text.starts_with("connection ended: "))
                .map(|line| line.text)
                .collect();
        }
        // The log answers from its files and waits for no queued line.
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
