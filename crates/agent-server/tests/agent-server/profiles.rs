//! The user's subagent settings at the tree's boundary (`subagents.md`
//! § Profiles, Acceptance 7, 14 to 17): what a spawn checks and in which
//! order, that each spawn and each `demi agent profiles` reads the settings
//! as they are then, and the provider runtime a profile's child runs on.

use std::{cell::Cell, rc::Rc};

use demi_agent_store::ClosePhase;
use demi_agent_tools::{Profile, ProfileModel, SubagentSettings};
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_provider_common::{InferenceRequest, ProviderRun, ProviderRuntime, RequestLimits};
use demi_shared_types::{AgentMessageEvent, CompletionOutcome};
use futures_util::future::LocalBoxFuture;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::subagents::{
    closed_phase, fixture, held_said, is_closed, refused, root, root_receipts, said, spawn,
    spawn_during_turn,
};
use crate::support::{
    Fixture, Gate, Model, TestProduct, agent, agent_call, conversation, is_idle, listed_model,
    open, until,
};

/// A profile the user created, enabled, with the parent's model and
/// instructions.
fn profile(name: &str, description: &str) -> Profile {
    Profile {
        name: name.into(),
        description: description.into(),
        model: None,
        instructions: None,
        can_spawn: true,
        enabled: true,
    }
}

/// Model settings of `provider`'s `model` at `effort`.
fn model_settings(provider: &str, model: &str, effort: Option<&str>) -> Option<ProfileModel> {
    Some(ProfileModel {
        provider_id: provider.into(),
        model_id: model.into(),
        thinking_effort: effort.map(str::to_owned),
        service_tier_id: None,
    })
}

/// A product whose user has subagents on with `profiles`.
fn product(profiles: Vec<Profile>) -> TestProduct {
    let product = TestProduct::default();
    product.subagents.borrow_mut().profiles = profiles;
    product
}

/// Changes the user's settings, as the settings page would between two
/// spawns.
fn change(fixture: &Fixture, edit: impl FnOnce(&mut SubagentSettings)) {
    edit(&mut fixture.product.subagents.borrow_mut());
}

/// `demi agent profiles` as text and as JSON, run by `node`.
async fn profiles(fixture: &Fixture, node: &demi_shared_types::NodeId) -> (String, Value) {
    let text = agent(&fixture.server, node, "profiles", json!({})).await;
    assert_eq!(text.code, 0, "{text:?}");
    let listed = agent_call(
        &fixture.server,
        node,
        "profiles",
        json!({}),
        true,
        CancellationToken::new(),
    )
    .await
    .expect("the call is dispatched");
    assert_eq!(listed.code, 0, "{listed:?}");
    (text.stdout, serde_json::from_str(&listed.stdout).unwrap())
}

/// A spawn by the root that fails, with what it wrote.
async fn refused_spawn(fixture: &Fixture, args: Value) -> String {
    let run = agent(&fixture.server, &root(), "spawn", args).await;
    refused(&run).to_owned()
}

