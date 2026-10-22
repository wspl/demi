//! A project's workflows on a paired device
//! (`crates/backend-user-shard/src/conversation/workflows.rs`).
//!
//! Scenarios:
//!
//! - Each directory's `.demi/instructions.md` joins the instructions block
//!   after its `AGENTS.md`.
//! - The Workflow menu's list reads `.demi/workflows/` as the backend keeps it,
//!   at once, unknown before its first read and kept across a restart while
//!   the Host is gone.
//! - A conversation's choice puts the workflow's instructions in the root's
//!   block and its profiles in place of the user's, a model no entry offers
//!   making a profile unavailable. No Subagents takes both away.
//! - A workflow added or edited raises the workflows revision the
//!   conversation's metadata carries, and the page reads the list again.
//! - An agent picks the workflow with `demi agent workflows use` once the user
//!   allows it in the conversation, as the menu does; a workflow that cannot
//!   be used is refused and the choice stays.

use demi_provider_common::testing::MockVendor;
use demi_shared_types::{Block, INSTRUCTIONS_SOURCE, InstructionEntry, WorkflowChoice};
use demi_web_api_protocol::conversations::ConversationUpdate;
use demi_web_api_protocol::workflows::{ConversationWorkflowList, KeptWorkflowList};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::auth::Role;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, Socket, anthropic_at, create, transcript};
use crate::files::watch_message;
use crate::support::{Harness, Session, TestBackend};
use crate::support::eventually;
use crate::work::{Driven, say, shell, switch};

const REVIEW: &str = "name: Review Team
instructions: |
  REVIEW STEPS
profiles:
  - id: explore
    description: Finds code.
    model: claude-opus-4-8
    context: 300K
  - id: slow
    description: Thinks long.
    model: nowhere-model
";

/// Makes `choice` the conversation's Workflow menu choice, as the menu does.
async fn choose(backend: &TestBackend, master: &Session, choice: serde_json::Value) -> WorkflowChoice {
    let chosen = backend
        .patch(&format!("/api/conversations/{FIRST}"), master, json!({ "workflow": choice }))
        .await;
    assert_eq!(chosen.status, StatusCode::OK, "{}", String::from_utf8_lossy(&chosen.body));
    chosen.json::<ConversationUpdate>().metadata.workflow
}

