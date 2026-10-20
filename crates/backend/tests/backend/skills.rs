//! Skills on a paired device
//! (`crates/backend-host-access/src/plugin_files.rs`).
//!
//! Scenarios:
//!
//! - A user skill turned on reaches the model's catalog beside the
//!   repository's own skills.
//! - The job that reads it finds its directory installed, read-only and with
//!   its script executable.
//! - A conversation that runs no job installs nothing.
//! - A skill turned off leaves the Host at the next job, and the plugin turned
//!   off adds no block.
//! - A job that `demi host shell` runs on an attached device finds the skill
//!   installed there too.
//! - What the search watches changes for a skill added or edited, or a
//!   skills directory that comes, and not for the other files beside them.

use std::sync::Arc;

use demi_plugin_skills::testing::{Repos, skill_md};
use demi_provider_common::testing::MockVendor;
use demi_runner_protocol::files::{Subject, SubscriptionKind};
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{FIRST, anthropic_at, create};
use crate::files::{send_watch, watch_message, watch_socket_of};
use crate::instructions::until_carried;
use crate::support::{Harness, Session, TestBackend};
use crate::work::{Driven, into_project, say, shell, switch};

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
    // The repository's own skill, in the project's directory.
    let release = home.join("app/.agents/skills/release");
    std::fs::create_dir_all(&release).unwrap();
    std::fs::write(
        release.join("SKILL.md"),
        skill_md("name: release\ndescription: Cut a release."),
    )
    .unwrap();
    into_project(&backend, &master, FIRST, &alpha, &home.join("app")).await;

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
async fn a_job_run_on_an_attached_device_finds_the_skill_installed_there() {
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
    backend.close().await;
}

// A few seconds: a real runner on a paired device answers the first search
// and reports each change of the project's skills.
//
// Planted defect it catches: a search that watches only the skills
// directories and the skills it found, which misses all three changes.
#[tokio::test]
async fn a_skill_created_added_to_a_group_or_repaired_reaches_a_later_request() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let skills = alpha.runner.home_dir().join("app/.agents/skills");
    let manifest = |name: &str| skill_md(&format!("name: {name}\ndescription: The skill {name}."));
    std::fs::create_dir_all(skills.join("empty")).unwrap();
    std::fs::create_dir_all(skills.join("group/one")).unwrap();
    std::fs::create_dir_all(skills.join("broken")).unwrap();
    std::fs::write(skills.join("group/one/SKILL.md"), manifest("one")).unwrap();
    std::fs::write(skills.join("broken/SKILL.md"), "not a skill").unwrap();
    into_project(&backend, &master, FIRST, &alpha, &alpha.runner.home_dir().join("app")).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let first = work.turn(vec![say("hello")]).await.first_request();
    assert!(first.contains("<name>one</name>"), "{first}");
    assert!(!first.contains("<name>broken</name>"), "{first}");

    std::fs::write(skills.join("empty/SKILL.md"), manifest("empty")).unwrap();
    std::fs::create_dir_all(skills.join("group/two")).unwrap();
    std::fs::write(skills.join("group/two/SKILL.md"), manifest("two")).unwrap();
    std::fs::write(skills.join("broken/SKILL.md"), manifest("broken")).unwrap();
    until_carried(&mut work, &["<name>empty</name>", "<name>two</name>", "<name>broken</name>"]).await;
    backend.close().await;
}