#[tokio::test(flavor = "local")]
async fn a_spawn_checks_the_switch_the_name_the_profile_switch_and_the_model_in_order_and_creates_nothing()
 {
    let explore = Profile {
        model: model_settings("stub", "fast-model", Some("minimal")),
        ..profile("explore", "Finds code.")
    };
    // Disabled, and its provider entry is gone as well.
    let reviewer = Profile {
        model: model_settings("gone", "big-model", Some("high")),
        enabled: false,
        ..profile("reviewer", "Reviews a change.")
    };
    let helper = profile("helper", "Helps.");
    let product = product(vec![explore, helper, reviewer]);
    product.subagents.borrow_mut().enabled = false;
    let model = Model::default();
    let fixture = fixture(&model, product);
    fixture
        .resolver
        .list("stub", vec![listed_model("fast-model", &["low", "high"], &[])]);
    let _client = fixture.opened().await;

    // Off comes first, whatever the profile.
    let off = "demi agent spawn: subagents are turned off in settings; do this work in this session, or ask the user to turn subagents on in Settings > Subagent.\n";
    for args in [
        json!({ "prompt": "task x", "profile": "nope" }),
        json!({ "prompt": "task x" }),
    ] {
        assert_eq!(refused_spawn(&fixture, args).await, off);
    }

    change(&fixture, |settings| settings.enabled = true);
    assert_eq!(
        refused_spawn(&fixture, json!({ "prompt": "task x", "profile": "nope" })).await,
        "demi agent spawn: unknown profile \"nope\"; the user's profiles are explore, helper. Run `demi agent profiles` for when to use each, or omit --profile to inherit the parent.\n"
    );
    assert_eq!(
        refused_spawn(&fixture, json!({ "prompt": "task x", "profile": "default" })).await,
        "demi agent spawn: unknown profile \"default\"; the user's profiles are explore, helper. Run `demi agent profiles` for when to use each, or omit --profile to inherit the parent.\n"
    );
    // Disabled before unavailable: the user's choice is what the model
    // learns.
    assert_eq!(
        refused_spawn(&fixture, json!({ "prompt": "task x", "profile": "reviewer" })).await,
        "demi agent spawn: profile \"reviewer\" is disabled in settings; omit --profile to inherit the parent, or ask the user to enable it in Settings > Subagent.\n"
    );
    // Its effort is gone: no other effort and not the parent's model.
    assert_eq!(
        refused_spawn(&fixture, json!({ "prompt": "task x", "profile": "explore" })).await,
        "demi agent spawn: profile \"explore\" is unavailable: model \"fast-model\" no longer offers the effort \"minimal\"; choose another effort for the profile in settings\n"
    );
    fixture.resolver.list("stub", Vec::new());
    assert_eq!(
        refused_spawn(&fixture, json!({ "prompt": "task x", "profile": "explore" })).await,
        "demi agent spawn: profile \"explore\" is unavailable: model \"fast-model\" is no longer in its provider entry's catalog; choose another model for the profile in settings\n"
    );

    change(&fixture, |settings| {
        for profile in &mut settings.profiles {
            profile.enabled = false;
        }
    });
    assert_eq!(
        refused_spawn(&fixture, json!({ "prompt": "task x", "profile": "nope" })).await,
        "demi agent spawn: unknown profile \"nope\"; the user has no enabled profile. Omit --profile to inherit the parent.\n"
    );

    // Nothing was created, asked for a runtime or inferred.
    assert!(fixture.store.numbered(1).is_none());
    assert_eq!(fixture.resolver.calls.borrow().len(), 1);
    assert!(model.requests().is_empty());
}