// A few seconds: a real runner on a paired device answers the reads, and
// three turns run a shell job each.
#[tokio::test]
async fn demis_own_files_and_the_chosen_workflow_reach_the_root_and_its_spawns() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    harness
        .control_database()
        .execute(
            "INSERT INTO permission_grants (conversation_id, category, granted_at) VALUES (?1, 'agent.profiles', 1)",
            [FIRST],
        )
        .unwrap();

    let repo = alpha.runner.home_dir().join("repo");
    let web = repo.join("web");
    let workflows = repo.join(".demi/workflows");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(web.join(".demi")).unwrap();
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(repo.join("AGENTS.md"), "ROOT RULES").unwrap();
    std::fs::write(repo.join(".demi/instructions.md"), "DEMI ROOT").unwrap();
    std::fs::write(web.join(".demi/instructions.md"), "DEMI WEB").unwrap();
    std::fs::write(workflows.join("review.yaml"), REVIEW).unwrap();
    std::fs::write(
        workflows.join("broken.yaml"),
        "name: Broken\nprofiles:\n  - id: Reader\n    description: Reads.\n",
    )
    .unwrap();
    switch(&backend, &master, FIRST, &alpha, &web).await;
    let path = |path: &std::path::Path| path.to_str().unwrap().to_owned();
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;

    // The menu's list, as the backend read it.
    let mut page = MenuPage::open(&backend, &master).await;
    let listed = page.when(&backend, &master, |_| true).await;
    let broken = format!(
        "{} does not read: line 3 column 9: validation error: is not 1 to 40 lowercase letters, digits and hyphens starting with a letter for `profiles[0].id`",
        path(&workflows.join("broken.yaml"))
    );
    assert_eq!(
        listed,
        json!({
            "directory": path(&workflows),
            "workflows": [
                { "id": "broken", "name": null, "path": path(&workflows.join("broken.yaml")), "unavailable": broken, "profiles": [] },
                { "id": "review", "name": "Review Team", "path": path(&workflows.join("review.yaml")), "unavailable": null, "profiles": ["explore", "slow"] },
            ],
            "unavailable": null,
        })
    );

    // Default: Demi's own files follow each directory's AGENTS.md, and no
    // workflow is in the block.
    let first = work.turn(vec![say("hello")]).await.first_request();
    let (root, demi_root, demi_web) = (
        first.find("ROOT RULES").expect("the root's AGENTS.md"),
        first.find("DEMI ROOT").expect("the root's .demi/instructions.md"),
        first.find("DEMI WEB").expect("web's .demi/instructions.md"),
    );
    assert!(root < demi_root && demi_root < demi_web, "{first}");
    assert!(!first.contains("<workflow"), "{first}");
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let entries: Vec<InstructionEntry> = blocks
        .iter()
        .filter_map(|block| match block {
            Block::Context(context) if context.source == INSTRUCTIONS_SOURCE => Some(context.instructions.clone()),
            _ => None,
        })
        .next_back()
        .unwrap();
    assert_eq!(
        entries,
        [repo.join("AGENTS.md"), repo.join(".demi/instructions.md"), web.join(".demi/instructions.md")]
            .iter()
            .map(|file| InstructionEntry::File { path: path(file) })
            .collect::<Vec<_>>()
    );

    // Review Team: its instructions join the root's block, and its profiles
    // are the conversation's; `slow`'s model is in no entry.
    assert_eq!(
        choose(&backend, &master, json!({ "type": "workflow", "id": "review" })).await,
        WorkflowChoice::Workflow { id: "review".into() }
    );
    let turn = work
        .turn(vec![
            shell(
                "t1",
                "demi agent profiles list; demi agent spawn --profile slow <<< 'Think'; demi agent profiles create mine --description Mine. </dev/null",
            ),
            say("listed"),
        ])
        .await;
    assert!(
        turn.first_request().contains("<workflow name=\\\"Review Team\\\">\\nREVIEW STEPS\\n</workflow>"),
        "{}",
        turn.first_request()
    );
    let review = path(&workflows.join("review.yaml"));
    // The card lists the workflow after the project's files, with its file.
    let newest: Vec<InstructionEntry> = transcript(&backend, &master, FIRST)
        .await
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Context(context) if context.source == INSTRUCTIONS_SOURCE => Some(context.instructions.clone()),
            _ => None,
        })
        .next_back()
        .unwrap();
    assert_eq!(
        newest.last(),
        Some(&InstructionEntry::Workflow { name: "Review Team".into(), path: review.clone() })
    );
    let unavailable = format!(
        "profile \"slow\" is unavailable: no provider entry of the user's offers model \"nowhere-model\"; change the profile's model in {review}, or ask the user to add a provider that offers it"
    );
    let received = &turn.received[0];
    assert!(
        received.contains("NAME     SOURCE       MODEL            EFFORT  DESCRIPTION\nexplore  Review Team  claude-opus-4-8  low     Finds code.\nslow     Review Team  -                -       Thinks long.\n"),
        "{received}"
    );
    // A profile made here is the user's, which no spawn here takes: the
    // command says so.
    assert!(received.contains("Name: mine\nSource: user\n"), "{received}");
    assert!(
        received.contains("demi agent profiles create: warning: this conversation runs workflow Review Team, so demi agent spawn here takes only its profiles; \"mine\" is for the user's other conversations"),
        "{received}"
    );
    assert!(received.contains(&format!("demi agent spawn: {unavailable}")), "{received}");

    // No Subagents: the workflow leaves the block, and spawns fail.
    assert_eq!(choose(&backend, &master, json!({ "type": "none" })).await, WorkflowChoice::None);
    let turn = work
        .turn(vec![shell("t2", "demi agent spawn <<< 'Think'"), say("refused")])
        .await;
    let request = turn.first_request();
    assert_eq!(request.matches("<workflow name=").count(), 1, "{request}");
    assert_eq!(request.matches("DEMI WEB").count(), 3, "a block without the workflow follows: {request}");
    assert!(
        turn.received[0].contains("subagents are turned off for this conversation"),
        "{}",
        turn.received[0]
    );
    backend.close().await;
}

