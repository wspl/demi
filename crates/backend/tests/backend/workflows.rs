//! A project's workflows on a paired device (`subagents.md` § Workflows of
//! a project, `instructions.md` § Which files are read): each directory's
//! `.demi/instructions.md` joins the instructions block after its
//! `AGENTS.md`; the Workflow menu's list reads `.demi/workflows/` with
//! what the page watches; a conversation's choice puts the workflow's
//! instructions in the root's block and its profiles in place of the
//! user's, a model no entry offers making a profile unavailable, and No
//! Subagents takes both away.

use demi_provider_common::testing::MockVendor;
use demi_shared_types::{Block, INSTRUCTIONS_SOURCE, InstructionEntry, WorkflowChoice};
use demi_web_api_protocol::conversations::ConversationUpdate;
use demi_web_api_protocol::workflows::ConversationWorkflows;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, anthropic_at, create, transcript};
use crate::support::{Harness, Session, TestBackend};
use crate::work::{Driven, say, shell, switch};

const REVIEW: &str = "name: Review Team
instructions: |
  REVIEW STEPS
profiles:
  - id: explore
    description: Finds code.
    model: claude-opus-4-8
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
    // the names of `web/.demi`, which could gain workflows nearer, and the
    // workflows directory's details.
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
                { "id": "broken", "name": null, "error": broken },
                { "id": "review", "name": "Review Team", "error": null },
            ],
            "watch": [
                { "kind": "folder_names", "path": path(&web.join(".demi")) },
                { "kind": "folder_details", "path": path(&workflows) },
            ],
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
            shell("t1", "demi agent profiles list; demi agent spawn --profile slow <<< 'Think'", 15_000),
            say("listed"),
        ])
        .await;
    assert!(
        turn.first_request().contains("<workflow name=\\\"Review Team\\\">\\nREVIEW STEPS\\n</workflow>"),
        "{}",
        turn.first_request()
    );
    let review = path(&workflows.join("review.yaml"));
    let unavailable = format!(
        "profile \"slow\" is unavailable: no provider entry of the user's offers model \"nowhere-model\"; change the profile's model in {review}, or ask the user to add a provider that offers it"
    );
    let received = &turn.received[0];
    assert!(received.contains("Workflow Review Team:\nexplore  Finds code.\nslow     Thinks long.\n"), "{received}");
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
