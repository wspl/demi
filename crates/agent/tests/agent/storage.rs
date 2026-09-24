//! A job's command storage: it reads and writes its node's storage at the
//! generation it recorded when it started, and nothing once that generation
//! or its call ended (`command-state-history.md` § Mutation API and
//! concurrency).

use demi_agent_protocol::ClientFrame;
use demi_provider::testing::ScriptedRuntime;
use demi_shell::{JobCaller, PortError, Revision, StorageOp, StorageReply};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::support::{Fixture, conversation};

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
