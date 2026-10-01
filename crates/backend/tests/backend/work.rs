//! Work through a conversation on paired devices (`sessions-and-targets.md`,
//! `commands.md`, `runner.md`, `edit-tracking.md`): the model's shell tool
//! runs where the conversation works, and the files it makes stay with the
//! target they were made on. A switch moves the work and leaves the
//! departed device reachable through `demi host shell`, whose pipes carry
//! bytes both ways. Two conversations on one device keep apart, a
//! command's edits outlive its runner, and neither a runner lost in the
//! middle of a command nor a backend restart leaves work dangling. The
//! model is an Anthropic endpoint the test scripts; the devices are real
//! runners.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use demi_agent_tools::testing::{field, shown_output};
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::{Block, EditedFile, ToolView};
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{
    Socket, answer, anthropic_at, choose, create, kinds, tool_use, transcript,
};
use crate::support::{Harness, Paired, Session, TestBackend};

/// The conversation ids the scenarios create.
const FIRST: &str = "1e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01";
const SECOND: &str = "2e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a02";

/// What opens each context block the model reads once the conversation's
/// execution context changed.
const CONTEXT: &str = "[Execution context ";

/// The line that opens a context block's announcement of a target switch.
const SWITCHED: &str = "[Execution target switched]";

/// The model's shell call `id` running `script`, watched for at most
/// `timeout_ms`.
pub(crate) fn shell(id: &str, script: &str, timeout_ms: u64) -> MockResponse {
    tool_use(
        id,
        "shell_exec",
        &json!({ "description": id, "script": script, "timeoutMs": timeout_ms }),
    )
}

/// The model's closing words.
pub(crate) fn say(text: &str) -> MockResponse {
    answer(&[text], 1, 1)
}

/// A directory in the device's home, made if missing.
fn directory(device: &Paired, name: &str) -> PathBuf {
    let path = device.runner.home_dir().join(name);
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// Moves the conversation to `path` on `device`, as the page's target
/// switch does. A switch refuses running work, so it follows a turn whose
/// end the socket has seen.
pub(crate) async fn switch(
    backend: &TestBackend,
    master: &Session,
    id: &str,
    device: &Paired,
    path: &Path,
) {
    let target = json!({ "target": { "kind": "device", "deviceId": device.id(), "path": path.to_str().unwrap() } });
    let moved = backend
        .patch(&format!("/api/conversations/{id}"), master, target)
        .await;
    assert_eq!(
        moved.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&moved.body)
    );
}

/// A conversation the test drives: its socket, and the path of the scripted
/// endpoint its model answers from.
pub(crate) struct Driven<'a> {
    vendor: &'a MockVendor,
    route: String,
    pub(crate) socket: Socket,
    /// The tool calls whose results were read already.
    seen: HashSet<String>,
    sent: usize,
}

impl<'a> Driven<'a> {
    /// Opens the conversation `id` with the model of `provider`, whose
    /// endpoint is under `prefix`.
    pub(crate) async fn open(
        backend: &TestBackend,
        master: &Session,
        vendor: &'a MockVendor,
        id: &str,
        provider: &str,
        prefix: &str,
    ) -> Self {
        Self {
            vendor,
            route: format!("{prefix}/v1/messages"),
            socket: Self::connect(backend, master, id, provider).await,
            seen: HashSet::new(),
            sent: 0,
        }
    }

    async fn connect(backend: &TestBackend, master: &Session, id: &str, provider: &str) -> Socket {
        choose(backend, master, id, provider, "claude-opus-4-8").await;
        let mut socket = Socket::connect(backend, master, id).await;
        socket.open().await;
        socket
    }

    /// Opens the conversation again, as a reload after a restart does; the
    /// tool results read already stay read.
    pub(crate) async fn reconnect(
        &mut self,
        backend: &TestBackend,
        master: &Session,
        id: &str,
        provider: &str,
    ) {
        self.socket = Self::connect(backend, master, id, provider).await;
    }

    /// Scripts the model's answers and sends a message, without waiting for
    /// its turn; answers how many requests the vendor had before.
    pub(crate) async fn start(&mut self, answers: Vec<MockResponse>) -> usize {
        let before = self.vendor.requests().len();
        for response in answers {
            self.vendor.respond_at(&self.route, response);
        }
        self.sent += 1;
        let message = format!("m{}", self.sent);
        self.socket
            .send(&crate::conversations::send(&message, "go"))
            .await;
        before
    }

