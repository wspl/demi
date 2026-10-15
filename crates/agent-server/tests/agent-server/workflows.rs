//! A conversation's workflow at the tree's boundary (`subagents.md`
//! § Workflows of a project): its profiles replace the user's at each
//! spawn and in `demi agent profiles list`, whatever the user's switch is;
//! No Subagents and an unavailable workflow refuse every spawn with their
//! own message; `demi agent workflows` lists the project's workflows and
//! checks a file with the one parser.
// Cost: in-memory store and scripted model; a few milliseconds each.

use demi_agent_store::testing::model_of;
use demi_agent_tools::{
    ChosenWorkflow, ConversationProfiles, ListedWorkflow, Profile, WorkflowChild, WorkflowListing,
};
use demi_shared_types::WorkflowChoice;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::subagents::{fixture, is_closed, refused, root, said, spawn};
use crate::support::{Fixture, Model, TestProduct, agent, agent_call};

/// The workflow Review Team: `explore`, whose model the user's entries
/// offer, with a Context of 300K, and `slow`, whose model none of them
/// does.
fn review_team() -> ConversationProfiles {
    ConversationProfiles::Workflow(ChosenWorkflow {
        id: "review".into(),
        name: "Review Team".into(),
        profiles: vec![
            WorkflowChild {
                context_limit: Some(300_000),
                name: "explore".into(),
                description: "Finds code.".into(),
                instructions: Some("Read and report.".into()),
                can_spawn: false,
                model: Some(Ok(model_of("stub", "fast-model"))),
            },
            WorkflowChild {
                context_limit: None,
                name: "slow".into(),
                description: "Thinks long.".into(),
                instructions: None,
                can_spawn: false,
                model: Some(Err(
                    "no provider entry of the user's offers model \"deep-model\"; change the profile's model in /p/.demi/workflows/review.yaml, or ask the user to add a provider that offers it".into(),
                )),
            },
        ],
    })
}

/// A product whose user has the profile `helper` and subagents off, so
/// only a workflow lets a spawn through.
fn product() -> TestProduct {
    let product = TestProduct::default();
    {
        let mut settings = product.subagents.borrow_mut();
        settings.enabled = false;
        settings.profiles = vec![Profile {
            context_limit: None,
            name: "helper".into(),
            description: "Helps.".into(),
            model: None,
            instructions: None,
            can_spawn: true,
            enabled: true,
        }];
    }
    product
}

/// `demi agent <verb>` by the root as text and as JSON.
async fn both(fixture: &Fixture, verb: &str, args: Value) -> (String, Value) {
    let text = agent(&fixture.server, &root(), verb, args.clone()).await;
    assert_eq!(text.code, 0, "{text:?}");
    let listed = agent_call(&fixture.server, &root(), verb, args, true, CancellationToken::new())
        .await
        .expect("the call is dispatched");
    assert_eq!(listed.code, 0, "{listed:?}");
    (text.stdout, serde_json::from_str(&listed.stdout).unwrap())
}

async fn refused_spawn(fixture: &Fixture, args: Value) -> String {
    refused(&agent(&fixture.server, &root(), "spawn", args).await).to_owned()
}

#[tokio::test(flavor = "local")]
async fn a_chosen_workflows_profiles_replace_the_users_and_one_whose_model_no_entry_offers_is_unavailable() {
    let model = Model::default();
    model.root([said("noted")]);
    model.child("task explore", [said("explored")]);
    let fixture = fixture(&model, product());
    *fixture.product.workflow.borrow_mut() = review_team();
    let mut client = fixture.opened().await;

    let (text, listing) = both(&fixture, "profiles list", json!({})).await;
    assert_eq!(
        text,
        "Workflow Review Team:\nexplore  Finds code.\nslow     Thinks long.\n         profile \"slow\" is unavailable: no provider entry of the user's offers model \"deep-model\"; change the profile's model in /p/.demi/workflows/review.yaml, or ask the user to add a provider that offers it\n"
    );
    assert_eq!(listing["enabled"], json!(true));
    assert_eq!(listing["choice"], json!({ "type": "workflow", "id": "review" }));
    assert_eq!(listing["workflow"], json!("Review Team"));

    // The user's profiles are not this conversation's.
    assert_eq!(
        refused_spawn(&fixture, json!({ "prompt": "task x", "profile": "helper" })).await,
        "demi agent spawn: unknown profile \"helper\"; the profiles of this conversation's workflow Review Team are explore, slow. Run `demi agent profiles list` for when to use each, or omit --profile to inherit the parent.\n"
    );
    assert_eq!(
        refused_spawn(&fixture, json!({ "prompt": "task x", "profile": "slow" })).await,
        "demi agent spawn: profile \"slow\" is unavailable: no provider entry of the user's offers model \"deep-model\"; change the profile's model in /p/.demi/workflows/review.yaml, or ask the user to add a provider that offers it\n"
    );
    assert!(fixture.store.numbered(1).is_none());

    // With the user's switch off, the workflow's profile spawns, on its
    // model and with its instructions.
    let child = spawn(
        &fixture,
        &root(),
        json!({ "prompt": "task explore", "profile": "explore" }),
    )
    .await;
    client.next_until(is_closed(&child)).await;
    let asked = &model.requests_of("task explore")[0];
    assert_eq!(asked.model_id, "fast-model");
    // And with its Context.
    assert_eq!(fixture.store.record(&child).unwrap().context_limit, Some(300_000));
    assert!(
        asked.system_prompt.ends_with("Instructions for this subagent:\n\nRead and report."),
        "{}",
        asked.system_prompt
    );
}

