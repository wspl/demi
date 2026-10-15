//! Instructions on a paired device (`instructions.md`): the user's personal
//! instructions and the project's files reach the model as one block, whole,
//! one file per directory from the repository's root down to the working
//! directory, with the list the page shows; an unchanged block is sent
//! once, and a change sends the block again. The project's files and skills
//! are kept by the runner's file watch: an edit reaches a request without a
//! search per request, a Host that stops answering holds no request, and a
//! directory's first search holds a request for at most 5 seconds.

use std::time::Duration;

use demi_plugin_skills::testing::skill_md;
use demi_provider_common::testing::MockVendor;
use demi_shared_types::{Block, INSTRUCTIONS_SOURCE, InstructionEntry};
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, anthropic_at, create, transcript};
use crate::support::{Harness, PATIENCE, Session, TestBackend};
use crate::work::{Driven, Turn, say, switch};

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
    // its `AGENTS.md`, which is far longer than the replay bound; `web` has
    // only a `CLAUDE.md`.
    let home = alpha.runner.home_dir().to_owned();
    let repo = home.join("repo");
    let web = repo.join("web");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(home.join("AGENTS.md"), "OUTSIDE THE REPOSITORY").unwrap();
    // Over 256 KiB in all: no size keeps a file out or cuts it.
    let filler = "x".repeat(150_000);
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
    let personal = InstructionEntry::Personal;
    let root_entry = InstructionEntry::File { path: root.clone() };
    let web_entry = InstructionEntry::File { path: nearer.clone() };
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

/// Runs turns until a request carries every one of `texts`, each turn
/// waiting for its own end; the last turn.
pub(crate) async fn until_carried(work: &mut Driven<'_>, texts: &[&str]) -> Turn {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        let turn = work.turn(vec![say("again")]).await;
        let request = turn.first_request();
        if texts.iter().all(|text| request.contains(text)) {
            return turn;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no request carried {texts:?}: {request}"
        );
    }
}

/// How long a turn whose requests wait for no Host may take, far below the
/// two minutes a read of a silent Host waits.
const UNHELD: Duration = Duration::from_secs(10);

// A few seconds: a real runner on a paired device answers the first search
// and reports the edits, then stops answering.
//
// Planted defect it catches: a request that searches the Host again, which
// waits for a Host that never answers.
#[tokio::test]
async fn an_edit_reaches_a_request_through_the_watch_and_a_silent_host_holds_none() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let repo = alpha.runner.home_dir().join("repo");
    let skill = repo.join(".agents/skills/release");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(repo.join("AGENTS.md"), "FIRST RULES").unwrap();
    std::fs::write(skill.join("SKILL.md"), skill_md("name: release\ndescription: Cut a release.")).unwrap();
    switch(&backend, &master, FIRST, &alpha, &repo).await;

    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let first = work.turn(vec![say("hello")]).await.first_request();
    assert!(first.contains("FIRST RULES"), "{first}");
    assert!(first.contains("Cut a release."), "{first}");

    // The runner reports the edits, and a later request carries them.
    std::fs::write(repo.join("AGENTS.md"), "SECOND RULES").unwrap();
    std::fs::write(skill.join("SKILL.md"), skill_md("name: release\ndescription: Cut a patch.")).unwrap();
    until_carried(&mut work, &["SECOND RULES", "Cut a patch."]).await;

    // The Host answers nothing now: a request takes what is held.
    alpha.runner.suspend();
    let silent = tokio::time::timeout(UNHELD, work.turn(vec![say("silent")]))
        .await
        .expect("no request waits for the Host");
    assert_eq!(silent.requests.len(), 1);
    alpha.runner.resume();
    backend.close().await;
}

// About five seconds: the first search waits its whole hold for a runner
// that answers nothing.
//
// Planted defect it catches: a first request that waits for the search
// however long it takes.
#[tokio::test]
async fn the_first_search_of_a_directory_holds_a_request_for_at_most_five_seconds() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let repo = alpha.runner.home_dir().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join("AGENTS.md"), "PROJECT RULES").unwrap();
    switch(&backend, &master, FIRST, &alpha, &repo).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;

    alpha.runner.suspend();
    let first = tokio::time::timeout(UNHELD, work.turn(vec![say("hello")]))
        .await
        .expect("the first search holds the request 5 seconds at most")
        .first_request();
    assert!(!first.contains("PROJECT RULES"), "{first}");

    // The search answers once the Host does, and a later request carries
    // what it found.
    alpha.runner.resume();
    until_carried(&mut work, &["PROJECT RULES"]).await;
    backend.close().await;
}

// A few seconds: two real runners on paired devices answer the searches and
// hold the subscriptions; the conversation moves twice.
//
// Planted defect it catches: a move that leaves what the conversation kept
// where it worked, subscribed until its tree closes.
#[tokio::test]
async fn a_move_releases_what_the_conversation_kept_where_it_worked() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let beta = backend.pair(&master, "beta").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let first_dir = alpha.runner.home_dir().join("one");
    let second_dir = alpha.runner.home_dir().join("two");
    for directory in [&first_dir, &second_dir] {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(directory.join("AGENTS.md"), "RULES").unwrap();
    }
    let watched = async |device: &str, directory: &std::path::Path| {
        let directory = directory.to_str().unwrap().to_owned();
        backend
            .subscribed(&master, device)
            .await
            .iter()
            .any(|path| path.starts_with(&directory))
    };
    switch(&backend, &master, FIRST, &alpha, &first_dir).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    work.turn(vec![say("hello")]).await;
    assert!(watched(alpha.id(), &first_dir).await);

    // Another directory of the same Host: the first is released at the move.
    switch(&backend, &master, FIRST, &alpha, &second_dir).await;
    crate::support::eventually("the first directory is released", || async {
        !watched(alpha.id(), &first_dir).await
    })
    .await;
    work.turn(vec![say("moved")]).await;
    assert!(watched(alpha.id(), &second_dir).await);

    // Another Host: nothing stays watched on the first.
    let on_beta = beta.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &beta, &on_beta).await;
    crate::support::eventually("nothing is watched on alpha", || async {
        !watched(alpha.id(), alpha.runner.home_dir()).await
    })
    .await;
    backend.close().await;
}
