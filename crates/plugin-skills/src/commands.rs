//! `demi skills` (`skills.md` § Commands): the agent's only way to manage
//! the user's skills, acting as the page's methods do and failing as they
//! would. Every command but `list` names the category Manage skills, so the
//! backend's dispatch runs it only in a conversation the user allowed it for
//! (`permissions.md`); the plugin never sees a permission.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use demi_host_interface::{GroupBuilder, LeafBuilder, RpcInvocation};
use demi_plugin_interface::{
    CommandPlugin, Placement, PluginError, PluginPort, PortHandled,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::origin::Origin;
use crate::sources::{self, Source, SourceState};
use crate::{Fetched, SkillsState, State};

/// The group's category: what the user allows a conversation.
const MANAGE: &str = "skills.manage";

const SUMMARY: &str = "The user's skills, which every conversation of the user receives: list, add, update, remove, enable, disable. Install and manage skills only with these commands, never with other tools such as `npx skills add`: Demi does not see a skill another tool installs, and it would reach no other host and no other conversation. Name a source by its repository, owner/repo for GitHub or an https URL; any spelling of one repository names the same source.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Manages the user's skills, packaged instructions and scripts that extend what you can do in every conversation of the user: list, add from a source, update, remove, turn on or off. Use it when the user asks to install or change a skill, and install skills only this way, never with npx or by copying files, so the user sees and controls them.";

const MANAGE_DESCRIPTION: &str = "Manage skills lets the agents of this conversation add, update and remove skill sources and turn skills on or off. Your skills reach every conversation, and the skills that are on are installed on every Host your conversations use.";

const REFUSED_OUTPUT: &str =
    "writes the reason to stderr and exits non-zero, having changed nothing the reason does not name";

/// The input of `demi skills list`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListArgs {}

/// The input of `demi skills add`, `enable` and `disable`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SkillsArgs {
    /// owner/repo for GitHub, or an https URL of a git repository
    repository: String,
    /// A skill of the source, by name; every skill when none is given
    skill: Option<Vec<String>>,
}

/// The input of `demi skills update` and `remove`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RepositoryArgs {
    /// owner/repo for GitHub, or an https URL of a git repository
    repository: String,
}

/// The `skills` group, whose leaves the plugin answers with its port.
pub(crate) fn commands() -> CommandPlugin {
    let group = GroupBuilder::new("skills", SUMMARY)
        .index_entry(ENTRY)
        .permission(MANAGE, "manage skills", MANAGE_DESCRIPTION)
        .leaf(
            LeafBuilder::rpc("list", "Every skill source of the user with its skills, whether each is on, and whether an update is available.")
                .input::<ListArgs>()
                .json_output::<SkillsState>()
                .success_output("each source with its commit, failure or available update, then its skills, or JSON matching { sources } when --json is passed")
                .failure_output("never fails on live data")
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("add", "Add a skill source and wait for its first fetch; its skills start on, or only those --skill names.")
                .input::<SkillsArgs>()
                .positionals(["repository"])
                .describe("skill", "A skill of the source to turn on; the others stay off")
                .permission(MANAGE)
                .success_output("the commit pinned and the skills turned on and left off")
                .failure_output(format!("a repository that is not owner/repo or an https URL, one added already, a failed fetch (the source stays, showing its failure), or a --skill the commit lacks or whose name a skill that is on has (the source stays, its skills as a new source's start); {REFUSED_OUTPUT}"))
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("update", "Fetch a source again, pin its newest commit, and wait for the fetch.")
                .input::<RepositoryArgs>()
                .positionals(["repository"])
                .permission(MANAGE)
                .success_output("the commit pinned and the skills turned on or off, or that nothing changed")
                .failure_output(format!("a repository the user has not added, or a failed fetch; {REFUSED_OUTPUT}"))
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("remove", "Remove a source and its skills.")
                .input::<RepositoryArgs>()
                .positionals(["repository"])
                .permission(MANAGE)
                .success_output("the source and the skills removed")
                .failure_output(format!("a repository the user has not added; {REFUSED_OUTPUT}"))
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("enable", "Turn on the named skills of a source, or every skill of it.")
                .input::<SkillsArgs>()
                .positionals(["repository"])
                .describe("skill", "A skill of the source to turn on; every skill when none is given")
                .permission(MANAGE)
                .success_output("the skills turned on")
                .failure_output(format!("a repository or a skill the user does not have, or a skill whose name a skill that is on has, naming that skill's source; {REFUSED_OUTPUT}"))
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("disable", "Turn off the named skills of a source, or every skill of it.")
                .input::<SkillsArgs>()
                .positionals(["repository"])
                .describe("skill", "A skill of the source to turn off; every skill when none is given")
                .permission(MANAGE)
                .success_output("the skills turned off")
                .failure_output(format!("a repository or a skill the user does not have; {REFUSED_OUTPUT}"))
                .bind(PortHandled),
        );
    CommandPlugin::new(Placement::Demi, vec![group])
        .expect("the skills group is a valid declaration")
}

