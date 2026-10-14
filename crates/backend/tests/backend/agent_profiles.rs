//! An agent's view and changes of the user's subagent profiles
//! (`subagents.md` § Managing profiles): `demi providers list` lists the
//! models the user has on, an entry or a model the user turned off left
//! out unless `--all`, and marks the model the agent runs on; a profile change asks
//! the user's permission first, then goes through the settings page's
//! checks with their words and reaches the settings page at once; and the
//! next spawn with a profile just created runs on its model. The model is
//! the scripted family of the subagent scenarios; the device is a real
//! runner. No test calls a real model.

use std::sync::Arc;

use demi_conversation_socket_protocol::{ServerFrame, SubagentEvent};
use demi_web_api_protocol::conversations::ModelSettings;
use demi_web_api_protocol::ids::ProviderId;
use demi_web_api_protocol::providers::ProviderAnswer;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, Socket};
use crate::permissions::{decide, permissions};
use crate::settings::synced;
use crate::subagents::{Scripts, WAIT, say, shell, tree_on};
use crate::support::Harness;

/// What the root of `FIRST` was asked last.
pub(crate) fn last_root(scripts: &Scripts) -> String {
    scripts
        .asked(|session| session == FIRST)
        .pop()
        .expect("the root was asked")
}

/// Whether a request's items, which a request shows as Rust's debug text,
/// carry the command output `text`.
fn shows(asked: &str, text: &str) -> bool {
    let escaped = format!("{text:?}");
    asked.contains(&escaped[1..escaped.len() - 1])
}

#[track_caller]
pub(crate) fn assert_shows(asked: &str, text: &str) {
    assert!(shows(asked, text), "{text:?} is not in {asked}");
}