#[tokio::test(flavor = "local")]
async fn each_spawn_reads_the_settings_as_they_are_and_a_child_keeps_what_it_was_spawned_with() {
    let explore = Profile {
        instructions: Some("explore prompt one".into()),
        can_spawn: false,
        ..profile("explore", "Finds code.")
    };
    let model = Model::default();
    let again = Gate::new();
    model.root([said("noted"), said("noted"), said("noted")]);
    model.child(
        "task one",
        [said("one done"), held_said(&again, "one again")],
    );
    model.child("task two", [said("two done")]);
    let fixture = fixture(&model, product(vec![explore]));
    let mut client = fixture.opened().await;

    let one = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task one", "profile": "explore" }),
    )
    .await;
    client.next_until(is_closed(&one)).await;
    change(&fixture, |settings| {
        let explore = &mut settings.profiles[0];
        explore.instructions = Some("explore prompt two".into());
        explore.can_spawn = true;
        settings.profiles.push(profile("reviewer", "Reviews a change."));
    });
    // The open tree answers the new list at once.
    let (listed, _) = profiles(&fixture, &root()).await;
    assert_eq!(
        listed,
        "explore   Finds code.\nreviewer  Reviews a change.\n"
    );
    let two = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task two", "profile": "explore" }),
    )
    .await;
    client.next_until(is_closed(&two)).await;

    let asked_one = &model.requests_of("task one")[0];
    let asked_two = &model.requests_of("task two")[0];
    // A profile's instructions replace only the identity: the harness guide
    // follows, and the index lists what the child may run.
    assert!(
        asked_one
            .system_prompt
            .starts_with("explore prompt one\n\nharness guide\n\n")
    );
    assert!(
        asked_one
            .system_prompt
            .contains("Operations: send, list, show, profiles\n")
    );
    assert!(
        asked_two
            .system_prompt
            .starts_with("explore prompt two\n\nharness guide\n\n")
    );
    assert!(
        asked_two
            .system_prompt
            .contains("Operations: spawn, send, abort, resume, list, show, profiles\n")
    );

    // Deleted after the spawn, the profile changes nothing in its child:
    // resumed, it runs as it was spawned.
    change(&fixture, |settings| settings.profiles.clear());
    until(|| fixture.store.record(&one).unwrap().delivered).await;
    let resumed = agent(
        &fixture.server,
        &root(),
        "resume",
        json!({ "id": fixture.number(&one), "message": "more" }),
    )
    .await;
    assert_eq!(resumed.code, 0, "{resumed:?}");
    let from_resumed = agent_call(
        &fixture.server,
        &one,
        "spawn",
        json!({ "prompt": "task three" }),
        false,
        CancellationToken::new(),
    )
    .await;
    again.open();
    client.next_until(is_closed(&one)).await;
    client.next_until(is_idle).await;
    let asked_again = model.requests_of("task one").pop().unwrap();
    assert!(asked_again.system_prompt.starts_with("explore prompt one\n"));
    assert!(
        matches!(&from_resumed, Ok(run) if run.code == 1 && run.stderr == "demi agent spawn: not an rpc command of this conversation\n"),
        "{from_resumed:?}"
    );
    let record = fixture.store.record(&one).unwrap();
    assert_eq!(record.profile.as_deref(), Some("explore"));
    assert_eq!(record.instructions.as_deref(), Some("explore prompt one"));
    assert!(!record.can_spawn_subagents);
    assert!(model.is_done());
}

#[tokio::test(flavor = "local")]
async fn profiles_lists_the_enabled_profiles_with_what_is_missing_and_says_when_subagents_are_off()
{
    let explore = Profile {
        model: model_settings("stub", "fast-model", Some("low")),
        ..profile("explore", "Finds code; it reports and changes nothing.")
    };
    let far = Profile {
        model: model_settings("gone", "big-model", Some("high")),
        ..profile("far-reviewer", "Reviews a finished change.")
    };
    let hidden = Profile {
        enabled: false,
        ..profile("hidden", "Never listed.")
    };
    let model = Model::default();
    model.root([said("noted")]);
    let restricted_gate = Gate::new();
    model.child("task listing", [held_said(&restricted_gate, "listed")]);
    let restricted = Profile {
        can_spawn: false,
        ..profile("restricted", "May not spawn.")
    };
    let fixture = fixture(&model, product(vec![explore, far, hidden, restricted]));
    fixture
        .resolver
        .list("stub", vec![listed_model("fast-model", &["low"], &[])]);
    let mut client = fixture.opened().await;

    let (text, listing) = profiles(&fixture, &root()).await;
    let gone = "profile \"far-reviewer\" is unavailable: its provider entry is gone or no longer available to the user; choose another model for the profile in settings";
    assert_eq!(
        text,
        format!(
            "explore       Finds code; it reports and changes nothing.\nfar-reviewer  Reviews a finished change.\n              {gone}\nrestricted    May not spawn.\n"
        )
    );
    assert_eq!(
        listing,
        json!({
            "enabled": true,
            "profiles": [
                { "name": "explore", "description": "Finds code; it reports and changes nothing.", "unavailable": null },
                { "name": "far-reviewer", "description": "Reviews a finished change.", "unavailable": gone },
                { "name": "restricted", "description": "May not spawn.", "unavailable": null },
            ],
        })
    );

    // A node that may not spawn reads them too.
    let child = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task listing", "profile": "restricted" }),
    )
    .await;
    let (from_child, _) = profiles(&fixture, &child).await;
    assert_eq!(from_child, text);
    restricted_gate.open();
    client.next_until(is_closed(&child)).await;
    client.next_until(is_idle).await;

    change(&fixture, |settings| {
        for profile in &mut settings.profiles {
            profile.enabled = false;
        }
    });
    let (none, listing) = profiles(&fixture, &root()).await;
    assert_eq!(
        none,
        "The user has no enabled subagent profile; demi agent spawn without --profile inherits the parent.\n"
    );
    assert_eq!(listing, json!({ "enabled": true, "profiles": [] }));

    change(&fixture, |settings| settings.enabled = false);
    let (off, listing) = profiles(&fixture, &root()).await;
    assert_eq!(
        off,
        "Subagents are turned off in settings; demi agent spawn fails until the user turns them on.\n"
    );
    assert_eq!(listing, json!({ "enabled": false, "profiles": [] }));
}

