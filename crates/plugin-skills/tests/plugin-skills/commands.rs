//! `demi skills` through the loopback (`skills.md` § Commands), as the
//! backend's dispatch hands a command to the plugin once the conversation
//! has the grant: each acts as the page's method does and fails as it would,
//! and prints what changed. Without the grant no command reaches the plugin,
//! which the backend's permission scenarios cover.

use demi_plugin_skills::SkillsState;
use demi_plugin_skills::testing::{Repos, skill_md};

use crate::support::{Plugged, local};

/// `owner/repo` holding a skill of each name.
fn repository(repos: &Repos, name: &str, skills: &[&str]) {
    let files: Vec<(String, String)> = skills
        .iter()
        .map(|skill| {
            (
                format!("{skill}/SKILL.md"),
                skill_md(&format!("name: {skill}\ndescription: The {skill} skill.")),
            )
        })
        .collect();
    let files: Vec<(&str, &str, bool)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str(), false))
        .collect();
    repos.commit(name, &files);
}

/// Each skill of the source `origin` with whether it is on.
fn skills(state: &SkillsState, origin: &str) -> Vec<(String, bool)> {
    state
        .sources
        .iter()
        .find(|source| source.origin == origin)
        .unwrap_or_else(|| panic!("no source {origin}: {state:?}"))
        .skills
        .iter()
        .map(|skill| (skill.name.clone(), skill.enabled))
        .collect()
}

fn on(names: &[(&str, bool)]) -> Vec<(String, bool)> {
    names
        .iter()
        .map(|(name, enabled)| ((*name).to_owned(), *enabled))
        .collect()
}

#[tokio::test]
async fn add_with_skills_turns_on_exactly_those_and_a_missing_or_taken_name_fails_after_the_fetch()
{
    local(async {
        let repos = Repos::new();
        repository(&repos, "acme/tools", &["lint", "review"]);
        repository(&repos, "acme/more", &["format", "review"]);
        let plugged = Plugged::new(&repos.skills());

        let (code, out, err) = plugged
            .run(&["skills", "add", "acme/tools", "--skill", "review"])
            .await;
        assert_eq!(code, 0, "{err}");
        assert!(out.starts_with("Added acme/tools at "), "{out}");
        assert!(out.contains("Turned on: review\nLeft off: lint\n"), "{out}");
        let state = plugged.state().await;
        assert_eq!(
            skills(&state, "acme/tools"),
            on(&[("lint", false), ("review", true)])
        );

        // A name the commit lacks and a name a skill that is on has: the
        // command fails after the fetch and names each, and the source
        // stays with its skills as a new source's start.
        let (code, out, err) = plugged
            .run(&[
                "skills", "add", "acme/more", "--skill", "review", "--skill", "nope",
            ])
            .await;
        assert_eq!(code, 1, "{out}");
        assert!(
            err.contains("the commit has no skill \"nope\"")
                && err.contains("A skill named \"review\" from acme/tools is on"),
            "{err}"
        );
        let state = plugged.state().await;
        assert_eq!(
            skills(&state, "acme/more"),
            on(&[("format", true), ("review", false)])
        );

        // One already added is refused, as the page's add is.
        let (code, _, err) = plugged
            .run(&["skills", "add", "https://github.com/acme/tools.git"])
            .await;
        assert_eq!(code, 1);
        assert!(err.contains("is added already"), "{err}");
    })
    .await;
}

#[tokio::test]
async fn list_update_switch_and_remove_act_as_the_page_does_and_print_what_changed() {
    local(async {
        let repos = Repos::new();
        repository(&repos, "acme/tools", &["lint", "review"]);
        let plugged = Plugged::new(&repos.skills());
        let (code, _, err) = plugged.run(&["skills", "add", "acme/tools"]).await;
        assert_eq!(code, 0, "{err}");

        let (_, listed, _) = plugged.run(&["skills", "list"]).await;
        assert!(listed.starts_with("acme/tools, at "), "{listed}");
        assert!(
            listed.contains("  on   review — The review skill.\n"),
            "{listed}"
        );
        let (_, json, _) = plugged.run(&["skills", "list", "--json"]).await;
        let json: SkillsState = serde_json::from_str(&json).unwrap();
        assert_eq!(json, plugged.state().await);

        // At its newest commit: the same commit, and nothing changed.
        let (code, out, _) = plugged.run(&["skills", "update", "acme/tools"]).await;
        assert_eq!(code, 0);
        assert!(out.contains("is at its newest commit"), "{out}");
        assert!(out.ends_with("nothing changed.\n"), "{out}");

        // A new commit: a new skill starts on, one the commit lacks is gone.
        repository(&repos, "acme/tools", &["format", "review"]);
        let (code, out, err) = plugged.run(&["skills", "update", "acme/tools"]).await;
        assert_eq!(code, 0, "{err}");
        assert!(out.starts_with("Updated acme/tools at "), "{out}");
        assert!(
            out.contains("Turned on: format\nTurned off: lint\n"),
            "{out}"
        );

        let (_, out, _) = plugged
            .run(&["skills", "disable", "acme/tools", "--skill", "review"])
            .await;
        assert_eq!(out, "acme/tools:\nTurned off: review\n");
        let (_, out, _) = plugged.run(&["skills", "enable", "acme/tools"]).await;
        assert_eq!(out, "acme/tools:\nTurned on: review\n");
        let (code, _, err) = plugged
            .run(&["skills", "enable", "acme/tools", "--skill", "nope"])
            .await;
        assert_eq!(code, 1);
        assert!(err.contains("The source has no skill \"nope\""), "{err}");

        let (_, out, _) = plugged.run(&["skills", "remove", "acme/tools"]).await;
        assert_eq!(out, "Removed acme/tools and its skills: format, review.\n");
        let (_, listed, _) = plugged.run(&["skills", "list"]).await;
        assert_eq!(listed, "No skill sources.\n");
        let (code, _, err) = plugged.run(&["skills", "update", "acme/tools"]).await;
        assert_eq!(code, 1);
        assert!(err.contains("No skill source"), "{err}");
    })
    .await;
}
