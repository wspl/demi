//! Instructions on a paired device (`instructions.md`): the user's personal
//! instructions and the project's files reach the model as one block, whole,
//! one file per directory from the repository's root down to the working
//! directory, with the list the page shows; an unchanged block is sent
//! once, and a change sends the block again.

use demi_provider_common::testing::MockVendor;
use demi_shared_types::{Block, INSTRUCTIONS_SOURCE, InstructionEntry};
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, anthropic_at, create, transcript};
use crate::support::{Harness, Session, TestBackend};
use crate::work::{Driven, say, switch};

/// Saves `text` as the user's personal instructions and waits for the page
/// to learn of it.
async fn save(backend: &TestBackend, master: &Session, text: &str) {
    let mut page = backend.sync(master).await;
    page.snapshot().await;
    let saved = backend
        .put("/api/instructions", master, json!({ "text": text }))
        .await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT);
    let expected = if text.trim().is_empty() { "" } else { text };
    page.until(|event| matches!(event, SyncEvent::Instructions { instructions } if instructions == expected))
        .await;
}

// A few seconds: a real runner on a paired device answers the reads.
#[tokio::test]
async fn the_personal_instructions_and_the_projects_files_reach_the_model_whole_once_per_change() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;

    // `~/AGENTS.md` is above the repository; the root's `CLAUDE.md` links to
    // its `AGENTS.md`, which is longer than the replay bound; `web` has
    // only a `CLAUDE.md`.
    let home = alpha.runner.home_dir().to_owned();
    let repo = home.join("repo");
    let web = repo.join("web");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(home.join("AGENTS.md"), "OUTSIDE THE REPOSITORY").unwrap();
    let filler = "x".repeat(10_000);
    let root_text = format!("ROOT RULES\n{filler}\nMIDDLE RULE\n{filler}");
    std::fs::write(repo.join("AGENTS.md"), &root_text).unwrap();
    std::os::unix::fs::symlink("AGENTS.md", repo.join("CLAUDE.md")).unwrap();
    std::fs::write(web.join("CLAUDE.md"), "WEB RULES").unwrap();
    switch(&backend, &master, FIRST, &alpha, &web).await;

    let too_long = "x".repeat(65_537);
    let refused = backend
        .put("/api/instructions", &master, json!({ "text": too_long }))
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    save(&backend, &master, "Reply in Chinese.").await;

    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let first = work.turn(vec![say("hello")]).await.first_request();
    assert_eq!(first.matches("<personal_instructions>").count(), 1, "{first}");
    assert!(first.contains("Reply in Chinese."), "{first}");
    assert_eq!(first.matches("ROOT RULES").count(), 1, "{first}");
    assert!(first.contains("MIDDLE RULE"), "the root's file is sent whole");
    assert!(first.contains("WEB RULES"), "{first}");
    assert!(!first.contains("OUTSIDE THE REPOSITORY"), "{first}");
    assert!(
        first.find("ROOT RULES") < first.find("WEB RULES"),
        "the root's file comes first"
    );

    // What the page lists, from the block it holds.
    let root = repo.join("AGENTS.md").to_str().unwrap().to_owned();
    let nearer = web.join("CLAUDE.md").to_str().unwrap().to_owned();
    let listed = |blocks: &[Block]| -> Vec<Vec<InstructionEntry>> {
        blocks
            .iter()
            .filter_map(|block| match block {
                Block::Context(context) if context.source == INSTRUCTIONS_SOURCE => {
                    Some(context.instructions.clone())
                }
                _ => None,
            })
            .collect()
    };
    let personal = InstructionEntry::Personal { tokens: 5 };
    let root_entry = InstructionEntry::File {
        path: root.clone(),
        tokens: (root_text.len() as u64).div_ceil(4),
    };
    let web_entry = InstructionEntry::File {
        path: nearer.clone(),
        tokens: 3,
    };
    assert_eq!(
        listed(&transcript(&backend, &master, FIRST).await.blocks),
        vec![vec![personal, root_entry.clone(), web_entry.clone()]]
    );

    // Unchanged: no second block.
    let second = work.turn(vec![say("again")]).await.first_request();
    assert_eq!(second.matches("<personal_instructions>").count(), 1, "{second}");

    // Removed: a new block without them.
    save(&backend, &master, "  ").await;
    let third = work.turn(vec![say("third")]).await.first_request();
    assert_eq!(third.matches("ROOT RULES").count(), 2, "{third}");
    assert_eq!(
        listed(&transcript(&backend, &master, FIRST).await.blocks).last(),
        Some(&vec![root_entry, web_entry])
    );
}
