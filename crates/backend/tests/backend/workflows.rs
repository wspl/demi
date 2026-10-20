//! A project's workflows on a paired device
//! (`crates/backend-user-shard/src/conversation/workflows.rs`).
//!
//! Scenarios:
//!
//! - Each directory's `.demi/instructions.md` joins the instructions block
//!   after its `AGENTS.md`.
//! - The Workflow menu's list reads `.demi/workflows/` with what the page
//!   watches.
//! - A conversation's choice puts the workflow's instructions in the root's
//!   block and its profiles in place of the user's, a model no entry offers
//!   making a profile unavailable. No Subagents takes both away.
//! - What the page watches changes for a workflow added or edited, or a
//!   `.demi` that comes nearer, and not for the other files of the
//!   directories searched.

use demi_provider_common::testing::MockVendor;
use demi_shared_types::{Block, INSTRUCTIONS_SOURCE, InstructionEntry, WorkflowChoice};
use demi_web_api_protocol::conversations::ConversationUpdate;
use demi_web_api_protocol::workflows::ConversationWorkflows;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::auth::Role;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, anthropic_at, create, transcript};
use crate::files::{send_watch, watch_message, watch_socket_of};
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
    chosen.json::<ConversationUpdate>().conversation.workflow
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

    // The menu's list, with the Host's number and what the page watches:
    // the `.git` that would make `web` the root, `web/.demi/workflows`,
    // which would be nearer, and the workflow files.
    let listed = backend
        .get(&format!("/api/conversations/{FIRST}/workflows"), Some(&master))
        .await;
    assert_eq!(listed.status, StatusCode::OK);
    assert!(listed.headers.contains_key("x-demi-read-at"));
    let listed: serde_json::Value = serde_json::to_value(listed.json::<ConversationWorkflows>()).unwrap();
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
            "watch": [
                { "kind": "file", "path": path(&web.join(".git")) },
                { "kind": "file", "path": path(&web.join(".demi/workflows")) },
                { "kind": "folder_details", "path": path(&workflows), "suffix": ".yaml" },
            ],
            "unavailable": null,
        })
    );

    // Default: Demi's own files follow each directory's AGENTS.md, and no
    // workflow is in the block.
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
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
                15_000,
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
        .turn(vec![shell("t2", "demi agent spawn <<< 'Think'", 15_000), say("refused")])
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

/// The Workflow menu's list of the conversation `FIRST`.
async fn menu(backend: &TestBackend, master: &Session) -> serde_json::Value {
    let listed = backend
        .get(&format!("/api/conversations/{FIRST}/workflows"), Some(master))
        .await;
    assert_eq!(listed.status, StatusCode::OK);
    serde_json::to_value(listed.json::<ConversationWorkflows>()).unwrap()
}

/// The menu's list once `holds` says it holds a change made on the Host,
/// which the backend learns of through the runner's file watch, as a page
/// that heard of it reads the list again.
async fn menu_once(
    backend: &TestBackend,
    master: &Session,
    holds: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let listed = std::cell::RefCell::new(serde_json::Value::Null);
    eventually("the menu holds the change", || async {
        let now = menu(backend, master).await;
        let held = holds(&now);
        listed.replace(now);
        held
    })
    .await;
    listed.into_inner()
}

