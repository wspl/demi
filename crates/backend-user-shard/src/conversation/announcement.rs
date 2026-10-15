//! The context block a node reads before its first request, and before
//! its next one once the conversation's execution context changed
//! (`sessions-and-targets.md` § Switch the primary target). Every block
//! names the primary Host with its system and the directory its shells
//! start in. Then, after a switch, the departed and current targets, that
//! no files moved, and the departed device under the name it stays
//! attached as; after a change of the attached hosts, that change. Every
//! block ends with the attached hosts as they now stand, and a block after
//! a reset of the user's Cloud says so before what changed. What a node saw
//! is the context blocks of its own transcript: each block names the
//! revision it describes, and a switch and a reset are each announced to a
//! node once.
//!
//! The reason a user's Resume carries is composed here too, from the same
//! primary Host: where a turn its offline Host left unfinished now runs.

use demi_backend_database::StorageError;
use demi_backend_database::conversation_index::{
    AttachedHost, ConversationRecord, ExecutionTarget, TargetSwitch,
};
use demi_agent_session::left_by_offline_host;
use demi_backend_database::devices::{CLOUD_NAME, DeviceRecord};
use demi_agent_store::Placement;
use demi_backend_database::tree;
use demi_conversation_socket_protocol::SubagentJob;
use demi_agent_tools::NodeContext;
use demi_backend_host_access::access::HostAccessError;
use demi_backend_host_access::{conversation_of, root_of};
use demi_backend_remote_host::host_paths;
use demi_host_interface::HostPaths;
use demi_runner_protocol::wire::RunnerPlatform;
use demi_web_api_protocol::ids::ConversationId;

use crate::shard::Shard;
use super::runs_on::runs_on;

/// Why the conversation's subagents could not be listed.
#[derive(Debug, thiserror::Error)]
pub enum SubagentsError {
    #[error(transparent)]
    Host(#[from] HostAccessError),
    /// The conversation's database could not be read.
    #[error(transparent)]
    Unreadable(StorageError),
}

/// What opens every context block, before the revision it describes.
const CONTEXT: &str = "[Execution context ";

/// The line that opens a switch's announcement.
const SWITCHED: &str = "[Execution target switched]";

/// What every block says of a job's standard utilities (`runner.md`
/// § Standard utilities).
const UTILITIES: &str = "Standard utilities (sed, grep, cp, ls, find, …) are GNU's on every host";

/// Where the agent keeps the files only it needs (`runner.md` § Scratch
/// directory), which every block says after where shells start.
const SCRATCH: &str = "Files only you need (screenshots, probe scripts) go in $DEMI_SCRATCH, which shows as no change and lasts as long as the conversation.";

/// Where a Mac keeps its own, BSD utilities, which a block for a Mac adds.
const MACOS_UTILITIES: &str = "macOS's own are in /usr/bin";

/// What a block for Windows adds: what the shell is, since a model
/// otherwise guesses Git Bash or MSYS and writes `/c/…` paths
/// (`sessions-and-targets.md` § Execution context).
const WINDOWS_SHELLS: &str = "Shells are bash (Demi's own, not Git Bash or WSL); Windows programs such as cmd, powershell and git run by name. Write paths as C:/Users/…, not /c/Users/…; quote a path with backslashes.";

/// The line of a block after the conversation's attached hosts changed.
const ATTACHED_CHANGED: &str = "[Attached hosts changed]";

/// What opens a block's line of attached hosts.
const ATTACHED: &str = "Attached hosts: ";

/// What follows each attached host's connection in that line.
const STARTS_IN: &str = ", shells start in ";

/// What a node learns once the user's Cloud was reset.
const CLOUD_RESET: &str = "Cloud was reset: system packages and configuration were rebuilt from the base image. Files under /home remain. Running processes, temporary files, and previous shell state are gone; check the environment before continuing.";

impl Shard {
    /// The context block for a node of the conversation `id` whose
    /// transcript holds the context texts `seen`, oldest first; none when
    /// the node saw the current revision.
    pub(crate) async fn execution_context(
        &self,
        id: &ConversationId,
        seen: &[&str],
    ) -> Result<Option<String>, StorageError> {
        let control = &self.services().control;
        let Some(record) = control.conversation(id.clone()).await? else {
            return Ok(None);
        };
        let marker = format!("{CONTEXT}{}]", record.context_version);
        if seen.iter().any(|text| text.contains(&marker)) {
            return Ok(None);
        }
        let primary = self.host_shard().resolve_target(&record).await?;
        let attached = control.attached_hosts(record.id.clone()).await?;
        let mut lines = vec![marker, self.primary_host_line(&primary).await?];
        // A Cloud reset is announced to a node once (`managed-hosts.md`
        // § System reset).
        if let Some(reset) = control.announced_cloud_reset(record.id.clone()).await? {
            let reset = format!("[Cloud reset {reset}]");
            if !seen.iter().any(|text| text.contains(&reset)) {
                lines.push(reset);
                lines.push(CLOUD_RESET.to_owned());
            }
        }
        // A node saw the switch when it read a block of the revision the
        // switch made or a later one, whatever the block called its targets:
        // their names may have changed since.
        let announced = match control.last_switch(record.id.clone()).await? {
            Some((switch, revision))
                if !seen.iter().any(|text| read_revision(text).is_some_and(|read| read >= revision)) =>
            {
                let description = format!(
                    "Previous target: {}. Current target: {}. New shells start in {}.",
                    self.describe(&switch.from).await?,
                    self.describe(&switch.to).await?,
                    switch.to.path()
                );
                Some(switch_lines(&switch, description, &attached))
            }
            _ => None,
        };
        // A node that read an earlier revision learns that the attached
        // hosts changed since, when they did; a block a runner's report
        // caused names no change, and one a node reads first learns the
        // context as it stands.
        let earlier = seen.iter().rev().find(|text| text.contains(CONTEXT));
        match announced {
            Some(announcement) => lines.extend(announcement),
            None if earlier.is_some_and(|text| attached_changed(text, &attached)) => {
                lines.push(ATTACHED_CHANGED.to_owned());
            }
            None => {}
        }
        lines.push(self.attached_hosts_line(&attached));
        Ok(Some(lines.join("\n")))
    }

