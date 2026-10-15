//! A root's turn a graceful shutdown interrupted (`failures-and-recovery.md`
//! § The unfinished turn): once the backend is back it resumes by itself,
//! without a page, with the child it waited on, each once, the model told
//! the backend restarted, and a later restart resumes nothing again; with
//! the user's `resumeAfterRestart` off both wait for the user, whose Resume
//! works; and a crash's interruption never resumes by itself. The model is
//! the scripted tree family; the device is a real runner, which the child's
//! spawn runs on. No test calls a real model.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use demi_backend_user_shard::shard::ShardPlacement;
use demi_conversation_socket_protocol::ClientFrame;
use demi_shared_types::Block;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, Socket, send, transcript};
use crate::subagents::{Scripts, hang, say, shell, tree_on};
use crate::support::{Harness, Session, TestBackend, eventually};

const REASON: &str = "Demi resumed this turn after the backend restarted.";
const RESUMED: &str = "a turn the shutdown interrupted resumed";
const DECLINED: &str = "a turn the shutdown interrupted waits for the user";

/// A backend whose conversation `FIRST` runs on a paired device with the
/// scripted tree family, with its shards on the test's thread, whose log
/// the test reads.
struct World {
    harness: Harness,
    backend: TestBackend,
    master: Session,
    scripts: Arc<Scripts>,
    _paired: crate::support::Paired,
    log: PathBuf,
    _logs: tempfile::TempDir,
    _subscriber: tracing::subscriber::DefaultGuard,
}

impl World {
    async fn start() -> Self {
        let logs = tempfile::tempdir().unwrap();
        let log = logs.path().join("backend.log");
        let file = Arc::new(std::fs::File::create(&log).unwrap());
        let subscriber =
            tracing::subscriber::set_default(tracing_subscriber::fmt().with_writer(file).finish());
        let scripts = Arc::new(Scripts::default());
        let mut harness = Harness::new();
        harness.shards = Some(ShardPlacement::Inline);
        let (harness, backend, master, paired, _root, _provider) = tree_on(&scripts, harness).await;
        Self {
            harness,
            backend,
            master,
            scripts,
            _paired: paired,
            log,
            _logs: logs,
            _subscriber: subscriber,
        }
    }

    /// How often the log says `line`.
    fn said(&self, line: &str) -> usize {
        std::fs::read_to_string(&self.log).unwrap().matches(line).count()
    }

    /// What the root's requests carried, in order.
    fn root_asked(&self) -> Vec<String> {
        self.scripts.asked(|session| session == FIRST)
    }

    /// What the child's requests carried, in order.
    fn child_asked(&self) -> Vec<String> {
        self.scripts.asked(|session| session != FIRST)
    }

    /// The root spawns a child, and both wait for the model, which never
    /// answers, until the backend goes.
    async fn run_a_root_and_its_child(&self) {
        let mut socket = Socket::connect(&self.backend, &self.master, FIRST).await;
        socket.open().await;
        self.scripts.child(vec![hang()]);
        self.scripts.root(
            FIRST,
            vec![
                shell("t1", "demi agent spawn --description worker <<< 'Work on it'"),
                hang(),
            ],
        );
        socket.send(&send("m1", "Start a worker")).await;
        eventually("the root and its child wait for the model", || async {
            self.root_asked().len() == 2 && self.child_asked().len() == 1
        })
        .await;
    }

    /// Closes the backend and starts it again on its address, which the
    /// device's runner reconnects to.
    async fn restart(mut self) -> Self {
        let address = self.backend.address();
        self.backend.close().await;
        self.backend = self.harness.start_at(address).await;
        self.master = self
            .backend
            .login(crate::support::MASTER_EMAIL, crate::support::MASTER_PASSWORD)
            .await;
        self
    }

    async fn prefer(&self, on: bool) {
        let turned = self
            .backend
            .patch("/api/settings/preferences", &self.master, json!({ "resumeAfterRestart": on }))
            .await;
        assert_eq!(turned.status, StatusCode::OK);
    }

    fn last_block(blocks: &[Block]) -> &Block {
        blocks.last().expect("the transcript holds blocks")
    }
}

