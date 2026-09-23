//! The tree store contract over the in-memory store (`subagents.md`
//! § Persistence): create queues the first message with the node; a save
//! delivers the completions it carries; reopen and delete; and a completion
//! of an earlier round marks nothing delivered.

use demi_agent::{
    AgentTreeStore,
    store::{
        CheckpointState, CheckpointUpdate, ClosePhase, CommandStateSnapshot, NodeClose, NodeRecord,
    },
    testing::{MemoryTreeStore, test_model, text},
};
use demi_core::{
    AgentMessage, AgentMessageBlock, AgentMessageEvent, Block, CompletionId, CompletionOutcome,
    NodeId, QueuedMessage, Sender, SessionPhase, Timestamp, TurnId, UserBlock,
};

fn id(value: &str) -> NodeId {
    NodeId::try_from(value).unwrap()
}

fn record(node: &str, parent: Option<&str>, round: u64) -> NodeRecord {
    NodeRecord {
        id: id(node),
        parent: parent.map(id),
        description: String::new(),
        profile: None,
        round,
        can_spawn_subagents: true,
        closed: None,
        delivered: false,
    }
}

fn update(queue: Vec<QueuedMessage>, blocks: Vec<Block>) -> CheckpointUpdate {
    CheckpointUpdate {
        state: CheckpointState {
            phase: SessionPhase::Idle,
            queue,
            cwd: "/w".into(),
            model: test_model(),
            harness: "test".into(),
        },
        command_state: Some(CommandStateSnapshot::initial()),
        block_count: blocks.len(),
        changed_blocks: blocks.into_iter().enumerate().collect(),
    }
}

fn message(turn: &str) -> QueuedMessage {
    QueuedMessage {
        id: TurnId::try_from(turn).unwrap(),
        content: text("brief"),
    }
}

/// The receipt a parent's transcript holds for a child's completed round.
fn receipt(child: &str, round: u64) -> Block {
    let completion = CompletionId {
        child: id(child),
        round,
    };
    let message = AgentMessage {
        id: completion.block_id(),
        sender: Sender {
            id: id(child),
            description: child.into(),
            round,
        },
        recipient_id: id("root"),
        timestamp: Timestamp::UNIX_EPOCH,
        content: format!("{child} done"),
        event: AgentMessageEvent::Completion {
            outcome: CompletionOutcome::Completed,
        },
    };
    Block::AgentMessage(AgentMessageBlock {
        id: completion.block_id(),
        turn_id: TurnId::try_from("turn").unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        message,
    })
}

fn completed(result: &str) -> NodeClose {
    NodeClose {
        phase: ClosePhase::Completed {
            result: result.into(),
        },
        at: Timestamp::UNIX_EPOCH,
    }
}

#[tokio::test(flavor = "local")]
async fn create_queues_the_first_message_with_the_node_and_a_save_replaces_it() {
    let store = MemoryTreeStore::new();
    store
        .create_node(record("root", None, 1), update(Vec::new(), Vec::new()))
        .await
        .unwrap();
    store
        .create_node(
            record("child", Some("root"), 2),
            update(vec![message("m1")], Vec::new()),
        )
        .await
        .unwrap();

    let children: Vec<NodeId> = store
        .children(&id("root"))
        .await
        .unwrap()
        .into_iter()
        .map(|node| node.id)
        .collect();
    assert_eq!(children, [id("child")]);
    let child = store.session_store(&id("child"));
    assert_eq!(
        child.load().await.unwrap().unwrap().state.queue,
        [message("m1")]
    );
    let user = Block::User(UserBlock {
        id: "u1".try_into().unwrap(),
        turn_id: TurnId::try_from("m1").unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        content: text("brief"),
        preamble: None,
    });
    child
        .save(update(Vec::new(), vec![user]), &Default::default())
        .await
        .unwrap();
    let loaded = child.load().await.unwrap().unwrap();
    assert!(loaded.state.queue.is_empty());
    assert_eq!(loaded.transcript.len(), 1);
    assert!(
        store
            .create_node(
                record("child", Some("root"), 3),
                update(Vec::new(), Vec::new())
            )
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "local")]
async fn a_save_delivers_the_completions_its_transcript_carries_and_only_those() {
    let store = MemoryTreeStore::new();
    store
        .create_node(record("root", None, 1), update(Vec::new(), Vec::new()))
        .await
        .unwrap();
    for child in ["a", "b"] {
        store
            .create_node(
                record(child, Some("root"), 1),
                update(Vec::new(), Vec::new()),
            )
            .await
            .unwrap();
        store
            .close_node(&id(child), completed(&format!("{child} done")))
            .await
            .unwrap();
    }

    let save = update(Vec::new(), vec![receipt("a", 1)]);
    assert_eq!(
        save.carried_completions().unwrap(),
        [CompletionId {
            child: id("a"),
            round: 1
        }]
    );
    store
        .session_store(&id("root"))
        .save(save, &Default::default())
        .await
        .unwrap();

    assert!(store.record(&id("a")).unwrap().delivered);
    assert!(!store.record(&id("b")).unwrap().delivered);
    store.mark_delivered(&id("b"), 1).await.unwrap();
    assert!(store.record(&id("b")).unwrap().delivered);
}