// One to four seconds: a real runner on a paired device answers the search,
// and the page subscribes to what the search watches.
//
// Fixed bug: a project without `.claude` was subscribed to the names of
// `.claude/skills`, which a runner on Linux or Windows watches through
// `.claude` and so fails to watch, and a skills directory made later went
// unnoticed.
#[tokio::test]
async fn the_skills_search_hears_of_skills_and_a_new_skills_directory_but_not_of_other_files() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let path = |path: &std::path::Path| path.to_str().unwrap().to_owned();

    // `app` has `.agents/skills` with a skill and a group of one, and no
    // `.claude`.
    let app = alpha.runner.home_dir().join("app");
    let skills = app.join(".agents/skills");
    let claude = app.join(".claude");
    let manifest = |name: &str| skill_md(&format!("name: {name}\ndescription: The skill {name}."));
    std::fs::create_dir_all(skills.join("release")).unwrap();
    std::fs::create_dir_all(skills.join("group/one")).unwrap();
    std::fs::write(skills.join("release/SKILL.md"), manifest("release")).unwrap();
    std::fs::write(skills.join("group/one/SKILL.md"), manifest("one")).unwrap();
    let initialized = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&app)
        .status()
        .unwrap();
    assert!(initialized.success());
    into_project(&backend, &master, FIRST, &alpha, &app).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let first = work.turn(vec![say("hello")]).await.first_request();
    assert!(first.contains("<name>release</name>"), "{first}");

    // The search watches the skills directory and the group's names, each
    // `SKILL.md` it looked for, and the coming of `.claude`.
    let subject = |kind, path: &std::path::Path| Subject {
        kind,
        path: path.to_str().unwrap().to_owned(),
        suffix: None,
    };
    let mut expected = vec![
        subject(SubscriptionKind::File, &skills.join("release/SKILL.md")),
        subject(SubscriptionKind::File, &skills.join("group/SKILL.md")),
        subject(SubscriptionKind::File, &skills.join("group/one/SKILL.md")),
        subject(SubscriptionKind::File, &claude),
        subject(SubscriptionKind::FolderNames, &skills),
        subject(SubscriptionKind::FolderNames, &skills.join("group")),
    ];
    expected.sort();
    let mut watched: Vec<Subject> = backend
        .subscribed(&master, alpha.id())
        .await
        .into_iter()
        .filter(|subject| [&skills, &claude].iter().any(|under| subject.path.starts_with(&path(under))))
        .collect();
    watched.sort();
    assert_eq!(watched, expected);

    // The page holds the repository's tree, so one watch carries everything
    // below in the order it happens, and the search's subjects under keys of
    // its own.
    let mut socket = watch_socket_of(&backend, &master, &format!("/api/conversations/{FIRST}/fs/watch")).await;
    let tree = json!({ "key": "tree", "kind": "worktree", "path": path(&app) });
    send_watch(&mut socket, json!({ "type": "subscribe", "subscriptions": [tree] })).await;
    let live = watch_message(&mut socket).await;
    assert_eq!(live["type"], "live", "{live}");
    let key = |wanted: &Subject| format!("w{}", watched.iter().position(|subject| subject == wanted).unwrap());
    let subscriptions: Vec<Value> = watched
        .iter()
        .map(|subject| json!({ "key": key(subject), "kind": subject.kind, "path": subject.path }))
        .collect();
    send_watch(&mut socket, json!({ "type": "subscribe", "subscriptions": subscriptions })).await;
    for _ in 0..subscriptions.len() {
        let live = watch_message(&mut socket).await;
        assert_eq!(live["type"], "live", "{live}");
    }
    let others_until = async |socket: &mut crate::files::Socket, subject: Subject| {
        crate::workflows::others_until(socket, &key(&subject)).await
    };

    // Files beside the skills directories and in a skill, then a new skill:
    // only the skills directory's names change.
    std::fs::write(app.join("notes.txt"), "notes").unwrap();
    std::fs::write(app.join(".agents/notes.txt"), "notes").unwrap();
    std::fs::create_dir_all(skills.join("release/scripts")).unwrap();
    std::fs::write(skills.join("release/scripts/run.sh"), "true").unwrap();
    std::fs::create_dir(skills.join("deploy")).unwrap();
    let names = subject(SubscriptionKind::FolderNames, &skills);
    assert_eq!(others_until(&mut socket, names).await, Vec::<String>::new());
    // A file in a skill of the group, then a `.claude/skills` made: only
    // `.claude`'s subscription changes.
    std::fs::write(skills.join("group/one/notes.md"), "notes").unwrap();
    std::fs::create_dir_all(claude.join("skills")).unwrap();
    let claude = subject(SubscriptionKind::File, &claude);
    assert_eq!(others_until(&mut socket, claude).await, Vec::<String>::new());
    // A skill edited in place.
    std::fs::write(skills.join("release/SKILL.md"), manifest("release-two")).unwrap();
    let release = subject(SubscriptionKind::File, &skills.join("release/SKILL.md"));
    assert_eq!(others_until(&mut socket, release).await, Vec::<String>::new());
    backend.close().await;
}