    /// What the model reads as the user resumes `record`'s turn its
    /// offline Host left unfinished (`failures-and-recovery.md`
    /// § The unfinished turn): "alpha is back online." when the
    /// conversation still runs on that device, or "This conversation now
    /// runs on beta." after a move. None for any other resume, and while
    /// the conversation's tree is not open, when the resume is refused.
    pub(crate) async fn resume_reason(
        &self,
        record: &ConversationRecord,
    ) -> Result<Option<String>, StorageError> {
        let Some(tree) = self.agent().tree(&root_of(&record.id)) else {
            return Ok(None);
        };
        let blocks = tree.root().session().transcript().blocks;
        let Some(away) = left_by_offline_host(&blocks) else {
            return Ok(None);
        };
        let primary = self.host_shard().resolve_target(record).await?;
        if primary.device().is_some_and(|device| device.as_str() == away.id) {
            return Ok(Some(format!("{} is back online.", away.name)));
        }
        let (_, name) = self.primary_host(&primary).await?;
        Ok(Some(format!("This conversation now runs on {name}.")))
    }

    /// The primary Host's device record, when it has one, and its name as
    /// the model reads it: the device's, or Cloud for a Cloud not made yet.
    async fn primary_host(
        &self,
        target: &ExecutionTarget,
    ) -> Result<(Option<DeviceRecord>, String), StorageError> {
        let device = match target.device() {
            Some(device) => self.services().control.device(device.clone()).await?,
            None => None,
        };
        let name = match (&device, target.device()) {
            (Some(record), _) => record.name.clone(),
            (None, Some(device)) => device.to_string(),
            (None, None) => CLOUD_NAME.to_owned(),
        };
        Ok((device, name))
    }

