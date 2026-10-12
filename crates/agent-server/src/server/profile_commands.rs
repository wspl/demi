//! The `demi agent profiles` group (`subagents.md` § Managing profiles):
//! the user's profiles listed, shown, created, edited and deleted through
//! the product's settings path. The changes need the user's permission, the
//! category [`PROFILES_CATEGORY`] names.

use std::rc::Weak;

use demi_agent_tools::{HostResolver, ModelChoice, ModelEdit, NewProfileDraft, ProfileEdit};
use demi_host_interface::{GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::trim;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{
    AgentServer,
    commands::{Invoked, fail, json, out, verb},
    tree::{Deleted, ProfileAnswer, ProfileListing},
};

/// The permission category the profile changes need, which the `agent`
/// group declares (`subagents.md` § Managing profiles, `permissions.md`
/// § Categories): its id, its action and what a grant allows. They change
/// a setting every conversation of the user's reads.
pub(super) const PROFILES_CATEGORY: (&str, &str, &str) = (
    "agent.profiles",
    "change subagent profiles",
    "Create, edit and delete the subagent profiles every conversation of yours can use.",
);

/// The `--model` value that makes a profile inherit the parent's model.
const PARENT: &str = "parent";

const PROFILES_GROUP: &str = "The user's subagent profiles: list them, show one, and create, edit or delete them; a change needs the user's permission and reaches every conversation.";

const LIST_SUMMARY: &str = "List the user's enabled subagent profiles with when to use each, as the user's settings say now; an unavailable one says what is missing, and a line says so when the user turned subagents off. A read, not a wait.";

const SHOW_SUMMARY: &str = "Show every field of one profile, enabled or not: name, description, model (parent's, or provider entry, model, effort and tier), spawning, enabled, and its instructions in full. A read.";

const CREATE_SUMMARY: &str = "Create a subagent profile, enabled unless --disabled. Without --model it inherits the parent's model. Its instructions, added to a child's system prompt, are stdin when given, else none. Needs the user's permission.";

const EDIT_SUMMARY: &str = "Change the fields the options name and keep the others. --model parent inherits the parent's model again; --effort or --tier alone change those of the profile's own model. Stdin replaces the instructions; --no-instructions removes them. Needs the user's permission.";

const DELETE_SUMMARY: &str = "Delete a subagent profile; a child already running on it goes on. Needs the user's permission.";

const NAME: &str = "The profile's name, the --profile value of spawn: 1 to 40 lowercase letters, digits and hyphens, starting with a letter; not default.";
const DESCRIPTION: &str = "When an agent should use the profile: one line of 1 to 500 characters, which `demi agent profiles list` shows.";
const MODEL: &str = "A model id `demi providers list` lists.";
const PROVIDER: &str = "The provider entry offering --model, by its name or id; needed only when more than one entry offers that id.";
const EFFORT: &str = "An effort the model lists, or off to turn thinking off; without it the model's default effort.";
const TIER: &str = "A service tier the model offers, such as fast, or default for the vendor's default tier.";
const INSTRUCTIONS: &str = "Text added to a child's system prompt after Demi's, which it never replaces: what this kind of subagent is for and how it works.";

/// The input of `demi agent profiles list`, which takes none.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListArgs {}

/// The input of `show` and `delete`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NameArgs {
    name: String,
}

/// The input of `demi agent profiles create`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct CreateArgs {
    name: String,
    description: String,
    model: Option<String>,
    provider: Option<String>,
    effort: Option<String>,
    tier: Option<String>,
    /// Children of this profile may not spawn children of their own.
    no_spawning: Option<bool>,
    /// Create it disabled: agents cannot use it until it is enabled.
    disabled: Option<bool>,
    instructions: Option<String>,
}

/// The input of `demi agent profiles edit`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct EditArgs {
    /// The name of the profile to change.
    profile: String,
    /// A new name for the profile.
    name: Option<String>,
    description: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    effort: Option<String>,
    tier: Option<String>,
    /// Children of this profile may spawn children of their own.
    spawning: Option<bool>,
    /// Children of this profile may not spawn children of their own.
    no_spawning: Option<bool>,
    /// Agents may use the profile.
    enabled: Option<bool>,
    /// Agents may not use the profile until it is enabled again.
    disabled: Option<bool>,
    /// Remove the profile's instructions.
    no_instructions: Option<bool>,
    instructions: Option<String>,
}

