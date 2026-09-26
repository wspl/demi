//! A job's command storage: it reads and writes its node's storage at the
//! generation it recorded when it started, and nothing once that generation
//! or its call ended; a write takes its place in the node's save order
//! (`command-state-history.md` § Mutation API and concurrency, § When a
//! boundary is recorded).

use std::{cell::Cell, rc::Rc};

use demi_agent_protocol::ClientFrame;
use demi_core::Block;
use demi_provider::testing::{ScriptedRuntime, Turn, event};
use demi_shell::{JobCaller, PortError, Revision, StorageOp, StorageReply};
use futures_util::{StreamExt, stream};
use serde_json::json;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::support::{
    Fixture, command_storage, conversation, frames_until, is_idle, send, session_of, until,
};

fn write(value: serde_json::Value, expected: u64) -> StorageOp {
    StorageOp::WriteIf {
        key: "todos.json".into(),
        value: Some(value),
        expected: Some(Revision(expected)),
    }
}

#[tokio::test(flavor = "local")]
async fn a_jobs_storage_commits_at_its_generation_and_nothing_once_it_ended() {
    let script = ScriptedRuntime::new([]);
    let fixture = Fixture::new(&script);
    let client = fixture.opened().await;
    let root = conversation();
    let node = fixture.server.node(&root, &root).unwrap();
    let caller = node.job_caller();
    let storage = |caller: JobCaller, op: StorageOp, call: CancellationToken| {
        let server = fixture.server.clone();
        let root = root.clone();
        async move { server.command_storage(&root, &caller, op, call).await }
    };
    let stale = PortError::Storage("the command storage handle is no longer current".into());

    let committed = storage(
        caller.clone(),
        write(json!(["T1"]), 0),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(
        committed,
        Ok(StorageReply::Committed {
            revision: Revision(1)
        })
    );
    let read = StorageOp::Read {
        key: "todos.json".into(),
    };
    assert_eq!(
        storage(caller.clone(), read.clone(), CancellationToken::new()).await,
        Ok(StorageReply::Value {
            value: Some(json!(["T1"])),
            revision: Revision(1),
        })
    );
    let checkpoint = fixture.store.checkpoint(&root).unwrap();
    assert_eq!(checkpoint.command_state.revision, 1);

    // Another generation's number, or a call that stopped, reaches nothing.
    let other = JobCaller {
        generation: caller.generation + 1,
        ..caller.clone()
    };
    assert_eq!(
        storage(other, write(json!(["T2"]), 1), CancellationToken::new()).await,
        Err(stale.clone())
    );
    let stopped = CancellationToken::new();
    stopped.cancel();
    assert_eq!(
        storage(caller.clone(), write(json!(["T2"]), 1), stopped).await,
        Err(stale.clone())
    );

    // A disposed node's jobs reach nothing either.
    client.send(ClientFrame::Close {}).await;
    assert_eq!(
        storage(caller, read, CancellationToken::new()).await,
        Err(stale)
    );
    assert_eq!(
        fixture
            .store
            .checkpoint(&root)
            .unwrap()
            .command_state
            .revision,
        1
    );
}

/// The answer text of the root's transcript: its id, and whether it
/// completed.
fn answer(fixture: &Fixture) -> Option<(demi_core::BlockId, bool)> {
    session_of(fixture)
        .transcript()
        .blocks
        .into_iter()
        .find_map(|block| match block {
            Block::Text(text) => Some((text.id, text.forkable)),
            _ => None,
        })
}

#[tokio::test(flavor = "local")]
async fn a_write_ordered_before_an_answer_completes_is_in_its_boundary_and_a_later_one_is_not() {
    // The answer streams, then ends when the test lets it; `ended` says the
    // session has read the end of the stream.
    let (finish, finished) = oneshot::channel::<()>();
    let ended = Rc::new(Cell::new(false));
    let streaming = Turn::Stream(Box::new({
        let ended = ended.clone();
        move |_| {
            stream::iter([event::text("streaming")])
                .chain(stream::once(async move {
                    let _ = finished.await;
                    ended.set(true);
                    event::response(1, 1)
                }))
                .boxed_local()
        }
    }));
    let script = ScriptedRuntime::new([streaming]);
    let fixture = Fixture::new(&script);
    let mut client = fixture.opened().await;
    let root = conversation();
    let committed = |revision| {
        Ok(StorageReply::Committed {
            revision: Revision(revision),
        })
    };
    assert_eq!(
        command_storage(&fixture.server, &root, write(json!(["T1"]), 0)).await,
        committed(1)
    );
    client.send(send("m1", "plan it")).await;
    until(|| answer(&fixture).is_some()).await;

    // A write takes its place in the save order while the answer streams.
    let gate = fixture.store.hold_saves();
    {
        let writing = command_storage(&fixture.server, &root, write(json!(["T1", "T2"]), 1));
        tokio::pin!(writing);
        while gate.waiting() == 0 {
            assert!(futures_util::poll!(writing.as_mut()).is_pending());
            tokio::task::yield_now().await;
        }
        let _ = finish.send(());
        until(|| ended.get()).await;
        // The stream ended, and the answer waits for the write ordered
        // before it.
        assert_eq!(answer(&fixture).map(|(_, done)| done), Some(false));
        gate.release();
        assert_eq!(writing.await, committed(2));
    }
    frames_until(&mut client, is_idle).await;
    let (answer_id, done) = answer(&fixture).unwrap();
    assert!(done);

    // A later write reaches no earlier boundary: a Fork at the answer
    // starts from the version the answer completed with.
    assert_eq!(
        command_storage(&fixture.server, &root, write(json!(["T1", "T2", "T3"]), 2)).await,
        committed(3)
    );
    let seed = fixture
        .server
        .prepare_fork(&root, &answer_id)
        .await
        .unwrap();
    assert_eq!(seed.command_state.revision, 2);
}
