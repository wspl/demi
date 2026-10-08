//! The product's `demi conversation` group (`sessions-and-targets.md`
//! § What the agent can change): with the user's Organize Conversations, an
//! agent renames its conversation at once, lists the user's projects, and
//! moves its conversation into or out of a project, or into a project it
//! makes of a directory, as a pending move that the shard makes once the
//! conversation's tree is next idle (`pending.rs`). And what the dispatch
//! needs to know of a call that names a project or a device: whether it
//! brings a paired device into the conversation (`permissions.md` § Several
//! categories).

use std::future::Future;
use std::rc::{Rc, Weak};

use demi_backend_database::conversation_index::{ChangeOutcome, RecordChange};
use demi_backend_database::workspaces::{WorkspaceRecord, new_workspace_id};
use demi_backend_host_access::host_commands::{conversation_of, reachable};
use demi_backend_page_sync::Part;
use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_web_api_protocol::conversations::{ConversationTarget, TITLE_MAX};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::text::Trimmed;
use demi_web_api_protocol::workspaces::WORKSPACE_NAME_MAX;
use futures_util::future::LocalBoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use typed_path::Utf8TypedPath;

use crate::shard::Shard;

/// The permission category of the commands that organize the conversation.
pub(crate) const ORGANIZE: &str = "conversation.organize";

const ORGANIZE_ACTION: &str = "organize conversations";

const ORGANIZE_DESCRIPTION: &str = "Organize Conversations lets the agents of this conversation rename it, list your projects, make a project of a directory, and move this conversation into or out of one. Projects show in your sidebar on every device.";

const SUMMARY: &str = "This conversation in the user's sidebar: rename it, list the user's projects, move it into or out of one, or make one of a directory.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Renames this conversation, lists the user's projects, and moves this conversation into or out of a project, or into a new project made of a directory, which changes where its commands run once its work ends. Use it when the user asks to organize the conversation, or when the work belongs in a project of the user's.";

const RENAME_SUMMARY: &str = "Rename this conversation, as its title in the user's sidebar.";

const PROJECTS_SUMMARY: &str = "The user's projects: name, id, Host, directory.";

const MOVE_SUMMARY: &str = "Move this conversation into a project, by name or id from `demi conversation projects`, or out of its project with --out, to the same directory on the same Host. The move applies when this conversation's work ends; the next turn runs in the new place.";

const CREATE_PROJECT_SUMMARY: &str = "Make a project of a directory on the primary Host, the shell's directory by default, and move this conversation into it once its work ends.";

/// The input of `demi conversation rename`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenameArgs {
    /// The new title
    #[schemars(length(min = 1, max = TITLE_MAX))]
    title: String,
}

/// The input of a leaf that takes none.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// The input of `demi conversation move`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MoveArgs {
    /// Project name or id from demi conversation projects
    project: Option<String>,
    /// Move this conversation out of its project instead
    out: Option<bool>,
}

/// The input of `demi conversation create-project`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateProjectArgs {
    /// The project's name, as the user's sidebar shows it
    #[schemars(length(min = 1, max = WORKSPACE_NAME_MAX))]
    name: String,
    /// The project's directory on the primary Host, absolute or relative to
    /// the shell's directory; the shell's directory by default
    directory: Option<String>,
}

/// The `conversation` group, whose handlers act in `shard`.
pub(crate) fn conversation_group(shard: Weak<Shard>) -> GroupBuilder {
    GroupBuilder::new("conversation", SUMMARY)
        .index_entry(ENTRY)
        .permission(ORGANIZE, ORGANIZE_ACTION, ORGANIZE_DESCRIPTION)
        .leaf(
            LeafBuilder::rpc("rename", RENAME_SUMMARY)
                .input::<RenameArgs>()
                .positionals(["title"])
                .permission(ORGANIZE)
                .bind(TypedRpc::new(verb(shard.clone(), rename))),
        )
        .leaf(
            LeafBuilder::rpc("projects", PROJECTS_SUMMARY)
                .input::<NoArgs>()
                .permission(ORGANIZE)
                .bind(TypedRpc::new(verb(shard.clone(), projects))),
        )
        .leaf(
            LeafBuilder::rpc("move", MOVE_SUMMARY)
                .input::<MoveArgs>()
                .positionals(["project"])
                .permission(ORGANIZE)
                .brings_host("project")
                .bind(TypedRpc::new(verb(shard.clone(), move_to))),
        )
        .leaf(
            LeafBuilder::rpc("create-project", CREATE_PROJECT_SUMMARY)
                .input::<CreateProjectArgs>()
                .positionals(["name", "directory"])
                .permission(ORGANIZE)
                .bind(TypedRpc::new(verb(shard, create_project))),
        )
}