    /// Scripts the model's answers, sends a message and waits for its turn
    /// to end.
    pub(crate) async fn turn(&mut self, answers: Vec<MockResponse>) -> Turn {
        let before = self.start(answers).await;
        self.socket.until_idle().await;
        self.observe(before)
    }

    /// What the requests since the `before`th showed the model.
    pub(crate) fn observe(&mut self, before: usize) -> Turn {
        let requests: Vec<Value> = self.vendor.requests()[before..]
            .iter()
            .filter(|request| request.uri.path() == self.route)
            .map(|request| request.json())
            .collect();
        let mut received = Vec::new();
        for request in &requests {
            for message in request["messages"].as_array().unwrap() {
                for block in message["content"].as_array().into_iter().flatten() {
                    if block["type"] != "tool_result" {
                        continue;
                    }
                    let id = block["tool_use_id"].as_str().unwrap().to_owned();
                    if !self.seen.insert(id) {
                        continue;
                    }
                    let text: Vec<&str> = block["content"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|part| part["text"].as_str().unwrap_or("[image]"))
                        .collect();
                    received.push(text.join("\n"));
                }
            }
        }
        Turn { received, requests }
    }
}

/// Waits until something is at `path`.
async fn until_exists(path: &Path) {
    crate::support::eventually(&format!("{} exists", path.display()), || {
        let exists = path.exists();
        async move { exists }
    })
    .await;
}

/// A turn as the model saw it.
pub(crate) struct Turn {
    /// Each tool result the model received, in order.
    pub(crate) received: Vec<String>,
    /// The requests the turn made.
    pub(crate) requests: Vec<Value>,
}

impl Turn {
    /// The text of the turn's first request, context blocks included.
    pub(crate) fn first_request(&self) -> String {
        self.requests[0]["messages"].to_string()
    }
}

// Several seconds: a real device installs the builtin package, and four turns
// run a shell job each.
#[tokio::test]
async fn the_model_creates_reads_edits_and_lists_its_files_where_the_conversation_works() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let home = alpha.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &alpha, &home).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;

    let heredoc =
        "mkdir src && cd src && demi file create notes.md <<'EOF'\nalpha\nbeta\ngamma\nEOF";
    let created = work
        .turn(vec![shell("t1", heredoc, 10_000), say("created")])
        .await;
    assert!(
        created.received[0].contains("exitCode: 0"),
        "{}",
        created.received[0]
    );
    assert!(
        created.received[0].contains("Created notes.md"),
        "{}",
        created.received[0]
    );
    assert_eq!(
        std::fs::read_to_string(home.join("src/notes.md")).unwrap(),
        "alpha\nbeta\ngamma\n"
    );
    // The model is offered the `demi.file` package's commands beside the
    // backend's own.
    let system = created.requests[0]["system"].to_string();
    assert!(
        system.contains("demi file create") && system.contains("demi host"),
        "{system}"
    );

    // The shell keeps its directory between turns.
    let script = "pwd && demi file read notes.md | grep -n a | sort -r";
    let read = work
        .turn(vec![shell("t2", script, 10_000), say("read")])
        .await;
    assert!(
        read.received[0].contains("/src\n3:gamma\n2:beta\n1:alpha"),
        "{}",
        read.received[0]
    );

    let script = "demi file edit notes.md --old beta --new delta && cat notes.md";
    let edited = work
        .turn(vec![shell("t3", script, 10_000), say("edited")])
        .await;
    assert!(
        edited.received[0].contains("Edited notes.md\nalpha\ndelta\ngamma"),
        "{}",
        edited.received[0]
    );
    assert_eq!(
        std::fs::read_to_string(home.join("src/notes.md")).unwrap(),
        "alpha\ndelta\ngamma\n"
    );

    let listed = work
        .turn(vec![
            shell("t4", "ls && demi host current", 10_000),
            say("listed"),
        ])
        .await;
    assert!(
        listed.received[0].contains("notes.md"),
        "{}",
        listed.received[0]
    );
    assert!(
        listed.received[0].contains("host: machine \"alpha\""),
        "{}",
        listed.received[0]
    );
    backend.close().await;
}

