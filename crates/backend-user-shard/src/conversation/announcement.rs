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
    AttachedHostRecord, ConversationRecord, ExecutionTarget, TargetSwitch,
};
use demi_agent_session::left_by_offline_host;
use demi_backend_database::devices::{CLOUD_NAME, DeviceRecord};
use demi_backend_host_access::root_of;
use demi_runner_protocol::wire::RunnerPlatform;
use demi_web_api_protocol::ids::ConversationId;

use crate::shard::Shard;

/// What opens every context block, before the revision it describes.
const CONTEXT: &str = "[Execution context ";

/// The line that opens a switch's announcement.
const SWITCHED: &str = "[Execution target switched]";

/// What every block says of a job's standard utilities (`runner.md`
/// § Standard utilities).
const UTILITIES: &str = "Standard utilities (sed, grep, cp, ls, find, …) are GNU's on every host";

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
        let switch = control.last_switch(record.id.clone()).await?;
        let announced = match &switch {
            Some(switch) => {
                let description = format!(
                    "Previous target: {}. Current target: {}. New shells start in {}.",
                    self.describe(&switch.from).await?,
                    self.describe(&switch.to).await?,
                    switch.to.path()
                );
                // The node's latest announcement of a switch tells whether
                // it saw this one.
                let observed = seen
                    .iter()
                    .rev()
                    .find(|text| text.contains(SWITCHED))
                    .is_some_and(|text| text.contains(&description));
                (!observed).then(|| switch_lines(switch, description, &attached))
            }
            None => None,
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

    /// The primary Host as the model reads it: its name, its operating
    /// system with its architecture as its runner last reported them, the
    /// directory its shells start in, and that its standard utilities are
    /// GNU's, with where a Mac's own are and what a Windows Host's shell
    /// is. A Host whose runner never connected, such as a Cloud not made
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
            "Primary host: {host}. Shells start in {}. {utilities}",
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
    fn attached_hosts_line(&self, attached: &[AttachedHostRecord]) -> String {
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

/// How an attached host's entry in a block's line of attached hosts opens:
/// its name, which stays while it is attached.
fn entry(host: &AttachedHostRecord) -> String {
    format!("\"{}\" (", host.name)
}

/// Whether the hosts `attached` now are others than the ones the block
/// `seen`, a node's latest, listed: a host joined or left. A host's
/// connection or directory is no change of the attached hosts.
fn attached_changed(seen: &str, attached: &[AttachedHostRecord]) -> bool {
    let Some(listed) = seen.lines().find(|line| line.starts_with(ATTACHED)) else {
        return true;
    };
    listed.matches(STARTS_IN).count() != attached.len()
        || attached.iter().any(|host| !listed.contains(&entry(host)))
}

/// A switch's announcement: what changed, that no file moved, and how to
/// reach the device left behind.
fn switch_lines(
    switch: &TargetSwitch,
    description: String,
    attached: &[AttachedHostRecord],
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
    use demi_backend_database::conversation_index::{ConversationStart, Creation};
    use demi_runner_protocol::wire::{OperatingSystem, RunnerPlatform};
    use demi_web_api_protocol::ids::ConversationId;

    use crate::services::Services;
    use crate::shard::{ShardPlacement, ShardPool};

    const ID: &str = "4f0c2a8e-1d3b-4e6f-9a7c-2b5d8e1f3a60";

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
            r"Primary host: DESKTOP-B5O1P43, Windows 11 Pro (build 26100) (x86_64). Shells start in C:\Users\ana. Standard utilities (sed, grep, cp, ls, find, …) are GNU's on every host. Shells are bash (Demi's own, not Git Bash or WSL); Windows programs such as cmd, powershell and git run by name. Write paths as C:/Users/…, not /c/Users/…; quote a path with backslashes."
        );
    }
}