/// What a leaf prints: its output, or why it refused.
type Printed = Result<String, String>;

/// Runs the leaf `invocation` names, which [`CommandPlugin::check`]
/// checked, and answers its exit status.
pub(crate) async fn run(
    state: &Rc<State>,
    invocation: RpcInvocation,
    port: &PluginPort,
) -> Result<u8, PluginError> {
    let leaf = invocation.path.last().cloned().unwrap_or_default();
    let printed = match leaf.as_str() {
        "list" => list(state, &invocation, port).await?,
        "add" => refusal(add(state, args(&invocation)?, port).await)?,
        "update" => refusal(update(state, args(&invocation)?, port).await)?,
        "remove" => refusal(remove(state, args(&invocation)?, port).await)?,
        "enable" => refusal(switch(state, args(&invocation)?, port, true).await)?,
        "disable" => refusal(switch(state, args(&invocation)?, port, false).await)?,
        _ => return Err(PluginError::failed("no such skills command")),
    };
    let rpc = port.rpc();
    match printed {
        Ok(text) => {
            rpc.stdout(text).await?;
            Ok(0)
        }
        Err(reason) => {
            rpc.stderr(format!("skills {leaf}: {reason}\n")).await?;
            Ok(1)
        }
    }
}

/// A refusal the page would see becomes what the command prints on stderr;
/// any other failure fails the call.
fn refusal(result: Result<Printed, PluginError>) -> Result<Printed, PluginError> {
    match result {
        Err(PluginError::Refused { message, .. }) => Ok(Err(message)),
        Err(PluginError::Port { refusal }) => Ok(Err(refusal.to_string())),
        result => result,
    }
}

fn args<A: DeserializeOwned>(invocation: &RpcInvocation) -> Result<A, PluginError> {
    serde_json::from_value(Value::Object(invocation.args.clone())).map_err(|error| {
        PluginError::failed(format!("the arguments do not decode as declared: {error}"))
    })
}

/// The id of the source `repository` names.
fn source_id(repository: &str) -> Result<String, PluginError> {
    Origin::parse(repository)
        .map(|origin| origin.id())
        .map_err(|message| PluginError::refused("invalid_origin", message))
}

async fn list(
    state: &Rc<State>,
    invocation: &RpcInvocation,
    port: &PluginPort,
) -> Result<Printed, PluginError> {
    let listed = state.state(port).await?;
    if invocation.json {
        let mut text = serde_json::to_string(&listed).expect("the state encodes as JSON");
        text.push('\n');
        return Ok(Ok(text));
    }
    if listed.sources.is_empty() {
        return Ok(Ok("No skill sources.\n".into()));
    }
    let mut text = String::new();
    for source in &listed.sources {
        text.push_str(&source_line(source));
        for skill in &source.skills {
            let on = if skill.enabled { "on " } else { "off" };
            text.push_str(&format!("  {on}  {} — {}", skill.name, skill.description));
            if let Some(origin) = &skill.taken_by {
                text.push_str(&format!(" (a skill of this name from {origin} is on)"));
            }
            text.push('\n');
        }
    }
    Ok(Ok(text))
}