/// A handler over the live shard; a call after the shard closed fails.
fn verb<A, F, Fut>(
    shard: Weak<Shard>,
    run: F,
) -> impl Fn(Call<A>, RpcPort) -> LocalBoxFuture<'static, Result<u8, RpcError>>
where
    A: 'static,
    F: Fn(Rc<Shard>, Call<A>, RpcPort) -> Fut + Clone + 'static,
    Fut: Future<Output = Result<u8, RpcError>> + 'static,
{
    move |call, port| {
        let shard = shard.upgrade();
        let run = run.clone();
        Box::pin(async move {
            let shard =
                shard.ok_or_else(|| RpcError::Failed("the backend is shutting down".into()))?;
            run(shard, call, port).await
        })
    }
}

fn failed(error: impl ToString) -> RpcError {
    RpcError::Failed(error.to_string())
}

async fn rename(shard: Rc<Shard>, call: Call<RenameArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let title = Trimmed::from(call.args.title).into_string();
    if title.is_empty() {
        port.stderr("conversation rename: the title is blank\n").await?;
        return Ok(2);
    }
    if let Err(refusal) = shard
        .transition(&conversation, RecordChange::Title(title.clone()).into())
        .await
    {
        port.stderr(format!("conversation rename: {refusal}\n")).await?;
        return Ok(1);
    }
    port.stdout(format!("Renamed this conversation to \u{201c}{title}\u{201d}.\n"))
        .await?;
    Ok(0)
}

async fn projects(shard: Rc<Shard>, _call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let workspaces = shard
        .services()
        .control
        .workspaces(shard.user().clone())
        .await
        .map_err(failed)?;
    if workspaces.is_empty() {
        port.stdout("The user has no projects.\n").await?;
        return Ok(0);
    }
    let mut lines = Vec::new();
    for workspace in &workspaces {
        let host = shard.host_name(&workspace.device).await;
        lines.push(format!(
            "{}  {}  {host}  {}",
            workspace.name, workspace.id, workspace.path
        ));
    }
    port.stdout(format!("{}\n", lines.join("\n"))).await?;
    Ok(0)
}

/// What a project name or id names among the user's projects: its id
/// first, then its name, which must name one alone.
enum Named {
    One(WorkspaceRecord),
    None,
    Several,
}

async fn named_project(shard: &Shard, wanted: &str) -> Result<Named, RpcError> {
    let workspaces = shard
        .services()
        .control
        .workspaces(shard.user().clone())
        .await
        .map_err(failed)?;
    if let Some(workspace) = workspaces
        .iter()
        .find(|workspace| workspace.id.as_str() == wanted)
    {
        return Ok(Named::One(workspace.clone()));
    }
    let mut named = workspaces
        .into_iter()
        .filter(|workspace| workspace.name == wanted);
    Ok(match (named.next(), named.next()) {
        (Some(workspace), None) => Named::One(workspace),
        (None, _) => Named::None,
        (Some(_), Some(_)) => Named::Several,
    })
}

/// The target of the directory a project names, outside the project: the
/// same directory on the same Host.
async fn outside(shard: &Shard, workspace: &WorkspaceRecord) -> Result<ConversationTarget, RpcError> {
    let device = shard
        .services()
        .control
        .device(workspace.device.clone())
        .await
        .map_err(failed)?;
    Ok(match device {
        Some(device) if device.kind == DeviceKind::Managed => ConversationTarget::Cloud {
            path: Some(workspace.path.clone()),
        },
        _ => ConversationTarget::Device {
            device_id: workspace.device.clone(),
            path: workspace.path.clone(),
        },
    })
}

async fn move_to(shard: Rc<Shard>, call: Call<MoveArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let record = shard
        .host_shard()
        .owned_conversation(&conversation)
        .await
        .map_err(failed)?;
    let MoveArgs { project, out } = call.args;
    let (target, said) = match (project, out.unwrap_or(false)) {
        (Some(wanted), false) => {
            let workspace = match named_project(&shard, &wanted).await? {
                Named::One(workspace) => workspace,
                Named::None => {
                    port.stderr(format!(
                        "conversation move: no project {wanted} (see `demi conversation projects`)\n"
                    ))
                    .await?;
                    return Ok(1);
                }
                Named::Several => {
                    port.stderr(format!(
                        "conversation move: several projects are named {wanted}; name one by its id (see `demi conversation projects`)\n"
                    ))
                    .await?;
                    return Ok(1);
                }
            };
            let host = shard.host_name(&workspace.device).await;
            let said = format!(
                "The move into {} ({host}, {}) applies when this conversation's work ends; until then commands run where they do now.\n",
                workspace.name, workspace.path
            );
            (
                ConversationTarget::Workspace {
                    workspace_id: workspace.id,
                },
                said,
            )
        }
        (None, true) => {
            let ConversationTarget::Workspace { workspace_id } = &record.target else {
                port.stderr("conversation move: this conversation is not in a project\n")
                    .await?;
                return Ok(1);
            };
            let workspace = shard
                .services()
                .control
                .workspace(workspace_id.clone())
                .await
                .map_err(failed)?
                .ok_or_else(|| failed("this conversation's project is gone"))?;
            let said = format!(
                "The move out of {} applies when this conversation's work ends; it then runs in {} without a project.\n",
                workspace.name, workspace.path
            );
            (outside(&shard, &workspace).await?, said)
        }
        _ => {
            port.stderr("usage: demi conversation move <project> | demi conversation move --out\n")
                .await?;
            return Ok(2);
        }
    };
    // A move to where it runs replaces a move that waits, and is made as
    // nothing.
    let there = record.target == target;
    pend(&shard, &conversation, target).await?;
    if there {
        port.stdout("This conversation runs there already.\n").await?;
    } else {
        port.stdout(said).await?;
    }
    Ok(0)
}