    /// The working directory of `node` on the primary Host: a child's own,
    /// and for the root the conversation's, its target's directory
    /// (`subagents.md` § Working directory).
    pub(crate) async fn node_directory(&self, node: NodeContext<'_>) -> Result<String, HostAccessError> {
        match node.cwd {
            Some(cwd) => Ok(cwd.to_owned()),
            None => Ok(self.placement(&conversation_of(node.root)).await?.directory),
        }
    }

    /// Where the conversation `id` works now: its primary Host's device,
    /// none for a Cloud not made yet, and its directory there, read without
    /// reaching the Host.
    pub async fn placement(&self, id: &ConversationId) -> Result<Placement, HostAccessError> {
        let record = self.host_shard().owned_conversation(id).await?;
        let target = self.host_shard().resolve_target(&record).await?;
        Ok(Placement {
            device: target.device().map(ToString::to_string),
            directory: target.path().to_owned(),
        })
    }

    /// Every subagent the conversation `id` has had, in the order they
    /// started, each working where the conversation works now: its live
    /// tree's own list whenever a tree is live as the answer is made, the
    /// one the open's `subagents` frame sends, and otherwise the store's
    /// (`subagents.md` § Protocol). The store's reading awaits, and a tree
    /// opened meanwhile answers instead, so the answer never disagrees with
    /// a handshake made before it.
    pub async fn subagents(&self, id: &ConversationId) -> Result<Vec<SubagentJob>, SubagentsError> {
        let placement = self.placement(id).await?;
        let live = || {
            self.agent()
                .tree(&root_of(id))
                .map(|tree| tree.subagent_jobs(Some(&placement)))
        };
        if let Some(listed) = live() {
            return Ok(listed);
        }
        let nodes = self
            .services()
            .conversations
            .read(id, tree::subagents)
            .await
            .map_err(SubagentsError::Unreadable)?
            .unwrap_or_default();
        let selections: Vec<_> = nodes.iter().map(|(_, model)| model.clone()).collect();
        let models = runs_on(self.services(), self.user(), &selections).await;
        Ok(live().unwrap_or_else(|| {
            nodes
                .iter()
                .zip(models)
                .filter_map(|((node, _), model)| node.job(model, Some(&placement)))
                .collect()
        }))
    }

    /// How the primary Host of the conversation `id` writes its paths: in
    /// the form of the system its runner reported, which the device keeps;
    /// a Cloud's is Linux's.
    pub(crate) async fn host_paths(&self, id: &ConversationId) -> Result<HostPaths, StorageError> {
        let Some(record) = self.services().control.conversation(id.clone()).await? else {
            return Ok(HostPaths::Posix);
        };
        let target = self.host_shard().resolve_target(&record).await?;
        let (device, _) = self.primary_host(&target).await?;
        Ok(device.map_or(HostPaths::Posix, |record| host_paths(record.platform)))
    }

    /// The primary Host as the model reads it: its name, its operating
    /// system with its architecture as its runner last reported them, the
    /// directory its shells start in, where the files only the agent needs
    /// go, and that its standard utilities are GNU's, with where a Mac's own
    /// are and what a Windows Host's shell is. A Host whose runner never connected, such as a Cloud not made
    /// yet, is named without its system.
    async fn primary_host_line(&self, target: &ExecutionTarget) -> Result<String, StorageError> {
        let (device, name) = self.primary_host(target).await?;
        let utilities = match device.as_ref().map(|record| &record.platform) {
            Some(RunnerPlatform::Darwin) => format!("{UTILITIES}; {MACOS_UTILITIES}."),
            Some(RunnerPlatform::Win32) => format!("{UTILITIES}. {WINDOWS_SHELLS}"),
            _ => format!("{UTILITIES}."),
        };
        let host = match device.and_then(|record| record.os) {
            Some(os) => format!("{name}, {} ({})", os.name, os.arch),
            None => name,
        };
        Ok(format!(
            "Primary host: {host}. Shells start in {}. {SCRATCH} {utilities}",
            target.path()
        ))
    }