/// A source's line: its origin, then its commit, its failure, whether a
/// fetch runs and whether an update is available.
fn source_line(source: &SourceState) -> String {
    let mut parts = vec![source.origin.clone()];
    if let Some(commit) = &source.commit {
        parts.push(format!("at {}", short(commit)));
    }
    if source.fetching {
        parts.push("updating".into());
    }
    if let Some(failure) = &source.failure {
        parts.push(format!("failed: {}", failure.message));
    }
    if source.update_available {
        parts.push("update available".into());
    }
    format!("{}\n", parts.join(", "))
}

async fn add(state: &Rc<State>, args: SkillsArgs, port: &PluginPort) -> Result<Printed, PluginError> {
    let id = state.add_source(port, &args.repository).await?;
    let fetched = state.wait_fetch(port, &id).await?;
    if let Some(refused) = fetch_refusal(fetched) {
        return Ok(Err(refused));
    }
    let (source, _) = sources::read_one(port, &id).await?;
    let Some(named) = args.skill else {
        return Ok(Ok(format!(
            "Added {}{}",
            pinned(&source),
            changes(None, &source)
        )));
    };
    let named: BTreeSet<String> = named.into_iter().collect();
    let all = sources::read(port).await?;
    let mut problems = Vec::new();
    for name in &named {
        if !source.skills.iter().any(|skill| skill.name == *name) {
            problems.push(format!("the commit has no skill \"{name}\""));
            continue;
        }
        if let Err(error) = sources::switch(&all, &id, &BTreeSet::from([name.clone()]), true) {
            problems.push(error.to_string());
        }
    }
    if !problems.is_empty() {
        return Ok(Err(format!(
            "{} was added with its skills as a new source's start: {}",
            source.origin,
            problems.join("; ")
        )));
    }
    let chosen = state.choose(port, &id, &named).await?;
    Ok(Ok(format!("Added {}{}", pinned(&chosen), changes(None, &chosen))))
}

async fn update(
    state: &Rc<State>,
    args: RepositoryArgs,
    port: &PluginPort,
) -> Result<Printed, PluginError> {
    let id = source_id(&args.repository)?;
    let (before, _) = sources::read_one(port, &id).await?;
    let fetched = state.wait_fetch(port, &id).await?;
    if let Some(refused) = fetch_refusal(fetched) {
        return Ok(Err(refused));
    }
    let (after, _) = sources::read_one(port, &id).await?;
    if after.commit.is_some() && after.commit == before.commit {
        return Ok(Ok(format!(
            "{} is at its newest commit, {}; nothing changed.\n",
            after.origin,
            after.commit.as_deref().map(short).unwrap_or_default()
        )));
    }
    Ok(Ok(format!(
        "Updated {}{}",
        pinned(&after),
        changes(Some(&before), &after)
    )))
}

async fn remove(
    state: &Rc<State>,
    args: RepositoryArgs,
    port: &PluginPort,
) -> Result<Printed, PluginError> {
    let id = source_id(&args.repository)?;
    let (source, _) = sources::read_one(port, &id).await?;
    state.remove_source(port, &id).await?;
    let names: Vec<&str> = source.skills.iter().map(|skill| skill.name.as_str()).collect();
    let skills = if names.is_empty() {
        String::new()
    } else {
        format!(" and its skills: {}", names.join(", "))
    };
    Ok(Ok(format!("Removed {}{skills}.\n", source.origin)))
}

async fn switch(
    state: &Rc<State>,
    args: SkillsArgs,
    port: &PluginPort,
    enabled: bool,
) -> Result<Printed, PluginError> {
    let id = source_id(&args.repository)?;
    let (before, _) = sources::read_one(port, &id).await?;
    let named = args.skill.map(|names| names.into_iter().collect::<BTreeSet<_>>());
    let chosen = |source: &Source| match &named {
        Some(names) => names.clone(),
        None => source.skills.iter().map(|skill| skill.name.clone()).collect(),
    };
    state.switch(port, &id, chosen, enabled).await?;
    let (after, _) = sources::read_one(port, &id).await?;
    let changed = changes(Some(&before), &after);
    if changed.is_empty() {
        return Ok(Ok(format!("{}: nothing changed.\n", after.origin)));
    }
    Ok(Ok(format!("{}:\n{}", after.origin, changed.trim_start_matches('\n'))))
}