const DOCS: &str = "name: Docs Review
instructions: |
  DOCS STEPS ONE
profiles: []
";

/// The Workflow menu's list as the backend stored it; none while it is
/// unknown or its first read was refused.
fn stored_menu(kept: &KeptWorkflowList) -> Option<serde_json::Value> {
    match kept {
        KeptWorkflowList::Unknown | KeptWorkflowList::Refused { .. } => None,
        KeptWorkflowList::Read { list, .. } => Some(serde_json::to_value(list).unwrap()),
    }
}

/// A page that shows the conversation `FIRST` and its Workflow menu, as
/// each metadata its socket brings carries the list.
struct MenuPage {
    socket: Socket,
    /// The latest list the page was sent.
    shown: ConversationWorkflowList,
}

impl MenuPage {
    async fn open(backend: &TestBackend, master: &Session) -> Self {
        let (socket, metadata) = Socket::shown(backend, master, FIRST, 0).await;
        Self {
            socket,
            shown: metadata.workflows,
        }
    }

    /// The list once it holds what `holds` asks, as the page was sent it at
    /// the open and at each change; a change made on the Host reaches it
    /// through the runner's file watch.
    async fn when(
        &mut self,
        _backend: &TestBackend,
        _master: &Session,
        holds: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        loop {
            if let Some(listed) = stored_menu(&self.shown.list)
                && holds(&listed)
            {
                return listed;
            }
            let seen = self.shown.clone();
            self.shown = self.socket.metadata_until(|metadata| metadata.workflows != seen).await.workflows;
        }
    }
}

/// The `changed` batches the watch sends until one names `key`: every
/// other key they name but `tree`'s.
pub(crate) async fn others_until(socket: &mut crate::files::Socket, key: &str) -> Vec<String> {
    let mut others = Vec::new();
    loop {
        let message = watch_message(socket).await;
        assert_eq!(message["type"], "changed", "{message}");
        let mut named = false;
        for change in message["changes"].as_array().unwrap() {
            match change[0].as_str().unwrap() {
                changed if changed == key => named = true,
                "tree" => {}
                changed => others.push(changed.to_owned()),
            }
        }
        if named {
            return others;
        }
    }
}

// One to four seconds: a real runner on a paired device answers the reads
// and reports the changes.
//
// Planted defect it catches: a change the backend's search finds that it
// never announces, so the page keeps the list it first read.
#[tokio::test]
async fn a_page_hears_of_a_workflow_added_or_edited_and_reads_the_list_again() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;

    // `repo/app/web` works with the workflows of `repo`; `app` has a
    // `.demi` without workflows, `web` none.
    let repo = alpha.runner.home_dir().join("repo");
    let app = repo.join("app");
    let web = app.join("web");
    let workflows = repo.join(".demi/workflows");
    std::fs::create_dir_all(app.join(".demi")).unwrap();
    std::fs::create_dir_all(&web).unwrap();
    std::fs::create_dir_all(&workflows).unwrap();
    let initialized = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(initialized.success());
    std::fs::write(workflows.join("review.yaml"), REVIEW).unwrap();
    switch(&backend, &master, FIRST, &alpha, &web).await;
    let _work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let names = |listed: &serde_json::Value| -> Vec<String> {
        listed["workflows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|workflow| workflow["name"].as_str().unwrap_or_default().to_owned())
            .collect()
    };

    let mut page = MenuPage::open(&backend, &master).await;
    let listed = page.when(&backend, &master, |_| true).await;
    assert_eq!(names(&listed), ["Review Team"]);

    // A new workflow, then the first one edited in place.
    std::fs::write(workflows.join("docs.yaml"), DOCS).unwrap();
    let listed = page.when(&backend, &master, |listed| names(listed).len() == 2).await;
    assert_eq!(names(&listed), ["Docs Review", "Review Team"]);
    std::fs::write(workflows.join("review.yaml"), REVIEW.replace("Review Team", "Review Team Two")).unwrap();
    let listed = page.when(&backend, &master, |listed| names(listed).contains(&"Review Team Two".to_owned())).await;
    assert_eq!(names(&listed), ["Docs Review", "Review Team Two"]);
    backend.close().await;
}

