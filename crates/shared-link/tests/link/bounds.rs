//! What a runner that is broken or hostile cannot make the backend hold or
//! do: every stream it opens, every message it announces and every byte it
//! leaves unread is bounded.

use std::{cell::Cell, rc::Rc, time::Duration};

use demi_shared_link::{SocketEnd, testing};
use futures_util::StreamExt as _;
use tokio::io::AsyncWriteExt as _;

use super::HANG_GUARD;

/// The bytes that open a stream of lane `kind` named `id`.
fn lane(kind: u8, id: &str) -> Vec<u8> {
    let mut header = vec![kind];
    header.extend_from_slice(&(id.len() as u16).to_be_bytes());
    header.extend_from_slice(id.as_bytes());
    header
}

async fn broken(backend: &mut demi_shared_link::Connection) -> String {
    let ended = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
        .await
        .expect("the link ends within the hang guard");
    match ended {
        Some(Err(SocketEnd::Broken(reason))) => reason,
        other => panic!("expected a broken link, got {other:?}"),
    }
}

/// A frame longer than a message may be breaks the link before a byte of it
/// is read.
#[tokio::test(flavor = "local")]
async fn a_frame_beyond_the_message_limit_breaks_the_link() {
    let (runner, mut backend, _network) = testing::pair(None);
    let mut stream = runner.outgoing.rogue_stream(&lane(1, "job")).await.unwrap();
    stream.write_all(&(5u32 * 1024 * 1024).to_be_bytes()).await.unwrap();
    assert!(broken(&mut backend).await.contains("over the"));
}

/// More streams than a link has room for break it, however small they are.
#[tokio::test(flavor = "local")]
async fn more_streams_than_a_link_allows_break_it() {
    let (runner, mut backend, _network) = testing::pair(None);
    let mut streams = Vec::new();
    for index in 0..1100 {
        match runner.outgoing.rogue_stream(&lane(1, &format!("job-{index}"))).await {
            Ok(stream) => streams.push(stream),
            Err(_) => break,
        }
    }
    assert!(broken(&mut backend).await.contains("streams"));
}

/// Messages that nobody takes are held up to a budget, however many streams
/// carry them: the senders wait once it is spent, and the link goes on.
#[tokio::test(flavor = "local", start_paused = true)]
async fn the_bytes_a_runner_can_leave_unread_are_bounded() {
    let (runner, mut backend, _network) = testing::pair(None);
    let written = Rc::new(Cell::new(0usize));
    let mut writers = Vec::new();
    for index in 0..16 {
        let mut stream = runner
            .outgoing
            .rogue_stream(&lane(1, &format!("job-{index}")))
            .await
            .unwrap();
        let written = written.clone();
        writers.push(tokio::task::spawn_local(async move {
            let piece = vec![0u8; 16 * 1024];
            // Messages of 4 MiB, over and over: 64 MiB for each stream.
            for _ in 0..16 {
                if stream.write_all(&(4u32 * 1024 * 1024).to_be_bytes()).await.is_err() {
                    return;
                }
                for _ in 0..(4 * 1024 / 16) {
                    if stream.write_all(&piece).await.is_err() {
                        return;
                    }
                    written.set(written.get() + piece.len());
                }
            }
        }));
    }
    let mut previous = usize::MAX;
    while previous != written.get() {
        previous = written.get();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let mib = written.get() / (1024 * 1024);
    // The budget, the first window of each stream and what the link allows
    // its windows beyond that, at their start and by growth: 32 + 4 + 64 MiB,
    // nowhere near the 1,024 MiB sent.
    assert!(mib < 116, "{mib} MiB were taken from streams nobody read");
    assert!(mib >= 16, "{mib} MiB: the budget itself was not used");
    assert!(writers.iter().any(|writer| !writer.is_finished()));
    // The link still carries what is read: taking one message opens room.
    let taken = tokio::time::timeout(HANG_GUARD, backend.incoming.next())
        .await
        .expect("a message arrives")
        .unwrap()
        .unwrap();
    assert_eq!(taken.len(), 4 * 1024 * 1024);
    drop(runner);
}