// Several seconds: the switch goes between two real devices, which each install
// the builtin package, and five shell jobs run on them.
#[tokio::test]
async fn a_switch_moves_the_work_and_the_departed_device_keeps_its_files_within_reach() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let beta = backend.pair(&master, "beta").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, FIRST).await;
    let on_alpha = directory(&alpha, "project");
    let on_beta = directory(&beta, "project");
    switch(&backend, &master, FIRST, &alpha, &on_alpha).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/work").await;
    let notes = "demi file create notes.md <<'EOF'\nalpha\nbeta\ngamma\nEOF";
    let created = work
        .turn(vec![shell("t1", notes, 10_000), say("created")])
        .await;
    assert!(
        created.received[0].contains("Created notes.md"),
        "{}",
        created.received[0]
    );

    // On the other device, the next turn opens with the switch; the file
    // stayed where it was made.
    switch(&backend, &master, FIRST, &beta, &on_beta).await;
    let moved = work
        .turn(vec![
            shell("t2", "cat notes.md; echo exit=$?", 10_000),
            shell("t3", notes, 10_000),
            say("recreated"),
        ])
        .await;
    let context = moved.first_request();
    let told = context.matches(CONTEXT).count();
    let switches = context.matches(SWITCHED).count();
    assert!(context.contains("[Execution target switched]"), "{context}");
    assert!(
        context.contains("Previous target: the machine \\\"alpha\\\""),
        "{context}"
    );
    assert!(
        context.contains("stays attached as \\\"alpha\\\""),
        "{context}"
    );
    assert!(
        moved.received[0].contains("No such file or directory"),
        "{}",
        moved.received[0]
    );
    assert!(
        moved.received[0].contains("exit=1"),
        "{}",
        moved.received[0]
    );
    assert!(
        moved.received[1].contains("Created notes.md"),
        "{}",
        moved.received[1]
    );
    let script = "demi file edit notes.md --old beta --new delta && cat notes.md";
    let edited = work
        .turn(vec![shell("t4", script, 10_000), say("edited")])
        .await;
    assert!(
        edited.received[0].contains("alpha\ndelta\ngamma"),
        "{}",
        edited.received[0]
    );
    assert_eq!(
        std::fs::read_to_string(on_beta.join("notes.md")).unwrap(),
        "alpha\ndelta\ngamma\n"
    );
    assert_eq!(
        std::fs::read_to_string(on_alpha.join("notes.md")).unwrap(),
        "alpha\nbeta\ngamma\n"
    );
    // The model was told of the switch once: the history holds it, and the
    // next turn is told nothing new.
    assert_eq!(
        edited.first_request().matches(CONTEXT).count(),
        told,
        "{}",
        edited.first_request()
    );

    // A change of the attached hosts alone is news of its own.
    let path = format!("/api/conversations/{FIRST}/hosts/{}", alpha.id());
    let renamed = backend
        .patch(&path, &master, json!({ "name": "first" }))
        .await;
    assert_eq!(
        renamed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&renamed.body)
    );
    let noted = work.turn(vec![say("noted")]).await.first_request();
    assert!(noted.contains("[Attached hosts changed]"), "{noted}");
    assert!(
        noted.contains("\\\"first\\\" (online, shells start in"),
        "{noted}"
    );
    assert_eq!(noted.matches(SWITCHED).count(), switches, "{noted}");

    // Back on the first device: the one left is attached under its name,
    // and `demi host shell --host` reaches it.
    switch(&backend, &master, FIRST, &alpha, &on_alpha).await;
    let script = "cat notes.md && demi host shell --host beta \"cat notes.md\"";
    let back = work
        .turn(vec![shell("t5", script, 20_000), say("back")])
        .await;
    assert!(back.first_request().contains("[Execution target switched]"));
    assert!(
        back.received[0].contains("alpha\nbeta\ngamma\nalpha\ndelta\ngamma"),
        "{}",
        back.received[0]
    );
    backend.close().await;
}