// A few seconds: a real runner on a paired device answers the reads, and
// two turns run a shell job each.
#[tokio::test]
async fn a_linked_workflow_is_read_check_lets_the_host_resolve_and_a_new_choice_reads_its_instructions_at_once() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let path = |path: &std::path::Path| path.to_str().unwrap().to_owned();

    // `repo/.demi/workflows` holds review.yaml, docs.yaml, and linked.yaml,
    // a link to a workflow outside the project, read as a file is.
    let home = alpha.runner.home_dir().to_owned();
    let repo = home.join("repo");
    let workflows = repo.join(".demi/workflows");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::create_dir_all(home.join("elsewhere/deep")).unwrap();
    std::fs::write(workflows.join("review.yaml"), REVIEW).unwrap();
    std::fs::write(workflows.join("docs.yaml"), DOCS).unwrap();
    std::fs::write(home.join("elsewhere/linked.yaml"), DOCS).unwrap();
    std::os::unix::fs::symlink(home.join("elsewhere/linked.yaml"), workflows.join("linked.yaml")).unwrap();
    // `repo/deep` is a link whose `..` is `elsewhere`, not `repo`.
    std::os::unix::fs::symlink(home.join("elsewhere/deep"), repo.join("deep")).unwrap();
    std::fs::write(home.join("elsewhere/outside.yaml"), "name: [\n").unwrap();
    std::fs::write(repo.join("outside.yaml"), DOCS).unwrap();
    switch(&backend, &master, FIRST, &alpha, &repo).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let mut page = MenuPage::open(&backend, &master).await;

    let listed = page.when(&backend, &master, |_| true).await;
    assert_eq!(
        listed["workflows"],
        json!([
            { "id": "docs", "name": "Docs Review", "path": path(&workflows.join("docs.yaml")), "unavailable": null, "profiles": [] },
            { "id": "linked", "name": "Docs Review", "path": path(&workflows.join("linked.yaml")), "unavailable": null, "profiles": [] },
            { "id": "review", "name": "Review Team", "path": path(&workflows.join("review.yaml")), "unavailable": null, "profiles": ["explore", "slow"] },
        ])
    );

    // The Host follows `deep` before `..`: the file `check` reads is
    // `elsewhere/outside.yaml`, which does not parse, not `repo/outside.yaml`.
    let turn = work
        .turn(vec![
            shell("t1", "demi agent workflows check deep/../outside.yaml"),
            say("checked"),
        ])
        .await;
    let received = &turn.received[0];
    assert!(
        received.contains("demi agent workflows check: deep/../outside.yaml does not read: error: line 1 column 7"),
        "{received}"
    );

    // Review Team chosen at the turn's start; mid-turn the user chooses No
    // Subagents, Review Team's instructions change, and the user chooses it
    // again: once the runner said the file changed, the next request carries
    // them as the file is now, not as the turn began with.
    choose(&backend, &master, json!({ "type": "workflow", "id": "review" })).await;
    let started = repo.join("started");
    let go = repo.join("go");
    let before = work
        .start(vec![
            shell("t2", "touch started; while [ ! -e go ]; do sleep 0.05; done"),
            say("done"),
        ])
        .await;
    eventually("the command started", || {
        let exists = started.exists();
        async move { exists }
    })
    .await;
    choose(&backend, &master, json!({ "type": "none" })).await;
    let changed = REVIEW
        .replace("REVIEW STEPS", "REVIEW STEPS TWO")
        .replace("Review Team", "Review Team Two");
    std::fs::write(workflows.join("review.yaml"), changed).unwrap();
    page.when(&backend, &master, |listed| listed["workflows"][2]["name"] == "Review Team Two").await;
    choose(&backend, &master, json!({ "type": "workflow", "id": "review" })).await;
    std::fs::write(&go, "").unwrap();
    work.socket.until_idle().await;
    let turn = work.observe(before);
    assert_eq!(turn.requests.len(), 2);
    let first = turn.requests[0]["messages"].to_string();
    let second = turn.requests[1]["messages"].to_string();
    assert!(first.contains("REVIEW STEPS\\n</workflow>"), "{first}");
    assert!(second.contains("REVIEW STEPS TWO"), "{second}");

    // A `.demi` the Host cannot read is not looked past: its workflows are
    // unavailable, and none of a directory above is offered instead. The
    // watch reports a workflow added below it, and the search then finds it
    // unreadable.
    use std::os::unix::fs::PermissionsExt;
    // Not readable, but searchable, so the workflow can still be written.
    std::fs::set_permissions(repo.join(".demi"), std::fs::Permissions::from_mode(0o300)).unwrap();
    std::fs::write(workflows.join("added.yaml"), DOCS).unwrap();
    let listed = page
        .when(&backend, &master, |listed| {
            listed["unavailable"].as_str().is_some_and(|reason| reason.starts_with("the Host cannot read"))
        })
        .await;
    std::fs::set_permissions(repo.join(".demi"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let reason = listed["unavailable"].as_str().unwrap_or_default().to_owned();
    assert!(reason.starts_with(&format!("the Host cannot read {}: ", path(&repo.join(".demi")))), "{listed}");
    assert_eq!(listed["workflows"], json!([]));
    backend.close().await;
}

// About a second: a real runner on a paired device answers the reads.
#[tokio::test]
async fn a_new_conversation_lists_its_directorys_workflows_and_keeps_the_choice_it_was_given() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let repo = alpha.runner.home_dir().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join(".demi/workflows")).unwrap();
    std::fs::write(repo.join(".demi/workflows/review.yaml"), REVIEW).unwrap();
    let directory = repo.to_str().unwrap().to_owned();

    // Before its first message, through the device.
    let listed = backend
        .get(
            &format!(
                "/api/devices/{}/workflows?{}",
                alpha.id(),
                url::form_urlencoded::Serializer::new(String::new()).append_pair("directory", &directory).finish()
            ),
            Some(&master),
        )
        .await;
    assert_eq!(listed.status, StatusCode::OK);
    let KeptWorkflowList::Read { list: listed, .. } = listed.json::<KeptWorkflowList>() else {
        panic!("a device's directory is read before it answers");
    };
    assert_eq!(listed.workflows.len(), 1);
    assert_eq!(listed.workflows[0].name.as_deref(), Some("Review Team"));

    // A directory that is no absolute path, one that is not there, and a
    // device of another user's.
    let ask = |device: String, directory: &str| {
        let query = url::form_urlencoded::Serializer::new(String::new()).append_pair("directory", directory).finish();
        format!("/api/devices/{device}/workflows?{query}")
    };
    for relative in ["repo", ""] {
        let refused = backend.get(&ask(alpha.id().to_owned(), relative), Some(&master)).await;
        assert_eq!(refused.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody), "{relative:?}");
    }
    let missing = repo.join("missing").to_str().unwrap().to_owned();
    let gone = backend.get(&ask(alpha.id().to_owned(), &missing), Some(&master)).await;
    assert_eq!(gone.refusal(), (StatusCode::NOT_FOUND, ErrorCode::FsNotFound));
    harness.add_user("user@example.test", "user-pass-1", Role::User);
    let other = backend.login("user@example.test", "user-pass-1").await;
    let foreign = backend.get(&ask(alpha.id().to_owned(), &directory), Some(&other)).await;
    assert_eq!(foreign.refusal(), (StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound));

    // Its first send makes it with the choice.
    let created = backend
        .post(
            "/api/conversations",
            Some(&master),
            json!({
                "id": FIRST,
                "target": { "kind": "device", "deviceId": alpha.id(), "path": directory },
                "workflow": { "type": "workflow", "id": "review" },
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let created = created.json::<demi_web_api_protocol::conversations::CreatedConversation>();
    assert_eq!(created.metadata.workflow, WorkflowChoice::Workflow { id: "review".into() });
    backend.close().await;
}

// About five seconds: the first searches wait their whole hold for a runner
// that answers nothing.
//
// Planted defect it catches: the workflows searched after the instruction
// files, each holding the first request 5 seconds of its own.
#[tokio::test]
async fn with_a_workflow_chosen_the_first_request_waits_one_hold_for_both_searches() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let repo = alpha.runner.home_dir().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join(".demi/workflows")).unwrap();
    std::fs::write(repo.join(".demi/workflows/review.yaml"), REVIEW).unwrap();
    switch(&backend, &master, FIRST, &alpha, &repo).await;
    choose(&backend, &master, json!({ "type": "workflow", "id": "review" })).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;

    alpha.runner.suspend();
    let started = tokio::time::Instant::now();
    work.turn(vec![say("hello")]).await;
    let waited = started.elapsed();
    alpha.runner.resume();
    // One hold of 5 seconds, not two after each other.
    assert!(waited < std::time::Duration::from_secs(8), "{waited:?}");
    backend.close().await;
}