#[tokio::test(flavor = "local")]
async fn turning_subagents_off_stops_spawns_at_every_depth_and_leaves_running_children_alone() {
    let model = Model::default();
    let (first_gate, second_gate) = (Gate::new(), Gate::new());
    model.root([said("noted"), said("noted")]);
    model.child(
        "task outer",
        [
            held_said(&first_gate, "outer working"),
            said("outer done"),
            held_said(&second_gate, "never said"),
        ],
    );
    let fixture = fixture(&model, TestProduct::default());
    let mut client = fixture.opened().await;
    let outer = spawn(&fixture, &root(), json!({ "prompt": "task outer" })).await;
    until(|| model.requests_of("task outer").len() == 1).await;

    change(&fixture, |settings| settings.enabled = false);
    let off = "subagents are turned off in settings; do this work in this session, or ask the user to turn subagents on in Settings > Subagent.";
    let nested = agent(
        &fixture.server,
        &outer,
        "spawn",
        json!({ "prompt": "task inner" }),
    )
    .await;
    assert_eq!(refused(&nested), format!("demi agent spawn: {off}\n"));
    let sent = agent(
        &fixture.server,
        &root(),
        "send",
        json!({ "id": fixture.number(&outer).to_string(), "message": "keep going" }),
    )
    .await;
    assert_eq!(sent.code, 0, "{sent:?}");

    // The running child goes on, reads the message and completes; a resume
    // and an abort still reach it.
    first_gate.open();
    client.next_until(is_closed(&outer)).await;
    until(|| fixture.store.record(&outer).unwrap().delivered).await;
    let resumed = agent(
        &fixture.server,
        &root(),
        "resume",
        json!({ "id": fixture.number(&outer), "message": "again" }),
    )
    .await;
    assert_eq!(resumed.code, 0, "{resumed:?}");
    until(|| model.requests_of("task outer").len() == 3).await;
    let aborted = agent(
        &fixture.server,
        &root(),
        "abort",
        json!({ "id": [fixture.number(&outer)] }),
    )
    .await;
    assert_eq!(aborted.code, 0, "{aborted:?}");
    client.next_until(is_idle).await;
    assert_eq!(closed_phase(&fixture, &outer), Some(ClosePhase::Aborted));
    assert!(fixture.store.numbered(2).is_none());
}

/// A provider runtime of another entry: it plays `model` and counts how
/// often a runtime of it, or a fork, is closed.
#[derive(Clone)]
struct Counted {
    model: Model,
    closed: Rc<Cell<usize>>,
}

impl ProviderRuntime for Counted {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        self.model.run(request)
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(self.clone())
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        self.closed.set(self.closed.get() + 1);
        Box::pin(async {})
    }

    fn request_limits(&self, _model: &demi_shared_types::Model) -> RequestLimits {
        RequestLimits::default()
    }
}