// Several seconds: a real device installs the builtin package, and two
// conversations run two turns each on it at once.
#[tokio::test]
async fn two_conversations_on_one_device_keep_their_directories_shells_and_todos_apart() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let home = alpha.runner.home_dir().to_owned();
    let first_model = anthropic_at(&backend, &master, &vendor, "/a").await;
    let second_model = anthropic_at(&backend, &master, &vendor, "/b").await;
    for id in [FIRST, SECOND] {
        create(&backend, &master, id).await;
        switch(&backend, &master, id, &alpha, &home).await;
    }
    let mut a = Driven::open(&backend, &master, &vendor, FIRST, &first_model, "/a").await;
    let mut b = Driven::open(&backend, &master, &vendor, SECOND, &second_model, "/b").await;

    // A moves into a directory, sets a variable and writes todos while B,
    // at the same time, does none of it.
    let moving = "mkdir -p sub && cd sub && MARK=from-a && echo \"a: $(pwd) $MARK\" && demi todo add \"draft the outline\" && demi todo add \"run the suite\"";
    let staying = "echo \"b: $(pwd) mark=${MARK:-unset}\"";
    let (first, second) = tokio::join!(
        a.turn(vec![shell("a1", moving, 10_000), say("a moved")]),
        b.turn(vec![shell("b1", staying, 10_000), say("b stayed")]),
    );
    let sub = home.join("sub");
    assert!(
        first.received[0].contains(&format!("a: {} from-a", sub.display())),
        "{}",
        first.received[0]
    );
    assert!(
        second.received[0].contains(&format!("b: {} mark=unset", home.display())),
        "{}",
        second.received[0]
    );

    // A's shell carries its directory to the next turn and nothing else;
    // B's never moved. The todos last across turns, and stay with the
    // conversation that wrote them.
    let todos = "echo \"a: $(pwd) mark=${MARK:-unset}\" && demi todo list --json";
    let (third, fourth) = tokio::join!(
        a.turn(vec![shell("a2", todos, 10_000), say("a again")]),
        b.turn(vec![
            shell("b2", &format!("{staying} && demi todo list --json"), 10_000),
            say("b again")
        ]),
    );
    assert!(
        third.received[0].contains(&format!("a: {} mark=unset", sub.display())),
        "{}",
        third.received[0]
    );
    let listed = &third.received[0];
    assert!(
        listed.contains("draft the outline") && listed.contains("run the suite"),
        "{listed}"
    );
    assert!(
        fourth.received[0].contains(&format!("b: {} mark=unset", home.display())),
        "{}",
        fourth.received[0]
    );
    assert!(
        fourth.received[0].contains("{\"todos\":[]}"),
        "{}",
        fourth.received[0]
    );
    backend.close().await;
}

// Several seconds: a real device installs the builtin package, loses its runner
// in the middle of a job, and starts it again.
#[tokio::test]
async fn a_runner_lost_in_the_middle_of_a_command_ends_it_and_the_returned_runner_serves_the_next_turn()
 {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let mut alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let home = alpha.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &alpha, &home).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let written = work
        .turn(vec![
            shell("t1", "echo -n before > before.txt", 10_000),
            say("written"),
        ])
        .await;
    assert!(
        written.received[0].contains("exitCode: 0"),
        "{}",
        written.received[0]
    );

    // The command is running on the device when its runner goes.
    let started = home.join("started");
    let before = work
        .start(vec![
            shell("t2", "touch started; sleep 20; echo late", 30_000),
            say("the runner is gone"),
        ])
        .await;
    until_exists(&started).await;
    alpha.runner.kill().await;
    work.socket.until_idle().await;
    let lost = work.observe(before);
    assert!(
        lost.received[0].contains("exitCode: 127"),
        "{}",
        lost.received[0]
    );
    assert!(
        lost.received[0].contains("runner disconnected"),
        "{}",
        lost.received[0]
    );

    alpha.runner.start_again();
    backend.until_online(&master, alpha.id(), true).await;
    let back = work
        .turn(vec![shell("t3", "cat before.txt", 10_000), say("back")])
        .await;
    assert!(back.received[0].contains("before"), "{}", back.received[0]);
    backend.close().await;
}