/// Records the move to `target` as the conversation's pending move, which
/// the shard makes once its tree is next idle.
async fn pend(
    shard: &Shard,
    conversation: &ConversationId,
    target: ConversationTarget,
) -> Result<(), RpcError> {
    match shard
        .services()
        .control
        .set_pending_move(conversation.clone(), target)
        .await
        .map_err(failed)?
    {
        Ok(_) => {
            shard.settle_when_idle(conversation);
            Ok(())
        }
        Err(ChangeOutcome::Archived) => Err(failed("this conversation is archived")),
        Err(_) => Err(failed("this conversation is gone")),
    }
}

/// `directory` on the Host whose shell is in `cwd`: as it is when it is
/// absolute, otherwise under `cwd`, in the Host's own spelling of paths.
fn on_host(cwd: &str, directory: &str) -> String {
    Utf8TypedPath::derive(cwd)
        .join(directory)
        .normalize()
        .into_string()
}

async fn create_project(
    shard: Rc<Shard>,
    call: Call<CreateProjectArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let Call { args, invocation } = call;
    let conversation = conversation_of(&invocation)?;
    let name = Trimmed::from(args.name).into_string();
    if name.is_empty() {
        port.stderr("conversation create-project: the name is blank\n")
            .await?;
        return Ok(2);
    }
    let host = shard.host_shard();
    let record = host.owned_conversation(&conversation).await.map_err(failed)?;
    let target = host.resolve_target(&record).await.map_err(failed)?;
    let Some(device) = target.device().cloned() else {
        return Err(failed("this conversation's primary Host has not run yet"));
    };
    // The shell's directory is a directory of the Host the command runs on,
    // which `demi host shell` may have made another.
    if invocation.host != device.as_str() {
        port.stderr(
            "conversation create-project: run it on this conversation's primary Host, not through `demi host shell`\n",
        )
        .await?;
        return Ok(1);
    }
    let path = on_host(&invocation.cwd, args.directory.as_deref().unwrap_or("."));
    let control = &shard.services().control;
    let workspace = control
        .create_workspace(
            new_workspace_id(),
            shard.user().clone(),
            device.clone(),
            path,
            name,
        )
        .await
        .map_err(failed)?
        .ok_or_else(|| failed("this conversation's primary Host is no longer the user's"))?;
    shard.mark(Part::Workspaces);
    let host_name = shard.host_name(&device).await;
    pend(
        &shard,
        &conversation,
        ConversationTarget::Workspace {
            workspace_id: workspace.id.clone(),
        },
    )
    .await?;
    port.stdout(format!(
        "Made the project {} of {} on {host_name}. The move into it applies when this conversation's work ends; until then commands run where they do now.\n",
        workspace.name, workspace.path
    ))
    .await?;
    Ok(0)
}

impl Shard {
    /// Whether the project or device `named`, as a `bringsHost` argument
    /// names it, brings into the conversation a paired device that is
    /// neither its primary Host nor attached (`permissions.md` § Several
    /// categories): a project by id or name stands for its device, then a
    /// paired device by id or name. A name that names nothing brings
    /// nothing; the handler refuses it.
    pub(crate) async fn brings_paired_device(
        &self,
        conversation: &ConversationId,
        named: &str,
    ) -> Result<bool, String> {
        let control = &self.services().control;
        let device = match named_project(self, named).await.map_err(|error| error.to_string())? {
            Named::One(workspace) => Some(workspace.device),
            Named::Several => None,
            Named::None => {
                let paired = control
                    .paired_devices(self.user().clone())
                    .await
                    .map_err(|error| error.to_string())?;
                paired
                    .iter()
                    .find(|device| device.id.as_str() == named)
                    .or_else(|| paired.iter().find(|device| device.name == named))
                    .map(|device| device.id.clone())
            }
        };
        let Some(device) = device else {
            return Ok(false);
        };
        let paired = control
            .device(device.clone())
            .await
            .map_err(|error| error.to_string())?
            .is_some_and(|record| record.kind == DeviceKind::User);
        if !paired {
            return Ok(false);
        }
        let hosts = reachable(self.host_shard(), conversation)
            .await
            .map_err(|error| error.to_string())?;
        Ok(!hosts.iter().any(|host| host.device == device))
    }
}