/// The `changed` batches the watch sends until one names `key`: every
/// other key they name but `tree`'s.
async fn others_until(socket: &mut crate::files::Socket, key: &str) -> Vec<String> {
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
// and watches what the page subscribes to.
//
// Planted defect it catches: a search that watches a searched directory's
// names, or every entry of the workflows directory, so that any file made
// in a home folder or a repository's root reads the list again.
#[tokio::test]
async fn the_page_hears_of_workflows_and_a_nearer_demi_but_not_of_other_files_beside_them() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    create(&backend, &master, FIRST).await;
    let path = |path: &std::path::Path| path.to_str().unwrap().to_owned();

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

    // The page watches the repository's changes list, so one watch of its
    // tree carries everything below, in the order it happens.
    let mut socket = watch_socket_of(&backend, &master, &format!("/api/conversations/{FIRST}/fs/watch")).await;
    let tree = json!({ "key": "tree", "kind": "worktree", "path": path(&repo) });
    send_watch(&mut socket, json!({ "type": "subscribe", "subscriptions": [tree] })).await;
    let live = watch_message(&mut socket).await;
    assert_eq!(live["type"], "live", "{live}");

    // It subscribes to what the list says to watch, under keys of its own.
    let listed = menu(&backend, &master).await;
    assert_eq!(
        listed["watch"],
        json!([
            { "kind": "file", "path": path(&web.join(".git")) },
            { "kind": "file", "path": path(&app.join(".git")) },
            { "kind": "file", "path": path(&web.join(".demi")) },
            { "kind": "file", "path": path(&app.join(".demi/workflows")) },
            { "kind": "folder_details", "path": path(&workflows), "suffix": ".yaml" },
        ])
    );
    let subscriptions: Vec<serde_json::Value> = listed["watch"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, subject)| {
            let mut subscription = subject.clone();
            subscription["key"] = json!(format!("w{index}"));
            subscription
        })
        .collect();
    send_watch(&mut socket, json!({ "type": "subscribe", "subscriptions": subscriptions })).await;
    for _ in 0..subscriptions.len() {
        let live = watch_message(&mut socket).await;
        assert_eq!(live["type"], "live", "{live}");
    }

    // Files beside what the search looked for, then a new workflow: only
    // the workflow files' subscription changes.
    std::fs::write(web.join("notes.txt"), "notes").unwrap();
    std::fs::write(app.join("notes.txt"), "notes").unwrap();
    std::fs::write(app.join(".demi/notes.md"), "notes").unwrap();
    std::fs::write(repo.join("notes.txt"), "notes").unwrap();
    std::fs::write(workflows.join("docs.yaml"), DOCS).unwrap();
    assert_eq!(others_until(&mut socket, "w4").await, Vec::<String>::new());
    // A file of the workflows directory that is no workflow, then a `.demi`
    // made in the working directory, which may bring workflows nearer: only
    // the `.demi`'s subscription changes.
    std::fs::write(workflows.join("README.md"), "notes").unwrap();
    std::fs::create_dir(web.join(".demi")).unwrap();
    assert_eq!(others_until(&mut socket, "w2").await, Vec::<String>::new());
    // A workflow edited in place.
    std::fs::write(workflows.join("review.yaml"), REVIEW.replace("REVIEW STEPS", "REVIEW STEPS TWO")).unwrap();
    assert_eq!(others_until(&mut socket, "w4").await, Vec::<String>::new());
    backend.close().await;
}