/// Each shell call's command and the files it kept, in the transcript's
/// order.
async fn kept_files(
    backend: &TestBackend,
    master: &Session,
    id: &str,
) -> Vec<(String, Vec<EditedFile>)> {
    transcript(backend, master, id)
        .await
        .blocks
        .into_iter()
        .filter_map(|block| match block {
            Block::ToolCall(call) => match call.view {
                Some(ToolView::Shell(view)) => {
                    Some((view.command_id.to_string(), view.files.unwrap_or_default()))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The two sides of `file`'s first edit segment, read from the blob route
/// as the change view reads them.
pub(crate) async fn edit_sides(
    backend: &TestBackend,
    master: &Session,
    file: &EditedFile,
) -> (String, String) {
    let copies = file.edits[0]
        .copies
        .as_ref()
        .unwrap_or_else(|| panic!("the edit has no copies: {file:?}"));
    let mut sides = Vec::new();
    for blob in [&copies.original, &copies.modified] {
        let answer = backend
            .get(&format!("/api/blobs/{blob}"), Some(master))
            .await;
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&answer.body)
        );
        sides.push(String::from_utf8(answer.body).unwrap());
    }
    let modified = sides.pop().unwrap();
    (sides.pop().unwrap(), modified)
}

// Several seconds: a real device installs the builtin package and runs the
// commands whose edits are kept, and its runner stops.
#[tokio::test]
async fn a_commands_edits_are_kept_as_its_call_history_and_outlive_its_runner() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let mut paired = backend.pair(&master, "paired").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/paired").await;
    create(&backend, &master, FIRST).await;
    let home = paired.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &paired, &home).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/paired").await;
    let create_files = "set -e\nprintf 'before\\n' > note.txt\nprintf 'created\\n' | demi file create native.txt\nprintf '\\0binary' > asset.bin\nprintf 'temporary' > removed.txt\nrm removed.txt";
    work.turn(vec![
        shell("create", create_files, 10_000),
        say("Created the files."),
    ])
    .await;
    let first = kept_files(&backend, &master, FIRST).await.remove(0);
    let names: Vec<&str> = first
        .1
        .iter()
        .map(|file| file.path.rsplit('/').next().unwrap())
        .collect();
    assert_eq!(names, ["note.txt", "native.txt", "asset.bin"]);
    // A binary file's edit is listed without copies.
    assert_eq!(first.1[2].edits[0].copies, None);
    let note = edit_sides(&backend, &master, &first.1[0]).await;
    assert_eq!(note, (String::new(), "before\n".to_owned()));

    let patch = "demi file patch <<'PATCH'\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-before\n+after\n--- a/native.txt\n+++ b/native.txt\n@@ -1 +1 @@\n-created\n+patched\nPATCH";
    work.turn(vec![
        shell("patch", patch, 10_000),
        say("Patched both files."),
    ])
    .await;
    let calls = kept_files(&backend, &master, FIRST).await;
    let second = calls.last().unwrap().clone();
    let kinds: Vec<String> = second
        .1
        .iter()
        .map(|file| {
            serde_json::to_value(file.kind)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(kinds, ["modified", "modified"]);
    let patched = edit_sides(&backend, &master, &second.1[0]).await;
    assert_eq!(patched, ("before\n".to_owned(), "after\n".to_owned()));

    // Neither the archive nor the runner's absence takes them away.
    let archived = backend
        .patch(
            &format!("/api/conversations/{FIRST}"),
            &master,
            json!({ "archived": true }),
        )
        .await;
    assert_eq!(
        archived.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&archived.body)
    );
    paired.runner.kill().await;
    let native = edit_sides(&backend, &master, &second.1[1]).await;
    assert_eq!(native, ("created\n".to_owned(), "patched\n".to_owned()));
    backend.close().await;
}

// Several seconds: a real device installs the builtin package, and its runner
// comes back after the backend's restart to run the next job.
#[tokio::test]
async fn after_a_backend_restart_the_runner_comes_back_and_the_conversation_goes_on_there() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let home = alpha.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &alpha, &home).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let kept = work
        .turn(vec![
            shell("t1", "echo -n kept > kept.txt && cat kept.txt", 10_000),
            say("remember me"),
        ])
        .await;
    assert!(kept.received[0].contains("kept"), "{}", kept.received[0]);
    // The conversation's first command is command 1.
    assert_eq!(field(&kept.received[0], "commandId"), "1");

    // The backend stops under a running command: the call ends as an error
    // and the turn as interrupted, with nothing left dangling.
    let started = home.join("started");
    work.start(vec![shell("t2", "touch started; sleep 20", 30_000)])
        .await;
    until_exists(&started).await;
    // The backend comes back at its address, where the runner reconnects.
    let address = backend.address();
    backend.close().await;
    let backend = harness.start_at(address).await;
    backend.until_online(&master, alpha.id(), true).await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let kinds = kinds(&blocks);
    let turn = kinds.iter().rposition(|kind| kind == "user").unwrap();
    assert_eq!(kinds[turn..], ["user", "tool_call", "response", "error"]);
    let Some(Block::ToolCall(cut)) = blocks.get(turn + 1) else {
        unreachable!()
    };
    assert_eq!(cut.status.to_string(), "error", "{cut:?}");
    let Some(Block::Error(record)) = blocks.last() else {
        unreachable!()
    };
    assert_eq!(record.code.as_deref(), Some("interrupted"));

    // The runner came back on its own, and the next turn runs there.
    work.reconnect(&backend, &master, FIRST, &provider).await;
    let script = "cat kept.txt && demi host current";
    let after = work
        .turn(vec![shell("t3", script, 10_000), say("and again")])
        .await;
    assert!(
        after.received[0].contains("Tool call aborted"),
        "{}",
        after.received[0]
    );
    assert!(after.received[1].contains("kept"), "{}", after.received[1]);
    assert!(
        after.received[1].contains("host: machine \"alpha\""),
        "{}",
        after.received[1]
    );
    // The cut command was 2; the restarted backend gives no number twice.
    assert_eq!(field(&after.received[1], "commandId"), "3");
    backend.close().await;
}