/// Whether any of `subjects` is in `directory`.
fn any_under(subjects: &[Subject], directory: &std::path::Path) -> bool {
    let directory = directory.to_str().unwrap();
    subjects.iter().any(|subject| subject.path.starts_with(directory))
}

// A few seconds: a real runner on a paired device answers the search and
// holds the subscriptions.
//
// Fixed bug: a conversation working in the home folder outside a project
// searched and watched `~/.claude/skills`, Claude Code's own skills.
#[tokio::test]
async fn a_conversation_in_its_home_has_no_project_skills_until_it_moves_into_a_project() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    // The home the runner reports, as the backend compares it.
    let home = std::path::PathBuf::from(alpha.runner.home());
    let app = home.join("app");
    for directory in [&home, &app] {
        let skills = directory.join(".claude/skills");
        std::fs::create_dir_all(skills.join("release")).unwrap();
        std::fs::write(
            skills.join("release/SKILL.md"),
            skill_md("name: release\ndescription: Cut a release."),
        )
        .unwrap();
    }
    let skills = home.join(".claude/skills");
    switch(&backend, &master, FIRST, &alpha, &home).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let outside = work.turn(vec![say("hello")]).await.first_request();
    assert!(!outside.contains("<name>release</name>"), "{outside}");
    assert!(!any_under(&backend.subscribed(&master, alpha.id()).await, &skills));

    // Moved to a directory in it, which is a project: its skills are
    // offered and watched, and the home's still are not.
    switch(&backend, &master, FIRST, &alpha, &app).await;
    let inside = work.turn(vec![say("moved")]).await.first_request();
    assert!(inside.contains("<name>release</name>"), "{inside}");
    let subscribed = backend.subscribed(&master, alpha.id()).await;
    assert!(any_under(&subscribed, &app.join(".claude/skills")));
    assert!(!any_under(&subscribed, &skills));
    backend.close().await;
}

// A few seconds: a real runner on a paired device answers the searches and
// holds the subscriptions.
//
// Planted defects it catches: a switch that leaves the plugin's search kept,
// and a request that keeps one for a plugin the user has off.
#[tokio::test]
async fn the_plugin_off_keeps_nothing_and_on_again_searches_at_the_next_request() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/alpha").await;
    create(&backend, &master, FIRST).await;
    let app = alpha.runner.home_dir().join("app");
    let skills = app.join(".agents/skills");
    let manifest = |name: &str| skill_md(&format!("name: {name}\ndescription: The skill {name}."));
    std::fs::create_dir_all(skills.join("release")).unwrap();
    std::fs::write(skills.join("release/SKILL.md"), manifest("release")).unwrap();
    into_project(&backend, &master, FIRST, &alpha, &app).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/alpha").await;
    let first = work.turn(vec![say("hello")]).await.first_request();
    assert!(first.contains("<name>release</name>"), "{first}");
    assert!(any_under(&backend.subscribed(&master, alpha.id()).await, &skills));

    let switch_to = async |enabled: bool| {
        let switched = backend
            .put("/api/plugins/skills", &master, json!({ "enabled": enabled }))
            .await;
        assert_eq!(switched.status, StatusCode::NO_CONTENT);
    };
    // Off: what it kept is released, and a request keeps nothing for it.
    switch_to(false).await;
    crate::support::eventually("the skills are no longer watched", || async {
        !any_under(&backend.subscribed(&master, alpha.id()).await, &skills)
    })
    .await;
    work.turn(vec![say("off")]).await;
    assert!(!any_under(&backend.subscribed(&master, alpha.id()).await, &skills));

    // On again: the next request searches, and finds the skill added while
    // nothing watched.
    std::fs::create_dir_all(skills.join("deploy")).unwrap();
    std::fs::write(skills.join("deploy/SKILL.md"), manifest("deploy")).unwrap();
    switch_to(true).await;
    let on = work.turn(vec![say("on")]).await.first_request();
    assert!(on.contains("<name>deploy</name>"), "{on}");
    backend.close().await;
}