/// The `profiles` group of every node: its reads, and its changes under
/// the category the `agent` group declares.
pub(super) fn profiles_group<H: HostResolver>(server: Weak<AgentServer<H>>) -> GroupBuilder {
    let (category, _, _) = PROFILES_CATEGORY;
    GroupBuilder::new("profiles", PROFILES_GROUP)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<ListArgs>()
                .json_output::<ProfileListing>()
                .bind(TypedRpc::new(verb(server.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("show", SHOW_SUMMARY)
                .input::<NameArgs>()
                .describe("name", "The profile's name.")
                .positionals(["name"])
                .json_output::<ProfileAnswer>()
                .bind(TypedRpc::new(verb(server.clone(), show))),
        )
        .leaf(
            LeafBuilder::rpc("create", CREATE_SUMMARY)
                .input::<CreateArgs>()
                .describe("name", NAME)
                .describe("description", DESCRIPTION)
                .describe("model", format!("{MODEL} Omit to inherit the parent's model."))
                .describe("provider", PROVIDER)
                .describe("effort", EFFORT)
                .describe("tier", TIER)
                .describe("instructions", INSTRUCTIONS)
                .positionals(["name"])
                .stdin_field("instructions")
                .json_output::<ProfileAnswer>()
                .permission(category)
                .bind(TypedRpc::new(verb(server.clone(), create))),
        )
        .leaf(
            LeafBuilder::rpc("edit", EDIT_SUMMARY)
                .input::<EditArgs>()
                .describe("name", format!("A new name for the profile. {NAME}"))
                .describe("description", DESCRIPTION)
                .describe(
                    "model",
                    format!("{MODEL} parent makes the profile inherit the parent's model."),
                )
                .describe("provider", PROVIDER)
                .describe("effort", EFFORT)
                .describe("tier", TIER)
                .describe("instructions", INSTRUCTIONS)
                .positionals(["profile"])
                .stdin_field("instructions")
                .stdin_unless(["no-instructions"])
                .json_output::<ProfileAnswer>()
                .permission(category)
                .bind(TypedRpc::new(verb(server.clone(), edit))),
        )
        .leaf(
            LeafBuilder::rpc("delete", DELETE_SUMMARY)
                .input::<NameArgs>()
                .describe("name", "The profile's name.")
                .positionals(["name"])
                .json_output::<Deleted>()
                .permission(category)
                .bind(TypedRpc::new(verb(server, delete))),
        )
}

async fn list<H: HostResolver>(call: Invoked<H, ListArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let listing = match call.tree.profiles().await {
        Ok(listing) => listing,
        Err(error) => return fail(&error),
    };
    if call.json {
        return json(&port, &listing).await;
    }
    out(&port, listing.render()).await
}

async fn show<H: HostResolver>(call: Invoked<H, NameArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let shown = call.tree.profile(&call.args.name).await;
    answered(&port, call.json, shown).await
}

async fn create<H: HostResolver>(
    call: Invoked<H, CreateArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let args = call.args;
    let choice = ModelChoice {
        model: args.model,
        provider: args.provider,
        effort: args.effort,
        tier: args.tier,
    };
    let model = match choice.model.as_deref() {
        None | Some(PARENT) if names_own(&choice) => {
            return fail(
                "--provider, --effort and --tier name the profile's own model; give --model too, or omit them to inherit the parent's model",
            );
        }
        None | Some(PARENT) => None,
        Some(_) => Some(choice),
    };
    let draft = NewProfileDraft {
        name: args.name,
        description: args.description,
        model,
        instructions: given(args.instructions),
        can_spawn: args.no_spawning != Some(true),
        enabled: args.disabled != Some(true),
    };
    let created = call.tree.create_profile(draft).await;
    answered(&port, call.json, created).await
}

async fn edit<H: HostResolver>(call: Invoked<H, EditArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let args = call.args;
    let choice = ModelChoice {
        model: args.model,
        provider: args.provider,
        effort: args.effort,
        tier: args.tier,
    };
    let model = match choice.model.as_deref() {
        Some(PARENT) if names_own(&choice) => {
            return fail(
                "--provider, --effort and --tier name the profile's own model; --model parent takes the parent's",
            );
        }
        Some(PARENT) => Some(ModelEdit::Parent),
        None if choice.provider.is_some() => {
            return fail("--provider names the entry of --model; give --model too");
        }
        None if !names_own(&choice) => None,
        _ => Some(ModelEdit::Choose(choice)),
    };
    let edit = ProfileEdit {
        name: args.name,
        description: args.description,
        model,
        instructions: if args.no_instructions == Some(true) {
            Some(None)
        } else {
            given(args.instructions).map(Some)
        },
        can_spawn: match pair(args.spawning, args.no_spawning, "--spawning", "--no-spawning") {
            Ok(value) => value,
            Err(reason) => return fail(&reason),
        },
        enabled: match pair(args.enabled, args.disabled, "--enabled", "--disabled") {
            Ok(value) => value,
            Err(reason) => return fail(&reason),
        },
    };
    if edit == ProfileEdit::default() {
        return fail("nothing to change: give an option, or the instructions on stdin");
    }
    let changed = call.tree.edit_profile(&args.profile, edit).await;
    answered(&port, call.json, changed).await
}

async fn delete<H: HostResolver>(
    call: Invoked<H, NameArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    match call.tree.delete_profile(&call.args.name).await {
        Ok(deleted) if call.json => json(&port, &deleted).await,
        Ok(deleted) => out(&port, deleted.render()).await,
        Err(error) => fail(&error),
    }
}

/// Prints a profile a read or a change answered, or fails with why not.
async fn answered(
    port: &RpcPort,
    json_output: bool,
    answer: Result<ProfileAnswer, String>,
) -> Result<u8, RpcError> {
    match answer {
        Ok(answer) if json_output => json(port, &answer).await,
        Ok(answer) => out(port, answer.render()).await,
        Err(error) => fail(&error),
    }
}

/// The instructions stdin gives: none when it is empty, as it is when the
/// caller gives no stdin.
fn given(stdin: Option<String>) -> Option<String> {
    stdin
        .map(|text| trim(&text).to_owned())
        .filter(|text| !text.is_empty())
}

/// Whether the options name a provider, an effort or a tier, which only a
/// model of the profile's own has.
fn names_own(choice: &ModelChoice) -> bool {
    choice.provider.is_some() || choice.effort.is_some() || choice.tier.is_some()
}

/// A switch two flags set: on, off, or unchanged when neither is given.
fn pair(
    on: Option<bool>,
    off: Option<bool>,
    on_flag: &str,
    off_flag: &str,
) -> Result<Option<bool>, String> {
    match (on == Some(true), off == Some(true)) {
        (true, true) => Err(format!("{on_flag} and {off_flag} contradict each other; give one")),
        (true, false) => Ok(Some(true)),
        (false, true) => Ok(Some(false)),
        (false, false) => Ok(None),
    }
}