/// A conversation that worked on `alpha` and then moved to `beta`, which
/// left `alpha` attached.
async fn moved_from_alpha_to_beta(
    backend: &TestBackend,
    master: &Session,
    alpha: &Paired,
    beta: &Paired,
) -> (PathBuf, PathBuf) {
    create(backend, master, FIRST).await;
    let on_alpha = alpha.runner.home_dir().to_owned();
    let on_beta = beta.runner.home_dir().to_owned();
    switch(backend, master, FIRST, alpha, &on_alpha).await;
    switch(backend, master, FIRST, beta, &on_beta).await;
    (on_alpha, on_beta)
}

// Several seconds: two real devices each install the builtin package, and `demi
// host shell` runs jobs on both.
#[tokio::test]
async fn demi_host_shell_carries_bytes_both_ways_through_pipes_and_keeps_the_far_hosts_directory() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let beta = backend.pair(&master, "beta").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    let (a, b) = moved_from_alpha_to_beta(&backend, &master, &alpha, &beta).await;
    // Well past the view a job's output travels in: only a pipe carries it
    // whole.
    let payload: Vec<u8> = (0..300 * 1024)
        .map(|index: usize| (index * 31 % 256) as u8)
        .collect();
    std::fs::write(a.join("notes.bin"), &payload).unwrap();
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/work").await;
    let a_path = a.display().to_string();
    let a_id = alpha.id().to_owned();

    let pull = format!(
        "demi host list && demi host shell --host alpha \"tar c -C {a_path} notes.bin\" | tar x && cmp notes.bin {a_path}/notes.bin && echo copied"
    );
    let pulled = work
        .turn(vec![shell("t1", &pull, 30_000), say("one")])
        .await;
    let result = &pulled.received[0];
    assert!(
        result.contains(&format!(
            "beta  {}  online  {}  (main)",
            beta.id(),
            b.display()
        )),
        "{result}"
    );
    assert!(
        result.contains(&format!("alpha  {a_id}  online  {a_path}  (attached)")),
        "{result}"
    );
    assert!(result.contains("copied"), "{result}");
    assert!(std::fs::read(b.join("notes.bin")).unwrap() == payload);

    // The other way: the caller's pipe is the far job's standard input.
    let push = format!(
        "head -c 250000 notes.bin > push.bin && tar c push.bin | demi host shell --host alpha \"tar x -C {a_path}\" && cmp push.bin {a_path}/push.bin && echo pushed"
    );
    let pushed = work
        .turn(vec![shell("t2", &push, 30_000), say("two")])
        .await;
    assert!(
        pushed.received[0].contains("pushed"),
        "{}",
        pushed.received[0]
    );
    assert!(std::fs::read(a.join("push.bin")).unwrap() == payload[..250_000]);

    // Where a shell on the attached host ends is where the next one starts,
    // and `--host` takes the device's id as well.
    let wander = format!(
        "demi host shell --host alpha \"mkdir -p sub && cd sub && pwd\" && demi host shell --host {a_id} \"pwd\" && demi host list"
    );
    let wandered = work
        .turn(vec![shell("t3", &wander, 30_000), say("three")])
        .await;
    let sub = format!("{a_path}/sub");
    assert!(
        wandered.received[0].contains(&format!("{sub}\n{sub}\n")),
        "{}",
        wandered.received[0]
    );
    assert!(
        wandered.received[0].contains(&format!("alpha  {a_id}  online  {sub}  (attached)")),
        "{}",
        wandered.received[0]
    );

    let stranger = work
        .turn(vec![
            shell(
                "t4",
                "demi host shell --host nope \"echo hi\"; echo exit=$?",
                10_000,
            ),
            say("four"),
        ])
        .await;
    assert!(
        stranger.received[0].contains("host nope is not reachable"),
        "{}",
        stranger.received[0]
    );
    assert!(
        stranger.received[0].contains("exit=1"),
        "{}",
        stranger.received[0]
    );
    backend.close().await;
}