#[tokio::test(flavor = "local")]
async fn no_subagents_and_an_unavailable_workflow_refuse_every_spawn_with_their_own_words() {
    let model = Model::default();
    let fixture = fixture(&model, TestProduct::default());
    *fixture.product.workflow.borrow_mut() = ConversationProfiles::Off;
    let _client = fixture.opened().await;

    let off = "demi agent spawn: subagents are turned off for this conversation; do this work in this session, or ask the user to choose Default or a workflow in the composer's Workflow menu.\n";
    for args in [json!({ "prompt": "task x" }), json!({ "prompt": "task x", "profile": "explore" })] {
        assert_eq!(refused_spawn(&fixture, args).await, off);
    }
    let (text, listing) = both(&fixture, "profiles list", json!({})).await;
    assert_eq!(
        text,
        "Subagents are turned off for this conversation; demi agent spawn fails until the user chooses Default or a workflow in the composer's Workflow menu.\n"
    );
    assert_eq!(listing["enabled"], json!(false));
    assert_eq!(listing["choice"], json!({ "type": "none" }));

    *fixture.product.workflow.borrow_mut() = ConversationProfiles::Unavailable {
        id: "review".into(),
        reason: "it is no longer in the project's .demi/workflows/".into(),
    };
    assert_eq!(
        refused_spawn(&fixture, json!({ "prompt": "task x" })).await,
        "demi agent spawn: this conversation's workflow \"review\" is unavailable: it is no longer in the project's .demi/workflows/; do this work in this session, or ask the user to choose another in the composer's Workflow menu.\n"
    );
    let (text, _) = both(&fixture, "profiles list", json!({})).await;
    assert_eq!(
        text,
        "This conversation's workflow \"review\" is unavailable: it is no longer in the project's .demi/workflows/; demi agent spawn fails until the user chooses another in the composer's Workflow menu.\n"
    );
    assert!(fixture.store.numbered(1).is_none());
    assert!(model.requests().is_empty());
}

#[tokio::test(flavor = "local")]
async fn workflows_list_marks_the_choice_and_check_names_the_line_and_field_that_are_wrong() {
    let model = Model::default();
    let fixture = fixture(&model, TestProduct::default());
    *fixture.product.workflows.borrow_mut() = Some(WorkflowListing {
        chosen: WorkflowChoice::Workflow { id: "review".into() },
        directory: Some("/p/.demi/workflows".into()),
        unavailable: None,
        workflows: vec![
            ListedWorkflow { id: "docs".into(), name: Some("Docs Review".into()), path: "/p/.demi/workflows/docs.yaml".into(), unavailable: None },
            ListedWorkflow { id: "review".into(), name: Some("Review Team".into()), path: "/p/.demi/workflows/review.yaml".into(), unavailable: None },
        ],
    });
    fixture.product.files.borrow_mut().extend([
        (
            "/workspace/.demi/workflows/docs.yaml".to_owned(),
            "name: Docs Review\nprofiles:\n  - id: reader\n    description: Reads the docs.\n".to_owned(),
        ),
        (
            "/workspace/.demi/workflows/port.yaml".to_owned(),
            "name: \"wasm32 port: lead + worker\"\nprofiles: []\n".to_owned(),
        ),
        (
            "/workspace/.demi/workflows/bad.yaml".to_owned(),
            "name: Bad\nprofiles:\n  - id: Reader\n    description: Reads the docs.\n".to_owned(),
        ),
    ]);
    let _client = fixture.opened().await;

    let (text, _) = both(&fixture, "workflows list", json!({})).await;
    assert_eq!(
        text,
        "Workflows in /p/.demi/workflows; this conversation uses the workflow Review Team (review).\n  docs    Docs Review\n* review  Review Team\n"
    );

    let good = agent(&fixture.server, &root(), "workflows check", json!({ "file": ".demi/workflows/docs.yaml" })).await;
    assert_eq!(good.code, 0, "{good:?}");
    assert_eq!(good.stdout, ".demi/workflows/docs.yaml reads: Docs Review, profiles reader.\n");
    let task_title = agent(&fixture.server, &root(), "workflows check", json!({ "file": ".demi/workflows/port.yaml" })).await;
    assert_eq!(task_title.code, 0, "{task_title:?}");
    assert_eq!(
        task_title.stdout,
        ".demi/workflows/port.yaml reads: wasm32 port: lead + worker, no profiles.\nWarning: the name wasm32 port: lead + worker has a colon, a plus sign or brackets; the Workflow menu shows it as a menu item, the way of working in a few words, such as Review Team or Parallel Development.\n"
    );
    let bad = agent(&fixture.server, &root(), "workflows check", json!({ "file": ".demi/workflows/bad.yaml" })).await;
    assert!(
        refused(&bad).starts_with(
            "demi agent workflows check: .demi/workflows/bad.yaml does not read: error: line 3 column 9: validation error: is not 1 to 40 lowercase letters, digits and hyphens starting with a letter for `profiles[0].id`\n"
        ),
        "{bad:?}"
    );
    let missing = agent(&fixture.server, &root(), "workflows check", json!({ "file": "nope.yaml" })).await;
    assert_eq!(refused(&missing), "demi agent workflows check: no file nope.yaml\n");
}
