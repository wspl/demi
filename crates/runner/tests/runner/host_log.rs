//! The Host log (`runner.md` § Host log) as `log_read` answers it: a page
//! holds what its cursor, limit and source ask for; the numbering carries
//! across a restart and past a torn line; and long lines fill the newer
//! file, which then replaces the older one, while a cursor still reads every
//! line once, a page at a time. The runner's own lines join the log as it
//! runs, so the tests ask for the lines they wrote into its files before it
//! started, by source or by cursor.

use std::{collections::BTreeMap, path::Path, time::Duration};

use demi_runner_protocol::wire::{Inbound, LogLine, Outbound};
use serde_json::json;

use crate::Host;

/// What each of the log's two files holds (`runner.md` § Host log).
const FILE_BYTES: u64 = 4 * 1024 * 1024;
/// What a page holds.
const PAGE_BYTES: usize = 2 * 1024 * 1024;

/// One line as the log's files keep it.
fn line(seq: u64, source: &str, conversation: Option<&str>, text: &str) -> String {
    let mut line = json!({"seq": seq, "at": 1_700_000_000_000_i64, "source": source, "text": text});
    if let Some(conversation) = conversation {
        line["conversationId"] = json!(conversation);
    }
    format!("{line}\n")
}

/// Stops the runner, gives its log `older` and `newer` as its two files,
/// and starts it again.
async fn with_log(host: &mut Host, older: &str, newer: &str) {
    host.stop().await;
    let log = host.state().join("log");
    std::fs::write(log.join("host.log.1"), older).unwrap();
    std::fs::write(log.join("host.log"), newer).unwrap();
    host.restart().await;
}

async fn read(
    host: &mut Host,
    since: Option<u64>,
    limit: u64,
    source: Option<&str>,
) -> (Vec<LogLine>, u64) {
    host.send(Inbound::LogRead {
        id: "read".into(),
        since,
        limit,
        source: source.map(str::to_owned),
    })
    .await;
    loop {
        if let Outbound::LogLines { id, lines, next } = host.frame().await {
            assert_eq!(id, "read");
            return (lines, next);
        }
    }
}

fn texts(lines: &[LogLine]) -> Vec<&str> {
    lines.iter().map(|line| line.text.as_str()).collect()
}

#[tokio::test]
async fn a_page_holds_what_its_cursor_limit_and_source_ask_for() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await;
        let older = line(100, "service:demi.browser", None, "tabs failed")
            + &line(101, "stream:browser.live", Some("conversation"), "no tabs");
        // A crash left half a line at the end.
        let newer = line(102, "service:demi.browser", None, "tabs listed") + "{\"seq\":";
        with_log(&mut host, &older, &newer).await;
        let mut host = host.online().await;

        let (lines, _) = read(&mut host, None, 10, Some("service:demi.browser")).await;
        assert_eq!(texts(&lines), ["tabs failed", "tabs listed"], "one source");
        let (lines, _) = read(&mut host, None, 1, Some("service:demi.browser")).await;
        assert_eq!(texts(&lines), ["tabs listed"], "the newest of one source");
        let (lines, next) = read(&mut host, Some(0), 2, None).await;
        assert_eq!(
            (texts(&lines), next),
            (vec!["tabs failed", "no tabs"], 101),
            "from before the log"
        );
        assert_eq!(lines[1].conversation_id.as_deref(), Some("conversation"));
        let (lines, next) = read(&mut host, Some(101), 1, None).await;
        assert_eq!(
            (texts(&lines), next),
            (vec!["tabs listed"], 102),
            "one after a cursor"
        );
        // The half line is skipped, and the runner's own lines go on from
        // the last whole one.
        let (lines, next) = read(&mut host, Some(102), 1, None).await;
        assert_eq!(
            (lines[0].source.as_str(), next),
            ("runner", 103),
            "after a restart"
        );
        // A cursor ahead of the newest line comes from a log that is gone:
        // it continues from the oldest line.
        let (lines, next) = read(&mut host, Some(u64::MAX), 1, None).await;
        assert_eq!(
            (texts(&lines), next),
            (vec!["tabs failed"], 100),
            "a cursor ahead"
        );
        host.close().await;
    })
    .await
    .unwrap();
}

/// About a second: the log is filled with 4 MiB before the runner starts
/// again, and read back in three pages.
#[tokio::test]
async fn long_lines_rotate_the_files_and_a_cursor_reads_each_line_once() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await;
        // The newer file, 150 bytes short of full: the runner's first lines
        // after its restart replace the older file with it.
        let text = "x".repeat(4000);
        let mut newer = String::new();
        let mut seq = 1000;
        while newer.len() as u64 + 2 * 4100 < FILE_BYTES {
            newer += &line(seq, "service:demi.fill", None, &text);
            seq += 1;
        }
        let seeded = seq - 1000;
        let padding = FILE_BYTES as usize - newer.len() - 150;
        let empty = line(seq, "service:demi.pad", None, "").len();
        newer += &line(seq, "service:demi.pad", None, &"x".repeat(padding - empty));
        with_log(&mut host, "", &newer).await;
        let mut host = host.online().await;
        let older = host.state().join("log/host.log.1");
        rotated(&older).await;
        assert!(std::fs::metadata(&older).unwrap().len() <= FILE_BYTES);

        let mut filled = 0;
        let mut since = 0;
        loop {
            let (lines, next) = read(&mut host, Some(since), 1000, Some("service:demi.fill")).await;
            if lines.is_empty() {
                break;
            }
            let bytes: usize = lines.iter().map(|line| line.text.len()).sum();
            assert!(bytes <= PAGE_BYTES, "a page of {bytes} bytes");
            assert!(lines.iter().all(|line| line.text == text));
            filled += lines.len() as u64;
            since = next;
        }
        assert_eq!(filled, seeded);
        host.close().await;
    })
    .await
    .unwrap();
}

/// Waits until the log's older file holds what the newer one held.
async fn rotated(older: &Path) {
    while std::fs::metadata(older)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
        < FILE_BYTES / 2
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