#[tokio::test(flavor = "local")]
async fn a_profile_of_another_entry_runs_its_child_on_a_runtime_of_its_own_closed_with_the_child()
{
    let far = Profile {
        model: model_settings("other", "other-model", None),
        ..profile("far", "Runs elsewhere.")
    };
    let near = Profile {
        model: model_settings("stub", "near-model", None),
        ..profile("near", "Runs on the parent's entry.")
    };
    let model = Model::default();
    model.root([said("noted"), said("noted"), said("noted")]);
    model.child("task near", [said("near done")]);
    let other = Model::default();
    other.child("task far", [said("far done")]);
    let held_gate = Gate::new();
    other.child("task held", [held_said(&held_gate, "never said")]);
    let closed = Rc::new(Cell::new(0));
    let fixture = fixture(&model, product(vec![far, near]));
    fixture.resolver.provide_runtime(
        "other",
        Counted {
            model: other.clone(),
            closed: closed.clone(),
        },
    );
    fixture
        .resolver
        .list("stub", vec![listed_model("near-model", &["low", "high"], &[])]);
    fixture
        .resolver
        .list("other", vec![listed_model("other-model", &["medium"], &[])]);
    let mut client = fixture.opened().await;

    let far_child = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task far", "profile": "far" }),
    )
    .await;
    client.next_until(is_closed(&far_child)).await;
    let near_child = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task near", "profile": "near" }),
    )
    .await;
    client.next_until(is_closed(&near_child)).await;

    // The far child inferred on a runtime built for its entry, at the
    // effort its profile was given, and closing it closed that runtime.
    let asked_far = &other.requests_of("task far")[0];
    assert_eq!(asked_far.model_id, "other-model");
    assert_eq!(closed.get(), 1);
    // The near child forked the root's runtime: no runtime was built for it.
    assert_eq!(model.requests_of("task near")[0].model_id, "near-model");
    let built: Vec<String> = fixture
        .resolver
        .calls
        .borrow()
        .iter()
        .map(|(_, provider)| provider.clone())
        .collect();
    assert_eq!(built, ["stub", "other"]);

    // A live far child's runtime is closed when its tree is disposed.
    let held = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task held", "profile": "far" }),
    )
    .await;
    until(|| other.requests_of("task held").len() == 1).await;
    client.next_until(is_idle).await;
    client.send(ClientFrame::Close {}).await;
    client
        .next_until(|frame| *frame == ServerFrame::Closed)
        .await;
    assert_eq!(closed.get(), 2);
    assert!(fixture.store.record(&held).unwrap().closed.is_none());
}

#[tokio::test(flavor = "local")]
async fn a_restored_child_whose_entry_is_gone_closes_as_an_error_with_its_subtree() {
    let far = Profile {
        model: model_settings("other", "other-model", None),
        ..profile("far", "Runs elsewhere.")
    };
    let model = Model::default();
    model.root([said("noted")]);
    let other = Model::default();
    let (outer_gate, never) = (Gate::new(), Gate::new());
    other.child(
        "task outer",
        [held_said(&outer_gate, "delegated"), held_said(&never, "never said")],
    );
    other.child("task inner", [held_said(&never, "never said")]);
    let fixture = fixture(&model, product(vec![far]));
    fixture.resolver.provide_runtime("other", other.clone());
    fixture
        .resolver
        .list("other", vec![listed_model("other-model", &[], &[])]);
    let mut client = fixture.opened().await;
    let outer = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task outer", "profile": "far" }),
    )
    .await;
    let inner_start = spawn_during_turn(&fixture, &outer, json!({ "prompt": "task inner" }));
    outer_gate.open();
    let inner = inner_start.await.unwrap();
    until(|| other.requests_of("task inner").len() == 1).await;
    client.send(ClientFrame::Close {}).await;
    client
        .next_until(|frame| *frame == ServerFrame::Closed)
        .await;

    // The entry goes while the tree is disposed; reopened, the child it
    // ran on closes as an error, its own child as aborted, and the root
    // receives the failed completion.
    fixture.resolver.remove("other");
    let mut reopened = fixture.client();
    reopened.send(open()).await;
    reopened.next_until(is_closed(&outer)).await;
    reopened.next_until(is_idle).await;

    assert_eq!(
        closed_phase(&fixture, &outer),
        Some(ClosePhase::Error {
            failure: "Provider \"other\" is not available".into()
        })
    );
    assert_eq!(closed_phase(&fixture, &inner), Some(ClosePhase::Aborted));
    let receipts = root_receipts(&fixture);
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].sender.clone().unwrap().id, outer);
    assert_eq!(
        receipts[0].event,
        AgentMessageEvent::Completion {
            outcome: CompletionOutcome::Failed
        }
    );
    assert!(fixture.store.record(&outer).unwrap().delivered);
    assert_eq!(
        fixture
            .server
            .tree(&conversation())
            .map(|tree| tree.is_quiescent()),
        Some(true)
    );
}