// About a second: a real device runs the shell jobs of five turns, one of
// them woken by the user's decision, and a child's.
#[tokio::test]
async fn an_agent_lists_the_menus_models_and_changes_profiles_under_the_permission_as_the_settings_page_does()
 {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _device, root, provider) =
        tree_on(&scripts, Harness::new()).await;
    // A second entry offers `n` too; the user turned it off, so the menu
    // leaves it out and `--model n` names the first entry's.
    let spare = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({
                "source": "custom", "providerType": "tree", "label": "Spare", "apiKey": "k",
                "models": [{
                    "id": "n", "displayName": "N", "contextWindow": 100000, "outputLimit": null,
                    "thinkingEfforts": ["low", "high"], "acceptedExtensions": null, "fastTier": null
                }]
            }),
        )
        .await;
    assert_eq!(spare.status, StatusCode::CREATED);
    let spare = spare.json::<ProviderAnswer>().provider.id;
    let hidden = backend
        .patch(
            "/api/settings/preferences",
            &master,
            json!({ "hiddenProviders": [spare] }),
        )
        .await;
    assert_eq!(hidden.status, StatusCode::OK);
    let mut page = backend.sync(&master).await;
    page.snapshot().await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The menu, and a change the conversation has no permission for.
    let create = "demi agent profiles create explore --description 'Finds code.' --model n --effort high";
    scripts.root(
        FIRST,
        vec![
            shell("t1", &format!("demi providers list; {create} <<< 'You only report.'")),
            say("asked"),
        ],
    );
    socket.chat("m1", "Make an explore profile on n").await;
    let asked = last_root(&scripts);
    assert_shows(
        &asked,
        "Tree · API key · Ready\n* m  M  100K                 -\n  n  N  100K  low* high off  -\n* the model this agent runs on; an effort with * is the model's default\n",
    );
    assert!(!asked.contains("Spare"), "{asked}");
    assert_shows(
        &asked,
        "demi agent profiles create: this conversation needs the user's permission to change subagent profiles; the request was sent to the user, and you will be told when the user decides",
    );
    let requested = permissions(&backend, &master, FIRST).await;
    assert_eq!(requested.requests.len(), 1, "{requested:?}");
    let request = &requested.requests[0];
    assert_eq!(request.command(), create);
    assert_eq!(crate::permissions::categories(request).len(), 1, "{request:?}");
    let category = &crate::permissions::categories(request)[0];
    assert_eq!(category.id, "agent.profiles");
    assert_eq!(category.action.as_deref(), Some("change subagent profiles"));
    assert_eq!(
        category.description.as_deref(),
        Some("Create, edit and delete the subagent profiles every conversation of yours can use.")
    );

    // Allowed, the agent creates it; what the settings page refuses, the
    // command refuses with the page's words.
    let long = "x".repeat(501);
    scripts.root(
        FIRST,
        vec![
            shell(
                "t2",
                &[
                    format!("{create} <<< 'You only report.'"),
                    "demi agent profiles create Explore --description x </dev/null".to_owned(),
                    "demi agent profiles create default --description x </dev/null".to_owned(),
                    "demi agent profiles create explore --description x </dev/null".to_owned(),
                    format!("demi agent profiles create e1 --description {long} </dev/null"),
                    "demi agent profiles create e2 --description x --model n --effort max </dev/null".to_owned(),
                    "demi agent profiles create e3 --description x --model zzz --provider Tree </dev/null".to_owned(),
                    "demi agent profiles create e4 --description x --model n --provider Spare </dev/null".to_owned(),
                    "demi agent profiles create e5 --description x --context 500K </dev/null".to_owned(),
                    "demi agent profiles create e6 --description x --model n --context full </dev/null".to_owned(),
                    "demi agent profiles create e7 --description x --model n --context 400K </dev/null".to_owned(),
                ]
                .join("\n"),
            ),
            say("created"),
        ],
    );
    let allowed = decide(&backend, &master, FIRST, request.id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    socket.until_idle().await;
    let asked = last_root(&scripts);
    for refusal in [
        "demi agent profiles create: name: must be 1 to 40 lowercase letters, digits and hyphens, starting with a letter",
        "demi agent profiles create: name: \"default\" is reserved for inheriting the parent",
        "demi agent profiles create: Another of your subagent profiles has that name",
        "demi agent profiles create: description: must be one line of 1 to 500 characters",
        "demi agent profiles create: The model does not offer the thinking effort \"max\"",
        "demi agent profiles create: The provider does not list that model",
        "demi agent profiles create: the provider entry \"Spare\" is turned off in Settings > Providers",
        "demi agent profiles create: context: the profile inherits the parent's model, which follows the user's context limit; give the profile a model of its own first",
        "demi agent profiles create: context: model \"n\" has a window of 100K and offers no choice of context; use default",
        "\"400K\" is not a context: use full, 500K, 300K, 200K or default",
    ] {
        assert_shows(&asked, refusal);
    }
    let provider_id = ProviderId::try_from(provider.as_str()).unwrap();
    let settings = synced(&mut page).await;
    assert_eq!(settings.profiles.len(), 1, "{settings:?}");
    let explore = &settings.profiles[0];
    assert_eq!(
        (
            explore.name.as_str(),
            explore.description.as_str(),
            explore.instructions.as_deref(),
            explore.can_spawn,
            explore.enabled
        ),
        ("explore", "Finds code.", Some("You only report."), true, true)
    );
    assert_eq!(
        explore.model,
        Some(ModelSettings {
            provider_id: provider_id.clone(),
            model_id: "n".into(),
            thinking_effort: Some("high".into()),
            service_tier_id: None,
        })
    );

    // An edit reaches the page, and a spawn with the profile runs on its
    // model. The child waits until the root's turn has ended, so that its
    // completion wakes the idle root.
    scripts.child(vec![shell("c1", WAIT), say("found it")]);
    scripts.root(
        FIRST,
        vec![
            shell(
                "t3",
                "demi agent profiles edit explore --description 'Finds code; it changes nothing.' --no-spawning </dev/null && demi agent profiles show explore && demi agent spawn --profile explore <<< 'Find the parser'",
            ),
            say("dispatched"),
            say("received"),
        ],
    );
    socket.chat("m2", "Use it").await;
    std::fs::write(format!("{root}/go"), "").unwrap();
    socket
        .until(|frame| {
            matches!(
                frame,
                ServerFrame::Subagent {
                    event: SubagentEvent::Closed,
                    ..
                }
            )
        })
        .await;
    socket.until_idle().await;
    let edited = synced(&mut page).await;
    assert_eq!(
        (edited.profiles[0].description.as_str(), edited.profiles[0].can_spawn),
        ("Finds code; it changes nothing.", false)
    );
    let shown = &scripts.asked(|session| session == FIRST)[5];
    assert_shows(
        shown,
        "name          explore\ndescription   Finds code; it changes nothing.\nmodel         n from Tree, effort high, tier default\ncontext       default\nspawning      no\nenabled       yes\ninstructions\nYou only report.\n",
    );
    let child = scripts
        .asked(|session| session != FIRST)
        .pop()
        .expect("the child was asked");
    assert!(child.contains("model=n "), "{child}");
    assert!(child.contains("effort: \"high\""), "{child}");

    // Deleted, it leaves the page and the list.
    scripts.root(
        FIRST,
        vec![
            shell("t4", "demi agent profiles delete explore && demi agent profiles list"),
            say("deleted"),
        ],
    );
    socket.chat("m3", "Delete it").await;
    let asked = last_root(&scripts);
    assert_shows(
        &asked,
        "deleted profile explore\nThe user has no enabled subagent profile; demi agent spawn without --profile inherits the parent.",
    );
    assert!(synced(&mut page).await.profiles.is_empty());

    // A model the user turns off leaves the menu, and no profile takes it.
    let hidden = backend
        .patch(
            "/api/settings/preferences",
            &master,
            json!({ "hiddenModels": [{ "providerId": provider, "modelId": "n" }] }),
        )
        .await;
    assert_eq!(hidden.status, StatusCode::OK);
    scripts.root(
        FIRST,
        vec![
            shell(
                "t5",
                "demi providers list; demi providers list --all; demi agent profiles create e5 --description x --model n </dev/null",
            ),
            say("refused"),
        ],
    );
    socket.chat("m4", "Use n").await;
    let asked = last_root(&scripts);
    assert_shows(
        &asked,
        "Tree · API key · Ready\n* m  M  100K  -\n* the model this agent runs on; an effort with * is the model's default\nTree · API key · Ready\n* m  M  100K                 -\n  n  N  100K  low* high off  -  turned off\nSpare · API key · Ready · turned off\n  n  N  100K  low* high off  -\n* the model this agent runs on; an effort with * is the model's default\ndemi agent profiles create: the model \"n\" of \"Tree\" is turned off in Settings > Providers",
    );
    backend.close().await;
}