// A few seconds: a real runner on a paired device answers the reads, and
// three turns run a shell job each.
#[tokio::test]
async fn links_are_refused_check_lets_the_host_resolve_and_a_new_choice_reads_its_instructions_at_once() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let path = |path: &std::path::Path| path.to_str().unwrap().to_owned();

    // `repo/.demi/workflows` holds review.yaml, docs.yaml, and linked.yaml,
    // a link to a workflow outside the project.
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

    let listed = menu(&backend, &master).await;
    assert_eq!(
        listed["workflows"],
        json!([
            { "id": "docs", "name": "Docs Review", "path": path(&workflows.join("docs.yaml")), "unavailable": null, "profiles": [] },
            { "id": "linked", "name": null, "path": path(&workflows.join("linked.yaml")), "unavailable": "Workflows are files of the project, not links.", "profiles": [] },
            { "id": "review", "name": "Review Team", "path": path(&workflows.join("review.yaml")), "unavailable": null, "profiles": ["explore", "slow"] },
        ])
    );

    // Chosen, the linked workflow refuses spawns with that reason.
    choose(&backend, &master, json!({ "type": "workflow", "id": "linked" })).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let turn = work
        .turn(vec![
            shell(
                "t1",
                "demi agent spawn <<< 'Think'; demi agent workflows check deep/../outside.yaml",
                15_000,
            ),
            say("refused"),
        ])
        .await;
    let received = &turn.received[0];
    assert!(
        received.contains("this conversation's workflow \"linked\" is unavailable: Workflows are files of the project, not links."),
        "{received}"
    );
    // The Host follows `deep` before `..`: the file it reads is
    // `elsewhere/outside.yaml`, which does not parse, not `repo/outside.yaml`.
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
            shell("t2", "touch started; while [ ! -e go ]; do sleep 0.05; done", 60_000),
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
    menu_once(&backend, &master, |listed| listed["workflows"][2]["name"] == "Review Team Two").await;
    choose(&backend, &master, json!({ "type": "workflow", "id": "review" })).await;
    std::fs::write(&go, "").unwrap();
    work.socket.until_idle().await;
    let turn = work.observe(before);
    assert_eq!(turn.requests.len(), 2);
    let first = turn.requests[0]["messages"].to_string();
    let second = turn.requests[1]["messages"].to_string();
    assert!(first.contains("REVIEW STEPS\\n</workflow>"), "{first}");
    assert!(second.contains("REVIEW STEPS TWO"), "{second}");

    // `.demi/workflows` itself a link: nothing in it is read, and the menu
    // says why.
    std::fs::remove_dir_all(&workflows).unwrap();
    std::fs::create_dir_all(home.join("shared-workflows")).unwrap();
    std::fs::write(home.join("shared-workflows/review.yaml"), REVIEW).unwrap();
    std::os::unix::fs::symlink(home.join("shared-workflows"), &workflows).unwrap();
    let listed = menu_once(&backend, &master, |listed| !listed["unavailable"].is_null()).await;
    assert_eq!(listed["directory"], json!(path(&workflows)));
    assert_eq!(listed["unavailable"], json!("Workflows are files of the project, not links."));
    assert_eq!(listed["workflows"], json!([]));
    assert_eq!(listed["watch"], json!([{ "kind": "folder_names", "path": path(&repo.join(".demi")) }]));

    // `.demi` itself a link, to a directory whose `workflows` holds a
    // workflow: nothing behind it is listed or read.
    std::fs::rename(repo.join(".demi"), home.join("old-demi")).unwrap();
    std::fs::create_dir_all(home.join("shared-demi/workflows")).unwrap();
    std::fs::write(home.join("shared-demi/workflows/review.yaml"), REVIEW).unwrap();
    std::os::unix::fs::symlink(home.join("shared-demi"), repo.join(".demi")).unwrap();
    // Between the rename and the link the repository has no `.demi`, which
    // the menu also watches for in `repo`, with no directory.
    let listed = menu_once(&backend, &master, |listed| {
        listed["watch"][0]["path"] == path(&repo) && listed["directory"] == path(&workflows)
    })
    .await;
    assert_eq!(listed["directory"], json!(path(&workflows)));
    assert_eq!(listed["unavailable"], json!("Workflows are files of the project, not links."));
    assert_eq!(listed["workflows"], json!([]));
    assert_eq!(listed["watch"], json!([{ "kind": "folder_names", "path": path(&repo) }]));

    // A `.demi` the Host cannot read is not looked past: its workflows are
    // unavailable, and none of a directory above is offered instead. It
    // arrives unreadable, in one rename the watch reports.
    let locked = home.join("locked-demi");
    std::fs::create_dir_all(locked.join("workflows")).unwrap();
    std::fs::write(locked.join("workflows/review.yaml"), REVIEW).unwrap();
    use std::os::unix::fs::PermissionsExt;
    // Not readable, but writable, which a rename of a directory needs.
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o300)).unwrap();
    std::fs::remove_file(repo.join(".demi")).unwrap();
    std::fs::rename(&locked, repo.join(".demi")).unwrap();
    let listed = menu_once(&backend, &master, |listed| {
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
    assert!(listed.headers.contains_key("x-demi-read-at"));
    let listed = listed.json::<ConversationWorkflows>();
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
    assert_eq!(created.conversation.workflow, WorkflowChoice::Workflow { id: "review".into() });
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

/// The menu's list of `FIRST` when no search of its directory is held: no
/// workflows, read at no number, with the working directory to watch.
async fn assert_nothing_held(backend: &TestBackend, master: &Session, directory: &str) {
    let listed = backend.get(&format!("/api/conversations/{FIRST}/workflows"), Some(master)).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&listed.body));
    assert!(!listed.headers.contains_key("x-demi-read-at"));
    assert_eq!(
        serde_json::to_value(listed.json::<ConversationWorkflows>()).unwrap(),
        json!({
            "directory": null,
            "unavailable": null,
            "workflows": [],
            "watch": [{ "kind": "folder_names", "path": directory }],
        }),
    );
}

// About three seconds: a real runner on a paired device, stopped midway.
//
// Planted defect it catches: a Host that does not run answering 409
// host_stopped, as it did, and a refused search answering that too.
#[tokio::test]
async fn a_host_that_does_not_run_lists_no_workflows_and_a_refused_search_answers_its_refusal() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    // On the Cloud, which is not made yet.
    create(&backend, &master, FIRST).await;
    assert_nothing_held(&backend, &master, &format!("/home/demi/sessions/{FIRST}")).await;

    // On a paired device whose runner is gone.
    let mut alpha = backend.pair(&master, "alpha").await;
    let repo = alpha.runner.home_dir().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let directory = repo.to_str().unwrap().to_owned();
    let moved = backend
        .patch(
            &format!("/api/conversations/{FIRST}"),
            &master,
            json!({ "target": { "kind": "device", "deviceId": alpha.id(), "path": directory }, "notifyAgent": false }),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    alpha.runner.stop().await;
    assert_nothing_held(&backend, &master, &directory).await;

    // Archived: the search is refused, and the answer says why.
    let archived = backend.patch(&format!("/api/conversations/{FIRST}"), &master, json!({ "archived": true })).await;
    assert_eq!(archived.status, StatusCode::OK, "{}", String::from_utf8_lossy(&archived.body));
    let refused = backend.get(&format!("/api/conversations/{FIRST}/workflows"), Some(&master)).await;
    assert_eq!(refused.refusal(), (StatusCode::CONFLICT, ErrorCode::ConversationArchived));
    backend.close().await;
}