// About three seconds: a real runner on a paired device, gone before the
// backend restarts.
//
// Planted defect it catches: a list kept in memory only, which a backend
// that restarts while the Host is gone answers as unknown; or a read that
// waits for the Host.
#[tokio::test]
async fn the_menu_answers_what_was_read_at_once_unknown_before_and_after_a_restart_with_the_host_gone() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    // On the Cloud, which is not made yet: never read.
    create(&backend, &master, FIRST).await;
    assert_eq!(stored_menu(&crate::conversations::metadata(&backend, &master, FIRST).await.workflows.list), None);

    // On a paired device, read while the conversation shows.
    let mut alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    let repo = alpha.runner.home_dir().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join(".demi/workflows")).unwrap();
    std::fs::write(repo.join(".demi/workflows/review.yaml"), REVIEW).unwrap();
    switch(&backend, &master, FIRST, &alpha, &repo).await;
    let work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let mut page = MenuPage::open(&backend, &master).await;
    let listed = page.when(&backend, &master, |_| true).await;
    assert_eq!(listed["workflows"][0]["name"], "Review Team");
    drop((work, page));

    // The runner goes, then the backend restarts: the list is what was read.
    alpha.runner.stop().await;
    let address = backend.address();
    backend.close().await;
    let backend = harness.start_at(address).await;
    let kept = crate::conversations::metadata(&backend, &master, FIRST).await.workflows.list;
    let KeptWorkflowList::Read { list, .. } = kept else {
        panic!("the list read before the restart: {kept:?}");
    };
    assert_eq!(list.workflows[0].name.as_deref(), Some("Review Team"));
    backend.close().await;
}

