//! The context block a node reads before its next request once the
//! conversation's execution context changed (`sessions-and-targets.md`
//! § Switch the main target): after a switch, the departed and current
//! targets, that no files moved, and the departed device under the name it
//! stays attached as; after a change of the attached hosts, that change.
//! Either block ends with the attached hosts as they now stand, and a block
//! after a reset of the user's Cloud says so first. What a node saw is the
//! context blocks of its own transcript: each block names the revision it
//! describes, and a switch and a reset are each announced to a node once.

use demi_web_api::ids::ConversationId;

use super::target::{ExecutionTarget, TargetSwitch};
use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::conversation_index::AttachedHostRecord;

/// The line that opens a switch's announcement.
const SWITCHED: &str = "[Execution target switched]";

/// What a node learns once the user's Cloud was reset.
const CLOUD_RESET: &str = "Cloud was reset: system packages and configuration were rebuilt from the base image. Files under /home remain. Running processes, temporary files, and previous shell state are gone; check the environment before continuing.";

impl Shard {
    /// The context block for a node of the conversation `id` whose
    /// transcript holds the context texts `seen`, oldest first; none when
    /// the node saw the current revision, or nothing ever changed.
    pub(crate) async fn execution_context(
        &self,
        id: &ConversationId,
        seen: &[&str],
    ) -> Result<Option<String>, StorageError> {
        let control = &self.services().control;
        let Some(record) = control.conversation(id.clone()).await? else {
            return Ok(None);
        };
        if record.context_version == 0 {
            return Ok(None);
        }
        let marker = format!("[Execution context {}]", record.context_version);
        if seen.iter().any(|text| text.contains(&marker)) {
            return Ok(None);
        }
        let attached = control.attached_hosts(record.id.clone()).await?;
        let mut lines = vec![marker];
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
        match announced {
            Some(announcement) => lines.extend(announcement),
            None => lines.push("[Attached hosts changed]".into()),
        }
        lines.push(self.attached_hosts_line(&attached));
        Ok(Some(lines.join("\n")))
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
            ExecutionTarget::Workspace { workspace_id, path, .. } => {
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
                let state = if self.devices().online(&host.device) { "online" } else { "offline" };
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
fn switch_lines(switch: &TargetSwitch, description: String, attached: &[AttachedHostRecord]) -> Vec<String> {
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
        ExecutionTarget::Workspace { device_id: before, .. },
        ExecutionTarget::Workspace { device_id: after, .. },
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
    use demi_web_api::conversations::ConversationTarget;
    use demi_web_api::ids::{DeviceId, UserId};

    use super::*;
    use demi_runner_protocol::wire::RunnerPlatform;
    use crate::auth::sessions::TokenHash;
    use crate::backend::Services;
    use crate::shard::{ShardPlacement, ShardPool};
    use crate::storage::control::testing;
    use crate::storage::conversation_index::{Creation, RecordChange, SwitchEnds};

    const ID: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";

    #[tokio::test(flavor = "local")]
    async fn a_node_is_told_of_a_switch_once_and_then_of_the_attached_hosts_that_changed() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control.create_conversation(owner.clone(), id.clone()).await.unwrap(),
            Creation::Created(_)
        ));
        let device = {
            let control = control.clone();
            let owner = owner.clone();
            move |name: &'static str| {
                let control = control.clone();
                let owner = owner.clone();
                async move {
                control
                    .create_device(owner, name.into(), RunnerPlatform::Linux, TokenHash::of(name))
                    .await
                    .unwrap()
                    .id
                }
            }
        };
        let laptop = device("laptop").await;
        let ci = device("ci").await;
        let on = |device: &DeviceId, path: &str| ConversationTarget::Device {
            device_id: device.clone(),
            path: path.into(),
        };
        let resolved = |device: &DeviceId, path: &str| ExecutionTarget::Device {
            device_id: device.clone(),
            path: path.into(),
        };
        let first = control.conversation(id.clone()).await.unwrap().unwrap().target;
        let switched = control
            .switch_conversation_target(
                id.clone(),
                first,
                on(&laptop, "/work"),
                TargetSwitch {
                    from: resolved(&ci, "/build"),
                    to: resolved(&laptop, "/work"),
                },
                SwitchEnds {
                    departed: Some((ci.clone(), "/build".into())),
                    arriving: Some(laptop.clone()),
                },
            )
            .await
            .unwrap();
        assert!(switched);
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let announced = shard.execution_context(&id, &[]).await.unwrap().unwrap();
                assert!(announced.starts_with("[Execution context 1]\n[Execution target switched]\n"), "{announced}");
                assert!(
                    announced.contains(&format!(
                        "Previous target: the machine \"ci\" (host {ci}). Current target: the machine \"laptop\" (host {laptop}). New shells start in /work."
                    )),
                    "{announced}"
                );
                assert!(announced.contains("stays attached as \"ci\""), "{announced}");
                assert!(announced.ends_with("\"ci\" (offline, shells start in /build). `demi host shell --host <name> <script>` runs a shell string on one; `demi host list` shows every host this conversation can reach."), "{announced}");
                // A node that saw it sees nothing more.
                assert_eq!(shard.execution_context(&id, &[&announced]).await.unwrap(), None);

                // A host attached later is news, and the switch is not.
                let spare = device("spare").await;
                let attach = RecordChange::Attach(AttachedHostRecord {
                    device: spare,
                    name: "spare".into(),
                    cwd: None,
                });
                shard.services().control.change_conversation(id.clone(), attach).await.unwrap();
                let changed = shard.execution_context(&id, &[&announced]).await.unwrap().unwrap();
                assert!(changed.starts_with("[Execution context 2]\n[Attached hosts changed]\nAttached hosts: "), "{changed}");
                assert!(changed.contains("\"spare\" (offline, shells start in its home directory)"), "{changed}");
                // A node that never saw the switch, such as a new child, is
                // told of it.
                let fresh = shard.execution_context(&id, &[]).await.unwrap().unwrap();
                assert!(fresh.contains(SWITCHED), "{fresh}");
            })
            .await
            .unwrap();
    }
}