// Several seconds: two real devices each install the builtin package, and the
// far job runs through `demi host shell`.
#[tokio::test]
async fn demi_host_shell_shows_the_far_jobs_errors_as_they_come_takes_its_input_and_is_stopped_with_it()
 {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let beta = backend.pair(&master, "beta").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    let (a, _) = moved_from_alpha_to_beta(&backend, &master, &alpha, &beta).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/work").await;

    // The far job reports on standard error at once and waits for a line,
    // then runs a process of its own; the model's window ends while it
    // runs.
    let far = "printf \"ready\\n\" >&2; read line; printf \"%s\" \"$line\" > got.txt; sh -c \"echo \\$\\$ > far.pid; exec /bin/sleep 30\"";
    let script = format!("demi host shell --host alpha '{far}'");
    let started = work
        .turn(vec![shell("t1", &script, 500), say("waiting")])
        .await;
    let result = &started.received[0];
    assert!(result.starts_with("status: running"), "{result}");
    let command = field(result, "commandId").to_owned();
    // The far job's error output reaches the model while the job runs: the
    // model reads the command's output, each read showing what came since
    // the one before, until `ready` is there. The far job's start (a login
    // shell on alpha) may outlast the window above, and its shell's printf
    // writes a byte at a time, so `ready` may come split between reads.
    let mut output = shown_output(result);
    let deadline = tokio::time::Instant::now() + crate::support::PATIENCE;
    let mut reads = 0;
    while !output.contains("ready") {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the far job's ready never came: {output:?}"
        );
        // Each read is a turn of two requests, and the backend starts at most
        // 120 a minute (`usage-and-quota.md` § Rate limit): 400 ms apart, the
        // reads of the whole wait make at most 100.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        reads += 1;
        let status = json!({ "commandId": command });
        let read = work
            .turn(vec![
                tool_use(&format!("s{reads}"), "shell_status", &status),
                say("still waiting"),
            ])
            .await;
        let result = read.received.first().expect("the read reaches the model");
        assert!(result.starts_with("status: running"), "{result}");
        output.push_str(&shown_output(result));
    }

    let write = json!({ "commandId": command, "stdin": "hello\n" });
    work.turn(vec![tool_use("t2", "shell_write", &write), say("fed")])
        .await;
    let got = a.join("got.txt");
    let pid = a.join("far.pid");
    crate::support::eventually("the far job read the line and went on", || {
        let read = std::fs::read_to_string(&got).unwrap_or_default() == "hello";
        let started = std::fs::read_to_string(&pid).is_ok_and(|pid| pid.ends_with('\n'));
        async move { read && started }
    })
    .await;

    // Stopping the command stops the far job with it.
    let pid = std::fs::read_to_string(&pid).unwrap().trim().to_owned();
    let alive = |pid: &str| {
        std::process::Command::new("kill")
            .args(["-0", pid])
            .status()
            .unwrap()
            .success()
    };
    assert!(alive(&pid));
    let stopped = work
        .turn(vec![
            tool_use("t3", "shell_abort", &json!({ "commandId": command })),
            say("stopped"),
        ])
        .await;
    assert!(
        stopped.received[0].starts_with("status: aborted"),
        "{}",
        stopped.received[0]
    );
    crate::support::eventually("the far job ended", || {
        let alive = alive(&pid);
        async move { !alive }
    })
    .await;
    backend.close().await;
}
