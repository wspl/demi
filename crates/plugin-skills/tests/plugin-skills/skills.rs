//! The skills plugin through the loopback (`skills.md` § Acceptance): its
//! sources, fetched from repositories the tests make, the Host directories
//! of the skills that are on, the project skills it reads on the Host, and
//! its catalog.

use std::time::Duration;

use demi_plugin_interface::PluginFactory;
use demi_plugin_interface::testing::loopback;
use demi_plugin_interface::{DirectoryFile, HostDirectory, PluginError, PluginId};
use demi_shared_types::{BlobRef, Timestamp};
use serde_json::json;

use demi_plugin_skills::SkillsState;
use demi_plugin_skills::testing::{FETCHED_AT, Repos, skill_md};

use crate::support::{Plugged, local};

fn tools(repos: &Repos) {
    repos.commit(
        "acme/tools",
        &[
            (
                "skills/review/SKILL.md",
                &skill_md("name: review\ndescription: Review a change."),
                false,
            ),
            (
                "skills/review/scripts/check.sh",
                "#!/bin/sh\necho ok\n",
                true,
            ),
            (
                "skills/Bad_Name/SKILL.md",
                &skill_md("name: Bad_Name\ndescription: Breaks the rule."),
                false,
            ),
            ("skills/broken/SKILL.md", &skill_md("name: broken"), false),
            (
                "skills/hidden/SKILL.md",
                &skill_md(
                    "name: hidden\ndescription: The user's alone.\ndisable-model-invocation: true",
                ),
                false,
            ),
        ],
    );
}

fn refusal(error: PluginError) -> (String, String) {
    match error {
        PluginError::Refused { reason, message } => (reason, message),
        error => panic!("{error:?}"),
    }
}

/// The path of `skill`'s `SKILL.md` on every Host, for the files it has.
fn location(name: &str, files: Vec<DirectoryFile>) -> String {
    let directory = HostDirectory {
        name: name.to_owned(),
        files,
    };
    format!(
        "{}/SKILL.md",
        directory.path(&PluginId::try_from("skills").unwrap())
    )
}

#[tokio::test]
async fn adding_a_source_lists_its_skills_with_their_warnings_and_its_skipped_files() {
    local(async {
        let repos = Repos::new();
        tools(&repos);
        let plugged = Plugged::new(&repos.skills());
        let source = plugged.add("acme/tools").await;

        let state = plugged.state().await;
        let [listed] = &state.sources[..] else {
            panic!("{state:?}")
        };
        assert_eq!(listed.id, source);
        assert_eq!(listed.origin, "acme/tools");
        assert!(listed.commit.is_some());
        assert_eq!(
            listed.fetched_at,
            Some(Timestamp::from_millisecond(FETCHED_AT).unwrap())
        );
        assert!(!listed.fetching);
        assert_eq!(listed.failure, None);
        let skills: Vec<(&str, bool)> = listed
            .skills
            .iter()
            .map(|skill| (skill.name.as_str(), skill.disable_model_invocation))
            .collect();
        assert_eq!(
            skills,
            [("Bad_Name", false), ("hidden", true), ("review", false)]
        );
        assert!(
            listed.skills[0].warnings[0].contains("not 1 to 64 lowercase"),
            "{:?}",
            listed.skills[0]
        );
        assert!(listed.skills[2].warnings.is_empty());
        let [skipped] = &listed.skipped[..] else {
            panic!("{listed:?}")
        };
        assert_eq!(skipped.path, "skills/broken/SKILL.md");
        assert!(skipped.reason.contains("no description"), "{skipped:?}");
        // The value names the files' blobs, which hold their bytes.
        let script = BlobRef::of(b"#!/bin/sh\necho ok\n");
        assert!(plugged.demi.value_blobs(&source).contains(&script));
        assert!(plugged.demi.blob_bytes(&script).is_some());

        let again = plugged
            .call(
                "add_source",
                json!({ "origin": "https://github.com/acme/tools.git/" }),
            )
            .await
            .unwrap_err();
        assert_eq!(refusal(again).0, "source_exists");
        let invalid = plugged
            .call("add_source", json!({ "origin": "not a repository" }))
            .await
            .unwrap_err();
        assert_eq!(refusal(invalid).0, "invalid_origin");
    })
    .await;
}

