//! Skills on a paired device (`skills.md` § Acceptance, `plugins.md` § Host
//! directories): a user skill turned on reaches the model's catalog beside
//! the repository's own skills, the job that reads it finds its directory
//! installed, read-only and with its script executable, a conversation that
//! runs no job installs nothing, a skill turned off leaves the Host at the
//! next job, and the plugin turned off adds no block.

use std::sync::Arc;

use demi_plugin_skills::testing::{Repos, skill_md};
use demi_provider_common::testing::MockVendor;
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{FIRST, anthropic_at, create};
use crate::support::Harness;
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
#[tokio::test]
async fn a_skill_on_reaches_the_catalog_and_the_jobs_that_need_it_find_it_installed() {
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

    let mut page = backend.sync(&master).await;
    page.snapshot().await;
    let added = backend
        .post(
            "/api/plugins/skills/calls/add_source",
            Some(&master),
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
    let switch_skill = |enabled: bool| {
        backend.post(
            "/api/plugins/skills/calls/set_enabled",
            Some(&master),
            json!({ "source": source, "skill": "review", "enabled": enabled }),
        )
    };
    assert_eq!(switch_skill(true).await.status, StatusCode::OK);

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
    assert_eq!(switch_skill(false).await.status, StatusCode::OK);
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
