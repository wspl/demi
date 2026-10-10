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
        // A node that read an earlier revision learns what changed since;
        // one reading its first block learns the context as it stands.
        let earlier = seen.iter().any(|text| text.contains(CONTEXT));
        match announced {
            Some(announcement) => lines.extend(announcement),
            None if earlier => lines.push("[Attached hosts changed]".into()),
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
    /// GNU's, with where a Mac's own are. A Host whose runner never
    /// connected, such as a Cloud not made yet, is named without its system.
    async fn primary_host_line(&self, target: &ExecutionTarget) -> Result<String, StorageError> {
        let (device, name) = self.primary_host(target).await?;
        let utilities = match device.as_ref().map(|record| &record.platform) {
            Some(RunnerPlatform::Darwin) => format!("{UTILITIES}; {MACOS_UTILITIES}."),
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
            return format!("Attached hosts: none. {LIST}");
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
                format!("\"{}\" ({state}, shells start in {directory})", host.name)
            })
            .collect();
        format!(
            "Attached hosts: {}. `demi host shell --host <name> <script>` runs a shell string on one; {LIST}",
            entries.join(", ")
        )
    }
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