    /// A target as the model reads it.
    async fn describe(&self, target: &ExecutionTarget) -> Result<String, StorageError> {
        let control = &self.services().control;
        let Some(device) = target.device() else {
            return Ok("Cloud (not allocated)".into());
        };
        let name = match control.device(device.clone()).await? {
            Some(record) => format!("\"{}\"", record.name),
            None => device.to_string(),
        };
        Ok(match target {
            ExecutionTarget::Workspace {
                workspace_id, path, ..
            } => {
                let workspace = control
                    .workspace(workspace_id.clone())
                    .await?
                    .map_or_else(|| workspace_id.to_string(), |workspace| workspace.name);
                format!("workspace \"{workspace}\" — directory {path} on device {name}")
            }
            ExecutionTarget::Cloud { .. } | ExecutionTarget::Device { .. } => {
                format!("the machine {name} (host {device})")
            }
        })
    }

    /// The attached hosts as one line: each one's name, connection and
    /// directory, and how to reach them.
    fn attached_hosts_line(&self, attached: &[AttachedHost]) -> String {
        const LIST: &str = "`demi host list` shows every host this conversation can reach.";
        if attached.is_empty() {
            return format!("{ATTACHED}none. {LIST}");
        }
        let entries: Vec<String> = attached
            .iter()
            .map(|host| {
                let state = if self.devices().online(&host.device) {
                    "online"
                } else {
                    "offline"
                };
                let directory = host
                    .cwd
                    .clone()
                    .or_else(|| self.devices().home(&host.device))
                    .unwrap_or_else(|| "its home directory".into());
                format!("{}{state}{STARTS_IN}{directory})", entry(host))
            })
            .collect();
        format!(
            "{ATTACHED}{}. `demi host shell --host <name> <script>` runs a shell string on one; {LIST}",
            entries.join(", ")
        )
    }
}

/// The revision the context block `text` describes, which its first line
/// names.
fn read_revision(text: &str) -> Option<u64> {
    let (_, rest) = text.split_once(CONTEXT)?;
    rest.split_once(']')?.0.parse().ok()
}

/// How an attached host's entry in a block's line of attached hosts opens:
/// its name and its device.
fn entry(host: &AttachedHost) -> String {
    format!("\"{}\" (host {}, ", host.name, host.device)
}

/// Whether the hosts `attached` now are others than the ones the block
/// `seen`, a node's latest, listed, in order: a host joined or left, or a
/// name now stands for another device, as when a rename gives a device the
/// name another went by. A host's connection or directory is no change of
/// the attached hosts.
fn attached_changed(seen: &str, attached: &[AttachedHost]) -> bool {
    let Some(listed) = seen.lines().find(|line| line.starts_with(ATTACHED)) else {
        return true;
    };
    if listed.matches(STARTS_IN).count() != attached.len() {
        return true;
    }
    let mut rest = listed;
    for host in attached {
        let entry = entry(host);
        let Some(at) = rest.find(&entry) else {
            return true;
        };
        rest = &rest[at + entry.len()..];
    }
    false
}

/// A switch's announcement: what changed, that no file moved, and how to
/// reach the device left behind.
fn switch_lines(
    switch: &TargetSwitch,
    description: String,
    attached: &[AttachedHost],
) -> Vec<String> {
    let mut lines = vec![
        SWITCHED.to_owned(),
        description,
        "No files were moved: everything created earlier lives on the previous target, and file paths from before the switch — including the full outputs of earlier commands — are stale here.".to_owned(),
    ];
    let departed = switch
        .from
        .device()
        .and_then(|device| attached.iter().find(|host| host.device == *device));
    if let Some(departed) = departed {
        let name = &departed.name;
        let from = switch.from.path();
        lines.push(format!(
            "The previous host stays attached as \"{name}\": `demi host shell --host {name} <script>` runs a shell string there with byte-faithful stdio, starting in {from} (e.g. `demi host shell --host {name} \"tar c -C {from} .\" | tar x` pulls its files into the current directory)."
        ));
    }
    if let (
        ExecutionTarget::Workspace {
            device_id: before, ..
        },
        ExecutionTarget::Workspace {
            device_id: after, ..
        },
    ) = (&switch.from, &switch.to)
        && before == after
    {
        lines.push(format!(
            "The previous directory {} is on the same device, so it is also directly accessible from this shell.",
            switch.from.path()
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use demi_backend_database::accounts::TokenHash;
    use demi_backend_database::control::testing;
    use demi_backend_database::control::ControlService;
    use demi_backend_database::conversation_index::{
        AttachedHostRecord, ConversationStart, Creation, ExecutionTarget, RecordChange, SwitchEnds,
        TargetSwitch,
    };
    use demi_backend_database::devices::DeviceChange;
    use demi_runner_protocol::wire::{OperatingSystem, RunnerPlatform};
    use demi_web_api_protocol::conversations::ConversationTarget;
    use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};

    use super::{ATTACHED_CHANGED, SWITCHED};

    use crate::services::Services;
    use crate::shard::{ShardPlacement, ShardPool};

    const ID: &str = "4f0c2a8e-1d3b-4e6f-9a7c-2b5d8e1f3a60";

    /// A conversation of the master's on its device named `primary` with
    /// devices named `others` attached in that order, and the pool that
    /// serves it; answers the owner, the devices and the conversation.
    async fn on_devices(
        data: &std::path::Path,
        primary: &str,
        others: &[&str],
    ) -> (ShardPool, UserId, Vec<DeviceId>, ConversationId, ControlService) {
        let services = Services::start_for_tests(data).await;
        let control = services.control.clone();
        let owner = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        control
            .create_conversation(owner.clone(), id.clone(), ConversationStart::default())
            .await
            .unwrap();
        let mut devices = Vec::new();
        for (index, name) in std::iter::once(&primary).chain(others).enumerate() {
            let device = control
                .create_device(
                    owner.clone(),
                    (*name).into(),
                    RunnerPlatform::Linux,
                    TokenHash::of(&index.to_string()),
                )
                .await
                .unwrap();
            devices.push(device.id);
        }
        testing::execute(
            &control,
            "UPDATE conversations SET target_kind = 'device', target_device_id = ?2, target_path = '/a'
             WHERE id = ?1",
            vec![ID.to_owned(), devices[0].to_string()],
        )
        .await;
        for device in &devices[1..] {
            let attach = RecordChange::Attach(AttachedHostRecord {
                device: device.clone(),
                cwd: None,
            });
            control.change_conversation(id.clone(), attach).await.unwrap();
        }
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        (pool, owner, devices, id, control)
    }

    /// The context block of a node whose transcript holds `seen`.
    async fn block(pool: &ShardPool, owner: &UserId, id: &ConversationId, seen: &[String]) -> Option<String> {
        let id = id.clone();
        let seen = seen.to_vec();
        pool.shards()
            .of(owner)
            .call(move |shard, _| async move {
                let seen: Vec<&str> = seen.iter().map(String::as_str).collect();
                shard.execution_context(&id, &seen).await
            })
            .await
            .unwrap()
            .unwrap()
    }

    async fn rename(control: &ControlService, device: &DeviceId, name: &str) {
        let change = DeviceChange {
            name: Some(name.into()),
            route: None,
        };
        control.change_device(device.clone(), change).await.unwrap().unwrap();
    }

    /// A switch is announced to a node once, however its devices are named
    /// afterwards: renaming either end gives the node a block that names
    /// them anew and announces no second switch.
    /// Planted defect this catches: a switch known by its description,
    /// which names the devices, so a rename announced it again.
    #[tokio::test(flavor = "local")]
    async fn a_rename_after_an_announced_switch_announces_no_other_switch() {
        let data = tempfile::tempdir().unwrap();
        let (pool, owner, devices, id, control) = on_devices(data.path(), "alpha", &["beta"]).await;
        let from = ConversationTarget::Device {
            device_id: devices[0].clone(),
            path: "/a".into(),
        };
        let to = ConversationTarget::Device {
            device_id: devices[1].clone(),
            path: "/b".into(),
        };
        let switch = TargetSwitch {
            from: ExecutionTarget::Device {
                device_id: devices[0].clone(),
                path: "/a".into(),
            },
            to: ExecutionTarget::Device {
                device_id: devices[1].clone(),
                path: "/b".into(),
            },
        };
        let ends = SwitchEnds {
            departed: Some((devices[0].clone(), "/a".into())),
            arriving: Some(devices[1].clone()),
        };
        control
            .switch_conversation_target(id.clone(), from, to, switch, ends, None)
            .await
            .unwrap()
            .expect("the switch wins");
        let mut seen = vec![block(&pool, &owner, &id, &[]).await.unwrap()];
        assert!(seen[0].contains(SWITCHED), "{}", seen[0]);

        rename(&control, &devices[0], "alpha two").await;
        let renamed = block(&pool, &owner, &id, &seen).await.expect("the context changed");
        assert!(!renamed.contains(SWITCHED), "{renamed}");
        assert!(renamed.contains(ATTACHED_CHANGED), "{renamed}");
        assert!(renamed.contains("\"alpha two\""), "{renamed}");
        seen.push(renamed);
        rename(&control, &devices[1], "beta two").await;
        let renamed = block(&pool, &owner, &id, &seen).await.expect("the context changed");
        assert!(!renamed.contains(SWITCHED), "{renamed}");
        assert!(!renamed.contains(ATTACHED_CHANGED), "{renamed}");
        assert!(renamed.contains("Primary host: beta two"), "{renamed}");
        seen.push(renamed);
        assert_eq!(block(&pool, &owner, &id, &seen).await, None);
    }

    /// A Fork's first switch is announced in the Fork once, though the
    /// transcript it copied holds blocks of the source's later revisions
    /// than the Fork's own first ones would be.
    /// Planted defect this catches: a Fork that starts at revision 0, whose
    /// first switch, at revision 1, the copied block of revision 7 hid.
    #[tokio::test(flavor = "local")]
    async fn a_forks_first_switch_is_announced_once_past_the_blocks_it_copied() {
        use demi_backend_database::forks::{ForkMetadata, ForkOperation};
        use demi_shared_types::BlockId;

        let data = tempfile::tempdir().unwrap();
        let (pool, owner, devices, source, control) = on_devices(data.path(), "alpha", &[]).await;
        let beta = control
            .create_device(owner.clone(), "beta".into(), RunnerPlatform::Linux, TokenHash::of("beta"))
            .await
            .unwrap()
            .id;
        // The source's history holds a block of revision 7, which the Fork
        // copies.
        testing::execute(
            &control,
            "UPDATE conversations SET context_version = 7 WHERE id = ?1",
            vec![ID.to_owned()],
        )
        .await;
        let copied = block(&pool, &owner, &source, &[]).await.unwrap();
        assert!(copied.starts_with("[Execution context 7]"), "{copied}");

        let fork = ConversationId::try_from("9e8d7c6b-5a4f-4e3d-8c2b-1a0f9e8d7c6b").unwrap();
        let on_alpha = ConversationTarget::Device {
            device_id: devices[0].clone(),
            path: "/a".into(),
        };
        let operation = ForkOperation {
            id: fork.clone(),
            owner: owner.clone(),
            source: source.clone(),
            block: BlockId::try_from("text-1").unwrap(),
            metadata: ForkMetadata {
                title: Some("Fork".into()),
                target: on_alpha.clone(),
                model: None,
                created_at: "2026-10-14T08:00:00Z".parse().unwrap(),
                attached_hosts: Vec::new(),
                context_version: 7,
            },
        };
        control.reserve_fork(operation).await.unwrap().expect("the id is free");
        control.publish_fork(fork.clone()).await.unwrap();
        let to = ConversationTarget::Device {
            device_id: beta.clone(),
            path: "/b".into(),
        };
        let switch = TargetSwitch {
            from: ExecutionTarget::Device {
                device_id: devices[0].clone(),
                path: "/a".into(),
            },
            to: ExecutionTarget::Device {
                device_id: beta.clone(),
                path: "/b".into(),
            },
        };
        let ends = SwitchEnds {
            departed: Some((devices[0].clone(), "/a".into())),
            arriving: Some(beta),
        };
        control
            .switch_conversation_target(fork.clone(), on_alpha, to, switch, ends, None)
            .await
            .unwrap()
            .expect("the switch wins");

        let mut seen = vec![copied];
        let announced = block(&pool, &owner, &fork, &seen).await.expect("the context changed");
        assert!(announced.contains(SWITCHED), "{announced}");
        seen.push(announced);
        assert_eq!(block(&pool, &owner, &fork, &seen).await, None);
    }

    /// Two attached devices that trade names change the attached hosts:
    /// `--host laptop` now reaches the other device, though the list of
    /// names reads the same.
    /// Planted defect this catches: hosts compared by their names alone.
    #[tokio::test(flavor = "local")]
    async fn two_attached_devices_that_trade_names_change_the_attached_hosts() {
        let data = tempfile::tempdir().unwrap();
        let (pool, owner, devices, id, control) =
            on_devices(data.path(), "desk", &["laptop", "laptop"]).await;
        let first = block(&pool, &owner, &id, &[]).await.unwrap();
        assert!(first.contains(&format!("\"laptop\" (host {}, ", devices[1])), "{first}");
        assert!(first.contains(&format!("\"laptop-2\" (host {}, ", devices[2])), "{first}");

        // The first attached takes the name the second went by, which goes
        // by the first's.
        rename(&control, &devices[1], "laptop-2").await;
        let traded = block(&pool, &owner, &id, &[first]).await.expect("the context changed");
        assert!(traded.contains(ATTACHED_CHANGED), "{traded}");
        assert!(traded.contains(&format!("\"laptop-2\" (host {}, ", devices[1])), "{traded}");
        assert!(traded.contains(&format!("\"laptop\" (host {}, ", devices[2])), "{traded}");
    }

    /// A model on Windows that is not told what the shell is guesses Git
    /// Bash and writes `/c/…` paths, which the runner's utilities refuse.
    #[tokio::test(flavor = "local")]
    async fn a_windows_hosts_block_says_its_shell_is_demis_bash_and_how_to_write_paths() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control
                .create_conversation(owner.clone(), id.clone(), ConversationStart::default())
                .await
                .unwrap(),
            Creation::Created(_)
        ));
        let desktop = control
            .create_device(
                owner.clone(),
                "DESKTOP-B5O1P43".into(),
                RunnerPlatform::Win32,
                TokenHash::of("desktop"),
            )
            .await
            .unwrap()
            .id;
        let os = OperatingSystem {
            name: "Windows 11 Pro (build 26100)".into(),
            distribution: None,
            arch: "x86_64".into(),
        };
        control
            .set_device_runner(desktop.clone(), os, "0".into(), 1, false)
            .await
            .unwrap();
        testing::execute(
            &control,
            r"UPDATE conversations SET target_kind = 'device', target_device_id = ?2, target_path = 'C:\Users\ana'
             WHERE id = ?1",
            vec![ID.to_owned(), desktop.to_string()],
        )
        .await;
        let pool = ShardPlacement::Inline;
        let pool = ShardPool::start(pool, services).await.unwrap();
        let block = pool
            .shards()
            .of(&owner)
            .call(move |shard, _| async move { shard.execution_context(&id, &[]).await })
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            block.lines().nth(1).unwrap(),
            r"Primary host: DESKTOP-B5O1P43, Windows 11 Pro (build 26100) (x86_64). Shells start in C:\Users\ana. Files only you need (screenshots, probe scripts) go in $DEMI_SCRATCH, which shows as no change and lasts as long as the conversation. Standard utilities (sed, grep, cp, ls, find, …) are GNU's on every host. Shells are bash (Demi's own, not Git Bash or WSL); Windows programs such as cmd, powershell and git run by name. Write paths as C:/Users/…, not /c/Users/…; quote a path with backslashes."
        );
    }
}