#[tokio::test]
async fn an_added_sources_skills_start_on_but_one_of_a_taken_name_and_one_never_offered() {
    local(async {
        let repos = Repos::new();
        tools(&repos);
        repos.commit(
            "acme/more",
            &[
                (
                    "review/SKILL.md",
                    &skill_md("name: review\ndescription: Another review."),
                    false,
                ),
                (
                    "lint/SKILL.md",
                    &skill_md("name: lint\ndescription: Lint it."),
                    false,
                ),
            ],
        );
        let plugged = Plugged::new(&repos.skills());
        plugged.add("acme/tools").await;
        plugged.add("acme/more").await;

        let state = plugged.state().await;
        let skills: Vec<(&str, &str, bool, Option<&str>)> = state
            .sources
            .iter()
            .flat_map(|source| source.skills.iter().map(move |skill| (source, skill)))
            .map(|(source, skill)| {
                (
                    source.origin.as_str(),
                    skill.name.as_str(),
                    skill.enabled,
                    skill.taken_by.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            skills,
            [
                ("acme/tools", "Bad_Name", true, None),
                ("acme/tools", "hidden", false, None),
                ("acme/tools", "review", true, None),
                ("acme/more", "lint", true, None),
                ("acme/more", "review", false, Some("acme/tools")),
            ]
        );
        let mut names: Vec<String> = plugged
            .demi
            .directories()
            .into_iter()
            .map(|directory| directory.name)
            .collect();
        names.sort();
        assert_eq!(names, ["bad-name", "lint", "review"]);
    })
    .await;
}

#[tokio::test]
async fn a_source_without_a_skill_or_over_its_bounds_shows_its_failure_and_keeps_no_skill() {
    // Over a second: the 64 MiB bound needs a repository larger than it,
    // which only a fetch of that many bytes proves stops the fetch.
    local(async {
        let repos = Repos::new();
        repos.commit("acme/empty", &[("README.md", "nothing here", false)]);
        repos.commit(
            "acme/huge",
            &[(
                "skills/big/SKILL.md",
                &skill_md("name: big\ndescription: Too much."),
                false,
            )],
        );
        repos.commit_bytes("acme/huge", "skills/big/blob.bin", &noise(65 * 1024 * 1024));
        let plugged = Plugged::new(&repos.skills());
        plugged.add("acme/empty").await;
        plugged.add("acme/huge").await;

        let state = plugged.state().await;
        let failures: Vec<(&str, bool, String)> = state
            .sources
            .iter()
            .map(|source| {
                let failure = source.failure.as_ref().expect("the fetch failed");
                assert_eq!(failure.at, Timestamp::from_millisecond(FETCHED_AT).unwrap());
                (
                    source.origin.as_str(),
                    source.skills.is_empty() && source.commit.is_none(),
                    failure.message.clone(),
                )
            })
            .collect();
        assert_eq!(
            failures,
            [
                (
                    "acme/empty",
                    true,
                    "the repository holds no skill".to_owned()
                ),
                (
                    "acme/huge",
                    true,
                    "the repository is larger than 64 MiB".to_owned()
                ),
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn a_skill_turned_on_has_a_host_directory_and_reaches_the_catalog_at_its_path() {
    local(async {
        let repos = Repos::new();
        tools(&repos);
        let plugged = Plugged::new(&repos.skills());
        // Added, `review` is on; `hidden` is never offered to the agent,
        // even turned on.
        let source = plugged.add("acme/tools").await;
        plugged.enable(&source, "hidden").await.unwrap();

        let directories = plugged.demi.directories();
        let review = directories
            .iter()
            .find(|directory| directory.name == "review")
            .expect("review has a directory");
        let mut files: Vec<(&str, bool)> = review
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.executable))
            .collect();
        files.sort();
        assert_eq!(files, [("SKILL.md", false), ("scripts/check.sh", true)]);

        // The Host is not running: the catalog lists the user skills alone.
        let block = plugged.context("/home/me/app", "t1", &[]).await.unwrap();
        let path = location("review", review.files.clone());
        assert!(
            block.contains(&format!("<location>{path}</location>")),
            "{block}"
        );
        assert!(
            block.contains("<description>Review a change.</description>"),
            "{block}"
        );
        assert!(!block.contains("hidden"), "{block}");
        // Told already, the model gets nothing new; after a compaction it is
        // told again.
        assert_eq!(plugged.context("/home/me/app", "t2", &[&block]).await, None);
        assert_eq!(
            plugged.context("/home/me/app", "t3", &[]).await,
            Some(block.clone())
        );

        // Off again, the directories go, and the model learns none is left.
        let params = json!({ "source": source, "enabled": false });
        plugged.call("set_source_enabled", params).await.unwrap();
        assert!(plugged.demi.directories().is_empty());
        assert_eq!(
            plugged
                .context("/home/me/app", "t4", &[&block])
                .await
                .as_deref(),
            Some("No skills are available now.")
        );
    })
    .await;
}

#[tokio::test]
async fn turning_on_a_second_skill_of_a_taken_name_is_refused_with_the_other_source() {
    local(async {
        let repos = Repos::new();
        tools(&repos);
        repos.commit(
            "acme/more",
            &[(
                "review/SKILL.md",
                &skill_md("name: review\ndescription: Another review."),
                false,
            )],
        );
        let plugged = Plugged::new(&repos.skills());
        let tools = plugged.add("acme/tools").await;
        let more = plugged.add("acme/more").await;
        plugged.enable(&tools, "review").await.unwrap();

        let (reason, message) = refusal(plugged.enable(&more, "review").await.unwrap_err());
        assert_eq!(reason, "skill_name_taken");
        assert!(message.contains("acme/tools"), "{message}");
        let whole = json!({ "source": more, "enabled": true });
        let (reason, _) = refusal(plugged.call("set_source_enabled", whole).await.unwrap_err());
        assert_eq!(reason, "skill_name_taken");
        let unknown = refusal(plugged.enable(&more, "nope").await.unwrap_err());
        assert_eq!(unknown.0, "skill_not_found");
    })
    .await;
}

#[tokio::test]
async fn an_update_keeps_each_skill_on_or_off_by_name_drops_those_the_new_commit_lacks_and_starts_new_ones_on()
{
    local(async {
        let repos = Repos::new();
        repos.commit(
            "acme/tools",
            &[
                (
                    "review/SKILL.md",
                    &skill_md("name: review\ndescription: Review a change."),
                    false,
                ),
                (
                    "lint/SKILL.md",
                    &skill_md("name: lint\ndescription: Lint it."),
                    false,
                ),
                (
                    "docs/SKILL.md",
                    &skill_md("name: docs\ndescription: Write the docs."),
                    false,
                ),
            ],
        );
        let plugged = Plugged::new(&repos.skills());
        let source = plugged.add("acme/tools").await;
        let params = json!({ "source": source, "skill": "lint", "enabled": false });
        plugged.call("set_enabled", params).await.unwrap();
        let first = plugged.state().await.sources[0].commit.clone();

        repos.commit(
            "acme/tools",
            &[
                (
                    "review/SKILL.md",
                    &skill_md("name: review\ndescription: Review a change, carefully."),
                    false,
                ),
                (
                    "lint/SKILL.md",
                    &skill_md("name: lint\ndescription: Lint it."),
                    false,
                ),
                (
                    "format/SKILL.md",
                    &skill_md("name: format\ndescription: Format it."),
                    false,
                ),
            ],
        );
        plugged.update(&source).await;
        let state = plugged.state().await;
        let listed = &state.sources[0];
        assert_ne!(listed.commit, first);
        let skills: Vec<(&str, bool)> = listed
            .skills
            .iter()
            .map(|skill| (skill.name.as_str(), skill.enabled))
            .collect();
        assert_eq!(
            skills,
            [("format", true), ("lint", false), ("review", true)]
        );
        let names: Vec<String> = plugged
            .demi
            .directories()
            .into_iter()
            .map(|directory| directory.name)
            .collect();
        assert_eq!(names, ["format", "review"]);

        // A failed update leaves the source as it was and shows why.
        repos.commit("acme/tools", &[("README.md", "no skills now", false)]);
        plugged.update(&source).await;
        let state = plugged.state().await;
        let after = &state.sources[0];
        assert_eq!(after.commit, listed.commit);
        assert_eq!(after.skills, listed.skills);
        let failure = after.failure.as_ref().expect("the update failed");
        assert_eq!(failure.message, "the repository holds no skill");
    })
    .await;
}

#[tokio::test]
async fn project_skills_up_to_the_repository_root_are_listed_at_their_paths_and_shadow_user_skills()
{
    local(async {
        let repos = Repos::new();
        tools(&repos);
        let plugged = Plugged::new(&repos.skills());
        let source = plugged.add("acme/tools").await;
        plugged.enable(&source, "review").await.unwrap();
        let project = |name: &str, description: &str| {
            skill_md(&format!("name: {name}\ndescription: {description}"))
        };
        let near = project("release", "Release from web.");
        let far = project("release", "Release from the root.");
        let review = project("review", "The repository's review.");
        let outside = project("outside", "Above the repository.");
        let claude = project("tests", "Write tests.");
        plugged.host(&[
            ("/home/me/app/.git/HEAD", "ref: refs/heads/main"),
            ("/home/me/app/web/.claude/skills/release/SKILL.md", &near),
            ("/home/me/app/.agents/skills/release/SKILL.md", &far),
            ("/home/me/app/.agents/skills/group/review/SKILL.md", &review),
            ("/home/me/app/.claude/skills/tests/SKILL.md", &claude),
            ("/home/me/.agents/skills/outside/SKILL.md", &outside),
        ]);

        let block = plugged
            .context("/home/me/app/web", "t1", &[])
            .await
            .unwrap();
        assert!(
            block.contains("<location>/home/me/app/web/.claude/skills/release/SKILL.md</location>"),
            "{block}"
        );
        assert!(!block.contains("Release from the root."), "{block}");
        assert!(
            block
                .contains("<location>/home/me/app/.agents/skills/group/review/SKILL.md</location>"),
            "{block}"
        );
        assert!(!block.contains("Review a change."), "{block}");
        assert!(
            block.contains("<location>/home/me/app/.claude/skills/tests/SKILL.md</location>"),
            "{block}"
        );
        assert!(!block.contains("outside"), "{block}");
        // Sorted by name.
        let release = block.find("<name>release</name>").unwrap();
        let tests = block.find("<name>tests</name>").unwrap();
        assert!(release < tests);
    })
    .await;
}

#[tokio::test]
async fn on_a_stopped_host_project_skills_appear_after_it_woke_and_a_turn_searches_once() {
    local(async {
        let repos = Repos::new();
        let plugged = Plugged::new(&repos.skills());
        let release = skill_md("name: release\ndescription: Cut a release.");

        // Stopped: no search answers, and nothing was ever told.
        assert_eq!(plugged.context("/home/me/app", "t1", &[]).await, None);
        // Woken by the turn's first command: the turn's next request finds
        // the repository's skills.
        plugged.host(&[("/home/me/app/.agents/skills/release/SKILL.md", &release)]);
        let block = plugged.context("/home/me/app", "t1", &[]).await.unwrap();
        assert!(block.contains("<name>release</name>"), "{block}");
        // The turn searched; its later requests keep what it found.
        plugged.host(&[]);
        assert_eq!(plugged.context("/home/me/app", "t1", &[&block]).await, None);
        // The next turn searches again.
        assert_eq!(
            plugged
                .context("/home/me/app", "t2", &[&block])
                .await
                .as_deref(),
            Some("No skills are available now.")
        );
    })
    .await;
}

#[tokio::test]
async fn a_catalog_over_its_budget_shortens_every_description_then_counts_the_skills_left_out() {
    local(async {
        let repos = Repos::new();
        let plugged = Plugged::new(&repos.skills());
        let long = "word ".repeat(60);
        let skills: Vec<(String, String)> = (0..30)
            .map(|index| {
                let name = format!("skill-{index:02}");
                let path = format!("/repo/.agents/skills/{name}/SKILL.md");
                (
                    path,
                    skill_md(&format!("name: {name}\ndescription: {long}")),
                )
            })
            .collect();
        let files: Vec<(&str, &str)> = skills
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str()))
            .collect();
        plugged.host(&files);
        let block = plugged.context("/repo", "t1", &[]).await.unwrap();
        assert!(block.chars().count() <= 8_000, "{}", block.chars().count());
        assert_eq!(block.matches("<skill>").count(), 30);
        assert_eq!(block.matches("…</description>").count(), 30);
        let lengths: std::collections::BTreeSet<usize> = block
            .split("<description>")
            .skip(1)
            .map(|rest| rest.split("</description>").next().unwrap().chars().count())
            .collect();
        assert_eq!(lengths.len(), 1, "one length: {lengths:?}");

        // Too many to fit even without descriptions: the last are counted.
        let name = "a".repeat(60);
        let skills: Vec<(String, String)> = (0..100)
            .map(|index| {
                let name = format!("{name}{index:02}");
                let path = format!("/repo/.agents/skills/{name}/SKILL.md");
                (
                    path,
                    skill_md(&format!("name: {name}\ndescription: Short.")),
                )
            })
            .collect();
        let files: Vec<(&str, &str)> = skills
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str()))
            .collect();
        plugged.host(&files);
        let block = plugged.context("/repo", "t2", &[]).await.unwrap();
        assert!(block.chars().count() <= 8_000, "{}", block.chars().count());
        assert!(!block.contains("<description>"), "{block}");
        let shown = block.matches("<skill>").count();
        assert!(
            block.ends_with(&format!("{} more skills are not listed.", 100 - shown)),
            "{block}"
        );
    })
    .await;
}

#[test]
fn a_shutdown_during_a_fetch_leaves_the_source_as_it_was() {
    // One blocking thread runs the fetches in the order they start, so once
    // a later source's fetch is recorded, an earlier one has ended too.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(local(async {
        let repos = Repos::new();
        tools(&repos);
        repos.commit(
            "acme/other",
            &[(
                "other/SKILL.md",
                &skill_md("name: other\ndescription: Another."),
                false,
            )],
        );
        let skills = repos.skills();
        let first = Plugged::new(&skills);
        let added = first
            .call("add_source", json!({ "origin": "acme/tools" }))
            .await
            .unwrap();
        let source = added["source"].as_str().unwrap().to_owned();
        // The shard closes with the fetch under way.
        let Plugged { plugin, demi, root } = first;
        drop(plugin);

        let second = Plugged {
            plugin: loopback(skills.instance()),
            demi: demi.clone(),
            root,
        };
        let other = second.add("acme/other").await;
        demi.until(|demi| {
            demi.value(&other)
                .is_some_and(|stored| stored.value.get("commit").is_some())
        })
        .await;
        let stored = demi.value(&source).unwrap();
        assert_eq!(stored.revision, 1, "{stored:?}");
        assert_eq!(stored.value.get("commit"), None);
        assert_eq!(stored.value.get("failure"), None);
    }));
}