/// What a fetch that did not pin a commit prints, if it did not.
fn fetch_refusal(fetched: Fetched) -> Option<String> {
    match fetched {
        Fetched::Pinned => None,
        Fetched::Failed(message) => Some(format!("the fetch failed: {message}")),
        Fetched::Removed => Some("the source was removed while it was fetched".into()),
        Fetched::Stopped => Some("the backend stopped during the fetch".into()),
    }
}

/// `<origin> at <commit>.` and a line break.
fn pinned(source: &Source) -> String {
    match &source.commit {
        Some(commit) => format!("{} at {}.\n", source.origin, short(commit)),
        None => format!("{}.\n", source.origin),
    }
}

/// The skills a change turned on and off, from `before`, none for a new
/// source, to `after`; a new source also lists the skills that start off.
fn changes(before: Option<&Source>, after: &Source) -> String {
    let was: BTreeMap<&str, bool> = before
        .map(|source| {
            source
                .skills
                .iter()
                .map(|skill| (skill.name.as_str(), skill.enabled))
                .collect()
        })
        .unwrap_or_default();
    let now: BTreeMap<&str, bool> = after
        .skills
        .iter()
        .map(|skill| (skill.name.as_str(), skill.enabled))
        .collect();
    let on: Vec<&str> = now
        .iter()
        .filter(|(name, enabled)| **enabled && !was.get(*name).copied().unwrap_or(false))
        .map(|(name, _)| *name)
        .collect();
    let off: Vec<&str> = was
        .iter()
        .filter(|(name, enabled)| **enabled && !now.get(*name).copied().unwrap_or(false))
        .map(|(name, _)| *name)
        .collect();
    let mut text = String::new();
    if !on.is_empty() {
        text.push_str(&format!("Turned on: {}\n", on.join(", ")));
    }
    if !off.is_empty() {
        text.push_str(&format!("Turned off: {}\n", off.join(", ")));
    }
    if before.is_none() {
        let left: Vec<&str> = now
            .iter()
            .filter(|(_, enabled)| !**enabled)
            .map(|(name, _)| *name)
            .collect();
        if !left.is_empty() {
            text.push_str(&format!("Left off: {}\n", left.join(", ")));
        }
    }
    text
}

/// A commit's first 12 hexadecimal digits.
fn short(commit: &str) -> &str {
    &commit[..commit.len().min(12)]
}

impl State {
    /// Starts a fetch of source `id`, or joins the one that runs, and waits
    /// for its end.
    async fn wait_fetch(self: &Rc<Self>, port: &PluginPort, id: &str) -> Result<Fetched, PluginError> {
        let mut end = self.start_fetch(port.clone(), id.to_owned());
        let ended = end
            .wait_for(Option::is_some)
            .await
            .map_err(|_| PluginError::failed("the fetch was cut off"))?
            .clone();
        ended
            .expect("the wait ends with the fetch's end")
            .map_err(PluginError::failed)
    }

    /// Turns on exactly the skills `named` of source `id` and the others
    /// off, writing again over another write that came first.
    async fn choose(
        &self,
        port: &PluginPort,
        id: &str,
        named: &BTreeSet<String>,
    ) -> Result<Source, PluginError> {
        let chosen = loop {
            let mut all = sources::read(port).await?;
            let (source, revision) = all.get(id).cloned().ok_or_else(|| sources::not_found(id))?;
            let others: BTreeSet<String> = source
                .skills
                .iter()
                .map(|skill| skill.name.clone())
                .filter(|name| !named.contains(name))
                .collect();
            let off = sources::switch(&all, id, &others, false)?;
            all.insert(id.to_owned(), (off, revision));
            let chosen = sources::switch(&all, id, named, true)?;
            match sources::write(port, id, &chosen, Some(revision)).await {
                Ok(_) => break chosen,
                Err(failure) if sources::conflict(&failure) => continue,
                Err(failure) => return Err(failure.into()),
            }
        };
        self.directories_changed(port).await?;
        Ok(chosen)
    }
}