/// With the preference absent, a root's turn and its child's, which the
/// shutdown interrupted, resume once the backend is back, without a page:
/// the root's next request says why, once, and each asks the model once
/// more. The root's resumed turn finishes, so the next restart resumes
/// nothing: the user's next message is the next thing the root asks.
// Several seconds: two restarts with a real device, whose runner connects
// again after each.
#[tokio::test(flavor = "local")]
async fn a_turn_the_shutdown_interrupted_resumes_with_its_child_once_the_backend_is_back() {
    let mut world = World::start().await;
    world.run_a_root_and_its_child().await;

    world.scripts.root(FIRST, vec![say("picked up again")]);
    world.scripts.child_again(vec![hang()]);
    world = world.restart().await;
    eventually("the root and its child asked again", || async {
        world.root_asked().len() == 3 && world.child_asked().len() == 2
    })
    .await;
    let resumed = &world.root_asked()[2];
    assert_eq!(resumed.matches(REASON).count(), 1, "{resumed}");
    eventually("the resumed turn ended", || async {
        let blocks = transcript(&world.backend, &world.master, FIRST).await.blocks;
        matches!(World::last_block(&blocks), Block::Response(_))
    })
    .await;
    assert_eq!(world.said(RESUMED), 1);

    // The child waits for the model again; the root rests.
    world.scripts.child_again(vec![hang()]);
    world = world.restart().await;
    let mut socket = Socket::connect(&world.backend, &world.master, FIRST).await;
    socket.open().await;
    world.scripts.root(FIRST, vec![say("here")]);
    socket.chat("m2", "Still there?").await;
    let asked = world.root_asked();
    assert_eq!(asked.len(), 4);
    let last = &asked[3];
    assert!(last.contains("Still there?"), "{last}");
    assert_eq!(last.matches(REASON).count(), 1, "one resume in the history: {last}");
    assert_eq!(world.said(RESUMED), 1);
    world.backend.close().await;
}

/// With the preference off, nothing resumes after the restart; the
/// conversation offers Resume, and the user's Resume continues the root's
/// turn, with no word of a restart.
// A few seconds: one restart with a real device.
#[tokio::test(flavor = "local")]
async fn with_the_preference_off_the_interrupted_turn_waits_for_the_users_resume() {
    let mut world = World::start().await;
    world.prefer(false).await;
    world.run_a_root_and_its_child().await;

    world = world.restart().await;
    eventually("the start declined the turn", || async { world.said(DECLINED) == 1 }).await;
    assert_eq!(world.said(RESUMED), 0);
    assert_eq!((world.root_asked().len(), world.child_asked().len()), (2, 1));
    let blocks = transcript(&world.backend, &world.master, FIRST).await.blocks;
    let Block::Error(record) = World::last_block(&blocks) else {
        panic!("the turn ends with its record: {blocks:?}")
    };
    assert_eq!(record.code.as_deref(), Some("interrupted"));

    // The page opens the tree, which brings the child back; the user's
    // Resume continues the root.
    world.scripts.child_again(vec![hang()]);
    world.scripts.root(FIRST, vec![say("picked up again")]);
    let mut socket = Socket::connect(&world.backend, &world.master, FIRST).await;
    socket.open().await;
    socket.send(&ClientFrame::Resume {}).await;
    socket.until_idle().await;
    let asked = world.root_asked();
    assert_eq!(asked.len(), 3);
    assert!(!asked[2].contains(REASON), "{}", asked[2]);
    world.backend.close().await;
}

/// The databases as a crash at this moment would leave them: a copy of
/// each, taken while the backend runs.
fn snapshot(databases: &[PathBuf], into: &Path) -> Vec<(PathBuf, PathBuf)> {
    databases
        .iter()
        .enumerate()
        .map(|(index, database)| {
            let copy = into.join(format!("{index}.sqlite"));
            let connection = rusqlite::Connection::open(database).unwrap();
            connection
                .busy_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            connection
                .execute("VACUUM INTO ?1", [copy.to_str().unwrap()])
                .unwrap();
            (copy, database.clone())
        })
        .collect()
}

/// A crash in the middle of the turn: the next start resumes nothing, and
/// opening the conversation shows the interruption and offers Resume.
// A few seconds: one restart with a real device.
#[tokio::test(flavor = "local")]
async fn a_turn_a_crash_interrupted_waits_for_the_user() {
    let mut world = World::start().await;
    world.run_a_root_and_its_child().await;
    let copies = tempfile::tempdir().unwrap();
    let data = world.harness.data_dir();
    let crashed = snapshot(
        &[
            data.join("control.sqlite"),
            data.join("conversations").join(format!("{FIRST}.sqlite")),
        ],
        copies.path(),
    );
    let address = world.backend.address();
    world.backend.close().await;
    for (copy, database) in crashed {
        for suffix in ["-wal", "-shm"] {
            let mut side = database.clone().into_os_string();
            side.push(suffix);
            // A side file the close left behind would belong to the
            // database the copy replaces.
            let _ = std::fs::remove_file(side);
        }
        std::fs::copy(copy, database).unwrap();
    }

    world.backend = world.harness.start_at(address).await;
    world.master = world
        .backend
        .login(crate::support::MASTER_EMAIL, crate::support::MASTER_PASSWORD)
        .await;
    world.scripts.child_again(vec![hang()]);
    let mut socket = Socket::connect(&world.backend, &world.master, FIRST).await;
    socket.open().await;
    eventually("the child came back with the tree", || async {
        world.child_asked().len() == 2
    })
    .await;
    assert_eq!(world.root_asked().len(), 2);
    assert_eq!(world.said(RESUMED), 0);
    let blocks = transcript(&world.backend, &world.master, FIRST).await.blocks;
    let Block::Error(record) = World::last_block(&blocks) else {
        panic!("the turn ends with its record: {blocks:?}")
    };
    assert_eq!(record.code.as_deref(), Some("interrupted"));
    world.backend.close().await;
}