#[tokio::test(flavor = "local")]
async fn reopen_makes_a_closed_node_live_with_its_message_and_delete_takes_the_subtree() {
    let store = MemoryTreeStore::new();
    store
        .create_node(record("root", None, 1), update(Vec::new(), Vec::new()))
        .await
        .unwrap();
    store
        .create_node(
            record("child", Some("root"), 1),
            update(Vec::new(), Vec::new()),
        )
        .await
        .unwrap();
    store
        .create_node(
            record("grandchild", Some("child"), 1),
            update(Vec::new(), Vec::new()),
        )
        .await
        .unwrap();
    let failed = NodeClose {
        phase: ClosePhase::Error {
            failure: "boom".into(),
        },
        at: Timestamp::UNIX_EPOCH,
    };
    store
        .close_node(&id("child"), failed.clone())
        .await
        .unwrap();
    assert_eq!(store.record(&id("child")).unwrap().closed, Some(failed));

    store
        .reopen_node(&id("child"), 9, message("m2"))
        .await
        .unwrap();
    let reopened = store.record(&id("child")).unwrap();
    assert_eq!(
        (reopened.closed, reopened.round, reopened.delivered),
        (None, 9, false)
    );
    assert_eq!(
        store.checkpoint(&id("child")).unwrap().state.queue,
        [message("m2")]
    );

    store.delete_node(&id("child")).await.unwrap();
    assert!(store.record(&id("child")).is_none());
    assert!(store.record(&id("grandchild")).is_none());
    assert!(store.record(&id("root")).is_some());
}

#[tokio::test(flavor = "local")]
async fn a_completion_of_an_earlier_round_marks_the_current_one_undelivered() {
    let store = MemoryTreeStore::new();
    store
        .create_node(record("root", None, 1), update(Vec::new(), Vec::new()))
        .await
        .unwrap();
    store
        .create_node(
            record("child", Some("root"), 1),
            update(Vec::new(), Vec::new()),
        )
        .await
        .unwrap();
    store
        .close_node(&id("child"), completed("first"))
        .await
        .unwrap();
    store
        .reopen_node(&id("child"), 3, message("again"))
        .await
        .unwrap();
    store
        .close_node(&id("child"), completed("second"))
        .await
        .unwrap();

    let old = update(Vec::new(), vec![receipt("child", 1)]);
    store
        .session_store(&id("root"))
        .save(old, &Default::default())
        .await
        .unwrap();
    store.mark_delivered(&id("child"), 1).await.unwrap();
    assert!(!store.record(&id("child")).unwrap().delivered);

    let current = update(Vec::new(), vec![receipt("child", 1), receipt("child", 3)]);
    store
        .session_store(&id("root"))
        .save(current, &Default::default())
        .await
        .unwrap();
    assert!(store.record(&id("child")).unwrap().delivered);
}
