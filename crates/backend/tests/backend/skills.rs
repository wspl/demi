//! Skills on a paired device (`skills.md` § Acceptance, `plugins.md` § Host
//! directories): a user skill turned on reaches the model's catalog beside
//! the repository's own skills, the job that reads it finds its directory
//! installed, read-only and with its script executable, a conversation that
//! runs no job installs nothing, a skill turned off leaves the Host at the
//! next job, and the plugin turned off adds no block. A job that `demi host
//! shell` runs on an attached device finds the skill installed there too.

use std::sync::Arc;

use demi_plugin_skills::testing::{Repos, skill_md};
use demi_provider_common::testing::MockVendor;
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{FIRST, anthropic_at, create, summary};
use crate::support::{Harness, Session, TestBackend};
use crate::work::{Driven, say, shell, switch};

/// How many of the skills plugin's blocks the request `request` carries:
/// catalogs, and the word that none is left.
fn skill_blocks(request: &Value) -> usize {
    let messages = request["messages"].to_string();
    messages.matches("<available_skills>").count()
        + messages.matches("No skills are available now.").count()
}

// Several seconds: a real runner on a paired device runs the jobs that find
// the skill installed and then removed.
/// The repository `acme/tools` with the skill `review`, whose script is
/// executable.
fn tools_repos() -> Arc<Repos> {
    let repos = Arc::new(Repos::new());
    repos.commit(
        "acme/tools",
        &[
            (
                "review/SKILL.md",
                &skill_md("name: review\ndescription: Review a change."),
                false,
            ),
            ("review/check.sh", "#!/bin/sh\necho checked\n", true),
        ],
    );
    repos
}

/// Adds `acme/tools` as a source of the user's skills and waits for its
/// first fetch to end; answers the source's id.
async fn add_tools(backend: &TestBackend, master: &Session) -> String {
    let mut page = backend.sync(master).await;
    page.snapshot().await;
    let added = backend
        .post(
            "/api/plugins/skills/calls/add_source",
            Some(master),
            json!({ "origin": "acme/tools" }),
        )
        .await;
    assert_eq!(added.status, StatusCode::OK);
    let source = added.json::<Value>()["source"].as_str().unwrap().to_owned();
    page.until(|event| {
        matches!(event, SyncEvent::Plugin { plugin, state }
            if plugin == "skills" && state["sources"][0]["commit"].is_string() && state["sources"][0]["fetching"] == false)
    })
    .await;
    source
}

/// Turns the skill `review` of `source` on or off.
async fn set_review(
    backend: &TestBackend,
    master: &Session,
    source: &str,
    enabled: bool,
) -> StatusCode {
    backend
        .post(
            "/api/plugins/skills/calls/set_enabled",
            Some(master),
            json!({ "source": source, "skill": "review", "enabled": enabled }),
        )
        .await
        .status
}

#[tokio::test]
async fn a_skill_on_reaches_the_catalog_and_the_jobs_that_need_it_find_it_installed() {
    let repos = tools_repos();
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_skill_repos(&repos);
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let home = alpha.runner.home_dir().to_owned();
    // The repository's own skill, in the working directory.
    let release = home.join(".agents/skills/release");
    std::fs::create_dir_all(&release).unwrap();
    std::fs::write(
        release.join("SKILL.md"),
        skill_md("name: release\ndescription: Cut a release."),
    )
    .unwrap();
    switch(&backend, &master, FIRST, &alpha, &home).await;

    let source = add_tools(&backend, &master).await;
    assert_eq!(
        set_review(&backend, &master, &source, true).await,
        StatusCode::OK
    );

    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let talked = work.turn(vec![say("hello")]).await;
    let messages = talked.requests[0]["messages"].to_string();
    assert!(messages.contains("<name>review</name>"), "{messages}");
    assert!(
        messages.contains("<location>~/.demi/plugins/skills/review-"),
        "{messages}"
    );
    assert!(
        messages.contains("/.agents/skills/release/SKILL.md</location>"),
        "{messages}"
    );
    // No job ran, so nothing was installed.
    assert!(!home.join(".demi/plugins/skills").exists());

    // The mode bits, which root's writes would ignore.
    let script = "cat ~/.demi/plugins/skills/review-*/SKILL.md && ~/.demi/plugins/skills/review-*/check.sh && ls -l ~/.demi/plugins/skills/review-*/";
    let read = work
        .turn(vec![shell("t1", script, 10_000), say("read")])
        .await;
    let output = &read.received[0];
    assert!(output.contains("Review a change."), "{output}");
    assert!(output.contains("checked"), "{output}");
    assert!(
        output.contains("-r-xr-xr-x") && output.contains("-r--r--r--"),
        "{output}"
    );

    // Off: the next job finds it gone.
    assert_eq!(
        set_review(&backend, &master, &source, false).await,
        StatusCode::OK
    );
    let listed = work
        .turn(vec![
            shell("t2", "ls -A ~/.demi/plugins/skills", 10_000),
            say("listed"),
        ])
        .await;
    assert!(
        !listed.received[0].contains("review-"),
        "{}",
        listed.received[0]
    );

    // The plugin off is no context source: no catalog joins the requests.
    let off = backend
        .put("/api/plugins/skills", &master, json!({ "enabled": false }))
        .await;
    assert_eq!(off.status, StatusCode::NO_CONTENT);
    let before = skill_blocks(listed.requests.last().unwrap());
    // On, the skills plugin would tell that none is left.
    std::fs::remove_dir_all(&release).unwrap();
    let quiet = work.turn(vec![say("quiet")]).await;
    assert_eq!(skill_blocks(&quiet.requests[0]), before);
    backend.close().await;
}

// Several seconds: real runners on two paired devices, and one turn whose
// job runs another on the attached one.
#[tokio::test]
async fn a_job_run_on_an_attached_device_finds_the_skill_installed_there_and_counts_as_a_job() {
    let repos = tools_repos();
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_skill_repos(&repos);
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let beta = backend.pair(&master, "beta").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    // Beta, left before any job ran there, stays attached without the
    // plugins' directories.
    let on_beta = beta.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &beta, &on_beta).await;
    let on_alpha = alpha.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &alpha, &on_alpha).await;
    let source = add_tools(&backend, &master).await;
    assert_eq!(
        set_review(&backend, &master, &source, true).await,
        StatusCode::OK
    );

    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let script = "demi host shell --host beta 'cat ~/.demi/plugins/skills/review-*/SKILL.md'";
    let read = work
        .turn(vec![shell("t1", script, 20_000), say("read")])
        .await;
    let output = &read.received[0];
    assert!(output.contains("Review a change."), "{output}");
    // Two jobs of the conversation ended: the one on alpha, and the one it
    // ran on beta.
    let ended = summary(&backend, &master, FIRST).await;
    assert_eq!(ended.working_tree_revision, 2);
    backend.close().await;
}