#[tokio::test]
async fn opening_the_page_shows_an_update_where_the_default_branch_moved_and_nowhere_else() {
    local(async {
        let repos = Repos::new();
        tools(&repos);
        repos.commit(
            "acme/more",
            &[(
                "lint/SKILL.md",
                &skill_md("name: lint\ndescription: Lint it."),
                false,
            )],
        );
        let plugged = Plugged::new(&repos.skills());
        let tools = plugged.add("acme/tools").await;
        let more = plugged.add("acme/more").await;
        repos.commit(
            "acme/more",
            &[(
                "lint/SKILL.md",
                &skill_md("name: lint\ndescription: Lint it, strictly."),
                false,
            )],
        );
        // The fetches count as checks for five minutes.
        repos.pass(Duration::from_secs(5 * 60));

        let before = plugged.demi.changes();
        plugged.call("check_updates", json!({})).await.unwrap();
        plugged.demi.until(|demi| demi.changes() > before).await;
        let available = |state: &SkillsState| -> Vec<(String, bool)> {
            state
                .sources
                .iter()
                .map(|source| (source.id.clone(), source.update_available))
                .collect()
        };
        assert_eq!(
            available(&plugged.state().await),
            [(tools.clone(), false), (more.clone(), true)]
        );

        // Updated, the source is at the newest commit again.
        plugged.update(&more).await;
        assert_eq!(
            available(&plugged.state().await),
            [(tools, false), (more, false)]
        );
    })
    .await;
}

/// `length` bytes that do not compress.
fn noise(length: usize) -> Vec<u8> {
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}