/// The conversation `FIRST`'s Workflow menu choice as a page that opens it
/// now reads it.
async fn shown_choice(backend: &TestBackend, master: &Session) -> WorkflowChoice {
    Socket::shown(backend, master, FIRST, 0).await.1.workflow
}

// A few seconds: a real runner on a paired device answers the reads and runs
// the shell jobs of three turns and of two the user's answers wake.
//
// Planted defects it catches: a pick that writes the choice without
// announcing it, so the menu keeps the old one; a pick of a workflow that
// does not read, which then fails every spawn.
#[tokio::test]
async fn an_agent_picks_the_workflow_as_the_menu_does_once_the_user_allows_it() {
    use std::sync::Arc;

    use crate::agent_profiles::{assert_shows, last_root};
    use crate::permissions::{categories, decide, permissions, settled};
    use crate::subagents::{Scripts, background, say, shell, tree_on};

    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _device, root, _provider) = tree_on(&scripts, Harness::new()).await;
    let root = std::path::Path::new(&root);
    let workflows = root.join(".demi/workflows");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join("review.yaml"), REVIEW).unwrap();
    std::fs::write(
        workflows.join("broken.yaml"),
        "name: Broken\nprofiles:\n  - id: Reader\n    description: Reads.\n",
    )
    .unwrap();
    let mut page = MenuPage::open(&backend, &master).await;
    page.when(&backend, &master, |listed| listed["workflows"].as_array().is_some_and(|listed| listed.len() == 2))
        .await;
    let review = WorkflowChoice::Workflow { id: "review".into() };

    // The first pick waits for the user's permission, which the user denies:
    // the command is not run and the choice stays.
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root(
        FIRST,
        vec![background("t1", "demi agent workflows use review"), say("asked"), say("refused"), say("refused")],
    );
    socket.chat("m1", "Use the review workflow").await;
    assert_shows(
        &last_root(&scripts),
        "waiting for the user's permission to change its workflow; its end will report to you, so do not run it again",
    );
    let requested = permissions(&backend, &master, FIRST).await;
    assert_eq!(requested.requests.len(), 1, "{requested:?}");
    let request = &requested.requests[0];
    assert_eq!(request.command(), "demi agent workflows use review");
    let category = &categories(request)[0];
    assert_eq!(category.id, "agent.workflow");
    assert_eq!(category.action.as_deref(), Some("change its workflow"));
    assert_eq!(
        category.description.as_deref(),
        Some("Choose the workflow this conversation works with, as the composer's Workflow menu does.")
    );
    let denied = decide(&backend, &master, FIRST, request.id().as_str(), "deny").await;
    assert_eq!(denied.status, StatusCode::NO_CONTENT);
    let told = settled(
        &scripts,
        &backend,
        &master,
        FIRST,
        &["The user denied this conversation permission to change its workflow.", "(t1) ended with exit code 1"],
    )
    .await;
    assert_shows(
        &told,
        "demi agent workflows use: the user denied this conversation permission to change its workflow; the command was not run",
    );
    assert!(!told.contains("REVIEW STEPS"), "{told}");
    assert_eq!(shown_choice(&backend, &master).await, WorkflowChoice::Default);

    // Asked again and allowed, the pick is the menu's: the open page hears
    // of it.
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root_anew(
        FIRST,
        vec![background("t2", "demi agent workflows use review"), say("asked"), say("used"), say("used")],
    );
    socket.chat("m2", "Use the review workflow").await;
    let request = permissions(&backend, &master, FIRST).await.requests.remove(0);
    let allowed = decide(&backend, &master, FIRST, request.id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    let told = settled(
        &scripts,
        &backend,
        &master,
        FIRST,
        &["The user allowed this conversation to change its workflow.", "(t2) ended with exit code 0"],
    )
    .await;
    assert_shows(&told, "Workflow: review\nName: Review Team");
    page.socket.metadata_until(|metadata| metadata.workflow == review).await;

    // From then on, with no more asking: the next request carries its
    // instructions and a spawn takes its profiles; a workflow that does not
    // read, or that the project lacks, fails with why and leaves the choice.
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let script = [
        "demi agent spawn --profile slow <<< 'Think'",
        "demi agent workflows use broken",
        "demi agent workflows use nothing",
        "demi agent workflows list",
    ];
    scripts.root_anew(FIRST, vec![shell("t3", &script.join("\n")), say("tried")]);
    socket.chat("m3", "Try them").await;
    let asked = last_root(&scripts);
    assert_shows(&asked, "<workflow name=\"Review Team\">\nREVIEW STEPS\n</workflow>");
    let file = |id: &str| workflows.join(format!("{id}.yaml")).to_str().unwrap().to_owned();
    assert_shows(
        &asked,
        &format!("demi agent spawn: profile \"slow\" is unavailable: no provider entry of the user's offers model \"nowhere-model\"; change the profile's model in {}", file("review")),
    );
    assert_shows(
        &asked,
        &format!(
            "demi agent workflows use: workflow \"broken\" cannot be used: {} does not read: line 3 column 9: validation error",
            file("broken")
        ),
    );
    assert_shows(
        &asked,
        "demi agent workflows use: the project has no workflow \"nothing\"; demi agent workflows list lists them",
    );
    assert_shows(&asked, &format!("review  yes      available    {}", file("review")));
    assert!(permissions(&backend, &master, FIRST).await.requests.is_empty());
    assert_eq!(shown_choice(&backend, &master).await, review);

    // No Subagents, then Global Workflow, each as the menu shows it.
    let script = [
        "demi agent workflows use none",
        "demi agent spawn <<< 'Think'",
        "demi agent workflows use global",
    ];
    scripts.root_anew(FIRST, vec![shell("t4", &script.join("\n")), say("switched")]);
    socket.chat("m4", "Switch").await;
    let asked = last_root(&scripts);
    assert_shows(&asked, "Workflow: none\nName: No Subagents");
    assert_shows(&asked, "subagents are turned off for this conversation");
    assert_shows(&asked, "Workflow: global\nName: Global Workflow");
    page.socket.metadata_until(|metadata| metadata.workflow == WorkflowChoice::None).await;
    page.socket.metadata_until(|metadata| metadata.workflow == WorkflowChoice::Default).await;
    backend.close().await;
}
