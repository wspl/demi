//! Runner adoption (`crates/runner-protocol/src/wire/mod.rs`).
//!
//! The shard of a device's owner:
//!
//! - takes the socket the edge accepted, binds it to the device with the
//!   connection's policy, and serves it until either end closes it;
//! - revokes a device, which ends its connection for good;
//! - renames a device.

use std::collections::HashMap;
use std::rc::Rc;

use axum::extract::ws::Message;
use demi_backend_database::StorageError;
use demi_backend_database::devices::{DeviceChange, DeviceRecord, DeviceRemoval, FirstHome};
use demi_backend_page_sync::Part;
use demi_plugin_host::Lost;
use demi_runner_client::{Beginning, Hello, Link, LinkOptions, RemoteHost, host_identity};
use demi_host_interface::{HostErrorKind, HostFs as _};
use demi_backend_runners::devices::{DeviceRecorder, Download, Reported, RunnerSocket, Serving, Updating, send};
use demi_runner_protocol::values::DeviceToken;
use demi_runner_protocol::wire::{HelloErrorCode, Inbound, KeptJob, Resume, RunnerInfo};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::devices::DeviceDto;
use demi_web_api_protocol::ids::DeviceId;
use std::sync::Arc;

use demi_backend_runners::prefetch::NextRelease;
use demi_runner_protocol::release::PrefetchState;
use tokio::sync::{oneshot, watch};
use tokio_util::task::AbortOnDropHandle;

use super::Shard;
use super::policy::ShardPolicy;

/// Why a connection that did not answer its ping ended when a new one of
/// its device arrived.
const REPLACED: &str = "replaced by a new connection of the device's runner";

/// How often a device's pages hear of its download's progress, at most
/// (`crates/runner/src/update.rs`).
const DOWNLOAD_ANNOUNCED_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

/// What a runner's hello says of the jobs it keeps (`crates/runner/src/connection/kept.rs`).
pub struct KeptJobs {
    /// The number this start of the runner drew.
    pub instance: u64,
    pub jobs: Vec<KeptJob>,
    /// The session the runner carries on, if it had one.
    pub resume: Option<Resume>,
}

impl Shard {
    /// Takes the socket of a runner that presented `device`'s token.
    pub async fn adopt_runner(
        self: Rc<Self>,
        device: DeviceRecord,
        runner: RunnerInfo,
        kept: KeptJobs,
        mut socket: RunnerSocket,
    ) {
        // The update the runner was sent to is over, whatever this hello
        // comes to.
        if self.devices().hello(&device.id) {
            self.mark(Part::Devices);
        }
        // The jobs the backend recorded on the device, which a backend that
        // started again takes up (`crates/runner-client/src/device_jobs.rs`).
        let recorded = match self.services().control.running_jobs(device.id.clone()).await {
            Ok(recorded) => recorded,
            Err(error) => {
                // Without them a restarted backend stops the device's jobs
                // as ones it has no command for, and their commands stay
                // running in their conversations' records.
                tracing::warn!(event = "runner_adoption.read_failed", host_id = %device.id, what = "running_jobs", error = &error as &dyn std::error::Error, "the device's running jobs could not be read");
                Vec::new()
            }
        };
        // The user's conversations, archived ones included, whose scratch
        // directories the runner keeps, and which of them are archived, whose
        // recordings it removes (`crates/runner-jobs/src/scratch.rs`).
        // Without them the runner would remove every one, so a runner whose
        // welcome cannot name them is dropped and connects again.
        let conversations = match self
            .services()
            .control
            .conversation_order(self.user().clone())
            .await
        {
            Ok(conversations) => conversations.iter().map(ToString::to_string).collect(),
            Err(error) => {
                tracing::warn!(event = "runner_adoption.read_failed", host_id = %device.id, what = "conversations", outcome = "runner_reconnects", error = &error as &dyn std::error::Error, "the user's conversations could not be read");
                return;
            }
        };
        let archived = match self.services().control.conversations(self.user().clone(), true).await {
            Ok(archived) => archived.iter().map(|record| record.id.to_string()).collect(),
            Err(error) => {
                tracing::warn!(event = "runner_adoption.read_failed", host_id = %device.id, what = "archived_conversations", outcome = "runner_reconnects", error = &error as &dyn std::error::Error, "the user's archived conversations could not be read");
                return;
            }
        };
        // What the device is to hold services for, and the packages it is to
        // hold none of: it lets go of every other at once. Without them it
        // would release what is in use, so a runner whose welcome cannot name
        // them is dropped and connects again.
        let idle = self.idle_released();
        // Beside the conversations whose records name the device, those with a
        // live subagent there, which a move of the conversation left on it.
        let with_agents = self.conversations_with_agents_on(&device.id);
        let held = match self.services().control.conversations_on(self.user().clone(), device.id.clone()).await {
            Ok(held) => held
                .into_iter()
                .chain(with_agents)
                .filter(|id| !idle.contains(id))
                .map(|id| id.to_string())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect(),
            Err(error) => {
                tracing::warn!(event = "runner_adoption.read_failed", host_id = %device.id, what = "held_conversations", outcome = "runner_reconnects", error = &error as &dyn std::error::Error, "the conversations the device holds could not be read");
                return;
            }
        };
        let stopped = match self.plugins().stopped_packages().await {
            Ok(stopped) => stopped,
            Err(error) => {
                tracing::warn!(event = "runner_adoption.read_failed", host_id = %device.id, what = "stopped_packages", outcome = "runner_reconnects", error = &error as &dyn std::error::Error, "the packages of the plugins turned off could not be read");
                return;
            }
        };
        let session = self.devices().session(&device.id);
        // A held connection that answers its ping keeps the device; one
        // that does not is the same runner's, lost without a close, and
        // gives way. A runner that carries its session on is the same one,
        // which lost the connection a network left held, so the held one
        // gives way without a ping.
        loop {
            self.devices().settled(&device.id).await;
            // From this check until the link is published nothing awaits, so
            // two runners of one device never both become online.
            let Some(held) = self.devices().link(&device.id) else {
                break;
            };
            if session.resumes(kept.instance, kept.resume) {
                tracing::info!(
                    event = "link.replaced",
                    host_id = %device.id,
                    cause = "resumed",
                    runner_name = %runner.name,
                    platform = %runner.platform,
                    "the device's runner carries its session on over a new connection, which replaces the held one"
                );
                held.disconnect(REPLACED);
                continue;
            }
            if held.answers(self.services().runners.probe).await {
                tracing::warn!(
                    event = "link.refused",
                    host_id = %device.id,
                    cause = "already_connected",
                    runner_name = %runner.name,
                    platform = %runner.platform,
                    "a runner's hello was refused: the device has a live connection"
                );
                let refusal = Inbound::HelloError {
                    code: HelloErrorCode::AlreadyConnected,
                    reason: format!("device {} already has a live connection", device.id),
                };
                // A runner that went away needs no refusal.
                if send(&mut socket, &refusal).await.is_ok() {
                    let _ = socket.send(Message::Close(None)).await;
                }
                return;
            }
            tracing::info!(
                event = "link.replaced",
                host_id = %device.id,
                cause = "no_answer",
                runner_name = %runner.name,
                platform = %runner.platform,
                "the device's connection did not answer its ping and gives way to a new one"
            );
            held.disconnect(REPLACED);
        }
        // Nothing awaits from the last check of the held connection until the
        // link is published, so the session carries on, or begins, for the
        // connection that is published.
        let beginning = session.begin(kept.instance, kept.resume);
        tracing::info!(
            event = if matches!(beginning, Beginning::Resumed { .. }) { "link.resumed" } else { "link.session_started" },
            host_id = %device.id,
            received = beginning.received(),
            "a runner's connection took the link's session"
        );
        let welcome = Inbound::HelloOk {
            device_id: device.id.to_string(),
            device_name: device.name.clone(),
            conversations,
            archived,
            held,
            stopped,
            session: session.number(),
            received: beginning.received(),
        };
        // A runner that started anew kept nothing of its packages' state
        // (`crates/command-protocol/src/messages.rs`).
        let started_anew = device
            .runner_instance
            .is_some_and(|last| last != kept.instance);
        let hello = Hello {
            instance: kept.instance,
            release: runner.version.clone(),
            jobs: kept.jobs,
            last_release: device.runner_version.clone(),
            last_instance: device.runner_instance,
            managed: device.kind == DeviceKind::Managed,
            // A job whose start waits for the device was never sent: its
            // agent sends it once it takes it up.
            recorded: recorded
                .iter()
                .filter(|recorded| recorded.sent)
                .map(|recorded| recorded.job.clone())
                .collect(),
        };
        // A closing shard takes no runner; dropping the socket closes it.
        let Some((serving, reported)) = self.bind(&device.id, runner, hello) else {
            return;
        };
        self.take_up(recorded.into_iter().map(|recorded| recorded.conversation));
        if started_anew {
            let shard = self.clone();
            let device = device.id.clone();
            self.tasks().spawn_local(async move {
                shard.package_lost(&device, Lost::Every).await;
            });
        }
        // The report is recorded before the welcome: the work the
        // connection queued runs after it, so no answer of that work reaches
        // an agent whose next context block still names what the runner no
        // longer reports.
        if let Some(settle) = reported.await {
            self.tasks().spawn_local(settle);
        }
        // A runner that went away before its welcome ends its connection at
        // once, as the connection reads its socket. What the connection
        // queued meanwhile leaves after the welcome.
        let _ = send(&mut socket, &welcome).await;
        self.forwards_presence(&device.id, true).await;
        self.projects_presence(&device.id).await;
        serving.serve(socket).await;
        self.forwards_presence(&device.id, false).await;
    }

    /// Takes the socket of a runner a claim just paired with `device`, and
    /// answers the device as it is once the runner is bound.
    pub async fn adopt_claimed(
        self: Rc<Self>,
        device: DeviceRecord,
        runner: RunnerInfo,
        instance: u64,
        token: DeviceToken,
        mut socket: RunnerSocket,
        bound: oneshot::Sender<DeviceDto>,
    ) {
        // A device just paired has no session before this connection; the
        // runner's next hello resumes the one it begins now.
        let session = self.devices().session(&device.id);
        session.begin(instance, None);
        let claimed = Inbound::Claimed {
            device_token: token,
            device_name: device.name.clone(),
            session: session.number(),
        };
        // A runner lost on the way leaves the grant unanswered, and the claim
        // then deletes the device it made.
        if send(&mut socket, &claimed).await.is_err() {
            return;
        }
        // A closing shard takes no runner, and the claim that waits for the
        // answer deletes the device it made. A device just paired has no
        // jobs.
        let hello = Hello {
            instance,
            release: runner.version.clone(),
            jobs: Vec::new(),
            last_release: None,
            last_instance: None,
            managed: false,
            recorded: Default::default(),
        };
        let Some((serving, reported)) = self.bind(&device.id, runner, hello) else {
            return;
        };
        // A device just paired has nothing an earlier release left.
        if let Some(settle) = reported.await {
            self.tasks().spawn_local(settle);
        }
        // The claim that waits for this answer may have gone; the runner is
        // paired all the same.
        let id = device.id.clone();
        let _ = bound.send(
            self.devices()
                .dto(device, self.services().public_url.get()),
        );
        self.forwards_presence(&id, true).await;
        self.projects_presence(&id).await;
        serving.serve(socket).await;
        self.forwards_presence(&id, false).await;
    }

    /// Publishes a new connection of the device under the shard's policy, with
    /// what records its runner's hello: its system and release, and the advance
    /// of each execution context the report changes
    /// (`crates/backend-database/src/selection.rs`).
    ///
    /// The record answers what settles a paired device's first home, which its
    /// caller runs while the connection serves, since the runner resolves its
    /// directories (`crates/server/src/main.rs`).
    ///
    /// None once the shard is closing, whose close ends every connection it
    /// published before.
    fn bind(
        self: &Rc<Self>,
        device: &DeviceId,
        runner: RunnerInfo,
        hello: Hello,
    ) -> Option<(Serving, impl Future<Output = Option<impl Future<Output = ()> + 'static>> + 'static)> {
        if self.is_closing() {
            return None;
        }
        let hello_instance = hello.instance;
        let home = runner.identity.home_dir.clone();
        let (link, driver) = Link::new(LinkOptions {
            device: device.to_string(),
            identity: host_identity(&runner),
            kernel: runner.kernel.clone(),
            shell: runner.shell.clone(),
            memory: runner.memory,
            pipes: self.pipes().clone(),
            policy: Rc::new(ShardPolicy::new(self, device.clone())),
            ping: self.services().runners.ping,
            jobs: self.devices().jobs(device),
            session: self.devices().session(device),
            hello,
        });
        let seen = DeviceRecorder::new(
            self.services().control.clone(),
            self.services().sync.of(self.user()),
        );
        let serving = self
            .devices()
            .bind(
                device,
                link,
                driver,
                seen.clone(),
                Reported {
                    installation: runner.installation,
                    target: runner.native_target.clone(),
                },
            );
        // What its conversations keep of its files is searched again on the
        // new connection.
        self.project_files().online(device);
        let access = Some(serving.device_access());
        let device = device.clone();
        let reported = async move {
            let first = seen
                .hello(
                    device.clone(),
                    runner.os,
                    runner.version,
                    hello_instance,
                    home,
                    runner.autostart_unavailable,
                )
                .await?;
            Some(settle_first_home(access, seen, device, first))
        };
        Some((serving, reported))
    }

    /// Restores the trees of the conversations whose commands ran on the user's
    /// Cloud `device`, which does not run as the backend starts: an upgrade
    /// stopped it.
    ///
    /// Each tree's agent takes its commands up as last recorded, without waking
    /// the Cloud: they stay recorded running until its runner's next hello
    /// says they were lost, to the upgrade when the release changed
    /// (`crates/runner-client/src/device_jobs.rs`).
    pub async fn take_up_stopped_cloud(self: Rc<Self>, device: DeviceId) {
        match self.services().control.running_jobs(device.clone()).await {
            Ok(recorded) => self.take_up(recorded.into_iter().map(|recorded| recorded.conversation)),
            Err(error) => {
                tracing::warn!(event = "runner_adoption.read_failed", host_id = %device, what = "stopped_cloud_jobs", error = &error as &dyn std::error::Error, "the stopped Cloud's running jobs could not be read");
            }
        }
    }

    /// Restores the tree of each of `conversations`, whose commands ran on a
    /// device whose runner just connected, as an `open` does.
    ///
    /// The agent that ran each command takes it up again and hears of its end
    /// (`crates/agent-session/src/reports.rs`). A tree that is live already is
    /// left as it is.
    fn take_up(self: &Rc<Self>, conversations: impl Iterator<Item = ConversationId>) {
        let conversations: std::collections::BTreeSet<ConversationId> = conversations.collect();
        for conversation in conversations {
            // Counted before the open, so the close waits for it before the
            // agent shuts down.
            let opening = self.tree_openers().token();
            let shard = self.clone();
            self.tasks().spawn_local(async move {
                let _opening = opening;
                if shard.is_closing() {
                    return;
                }
                if let Err(error) = shard.restore_tree(&conversation).await {
                    tracing::warn!(event = "runner_adoption.tree_restore_failed", %conversation, %error, "the tree whose commands ran on the device was not restored");
                }
            });
        }
    }

    /// Shows `device` as updating: its runner was just sent to the backend's
    /// runner release (`crates/runner/src/update.rs`).
    ///
    /// It shows so from the first such 409 until its next hello, or until the
    /// window of the runners' tuning passes without a byte of its download served.
    /// A retry's 409 changes nothing.
    pub fn runner_updating(self: &Rc<Self>, device: &DeviceId) {
        let until = tokio::time::Instant::now() + self.services().runners.updating;
        let began = self
            .devices()
            .begin_update(device, || Updating::new(until, self.update_expiry(device)));
        if began {
            self.mark(Part::Devices);
        }
    }

    /// Announces the end of `device`'s update to its pages once its window
    /// passed, which each byte its download is served moves on; the slot's
    /// drop of the update stops it, as the next hello does.
    fn update_expiry(self: &Rc<Self>, device: &DeviceId) -> AbortOnDropHandle<()> {
        let shard = Rc::downgrade(self);
        let device = device.clone();
        AbortOnDropHandle::new(tokio::task::spawn_local(async move {
            loop {
                let Some(until) = shard
                    .upgrade()
                    .and_then(|shard| shard.devices().update_until(&device))
                else {
                    return;
                };
                if tokio::time::Instant::now() >= until {
                    if let Some(shard) = shard.upgrade() {
                        shard.devices().update_abandoned(&device);
                        shard.mark(Part::Devices);
                    }
                    return;
                }
                tokio::time::sleep_until(until).await;
            }
        }))
    }

    /// Counts what a download of a runner executable has served `device`,
    /// as `served` announces it until the download ends (`crates/runner/src/update.rs`): the device's state carries the progress, its
    /// pages hear of it at most once a second, and while it moves an
    /// update the device was sent to shows on.
    pub fn runner_downloading(self: &Rc<Self>, device: DeviceId, mut served: watch::Receiver<Download>) {
        let request = self.devices().download_request();
        let shard = Rc::downgrade(self);
        // A closing shard waits for its tasks, and the download may outlast it.
        let closing = self.closing.clone();
        self.tasks().spawn_local(async move {
            let started = std::time::Instant::now();
            loop {
                let changed = tokio::select! {
                    () = closing.cancelled() => return,
                    changed = served.changed() => changed,
                };
                let Some(shard) = shard.upgrade() else {
                    return;
                };
                let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                if changed.is_err() {
                    let download = *served.borrow();
                    tracing::info!(
                        event = "runner_download.ended",
                        host_id = %device,
                        bytes = download.served,
                        total = download.size,
                        ms,
                        "a runner executable's download ended"
                    );
                    shard.devices().download_ended(&device, request, ms);
                    shard.mark(Part::Devices);
                    return;
                }
                let download = *served.borrow_and_update();
                tracing::debug!(
                    event = "runner_update.progress",
                    host_id = %device,
                    bytes = download.served,
                    total = download.size,
                    ms,
                    "a runner executable's download moved"
                );
                let until = tokio::time::Instant::now() + shard.services().runners.updating;
                shard
                    .devices()
                    .download_moved(&device, request, download, until, || shard.update_expiry(&device));
                shard.mark(Part::Devices);
                drop(shard);
                // What is served meanwhile waits for the next announcement.
                tokio::select! {
                    () = closing.cancelled() => return,
                    () = tokio::time::sleep(DOWNLOAD_ANNOUNCED_EVERY) => {}
                }
            }
        });
    }

    /// Has the runner of each of the user's paired devices that is
    /// connected download `next` ahead while it serves (`crates/runner/src/update.rs`): each is counted at once, and `next` hears how its
    /// download went, a connection that ends failing it.
    pub async fn prefetch(self: &Rc<Self>, next: Arc<NextRelease>) {
        let devices = match self.services().control.paired_devices(self.user().clone()).await {
            Ok(devices) => devices,
            Err(error) => {
                // Their runners download after the switch.
                tracing::warn!(event = "runner_release.devices_list_failed", error = &error as &dyn std::error::Error, "the devices to download the next runner cannot be listed");
                return;
            }
        };
        for device in devices {
            let Some(link) = self.devices().link(&device.id) else {
                continue;
            };
            next.begin(&device.id, &device.name);
            let target = self.devices().target(&device.id);
            let artifact = target
                .as_deref()
                .and_then(|target| next.release.targets.get(target))
                .cloned();
            let programs = next.programs(&self.services().public_url);
            let next = next.clone();
            self.tasks().spawn_local(async move {
                let state = match artifact {
                    None => PrefetchState::Failed {
                        reason: format!(
                            "the release has no runner for {}",
                            target.as_deref().unwrap_or("the device's system")
                        ),
                    },
                    Some(artifact) => match link
                        .prefetch(&next.release.release, &artifact.sha256, artifact.size, programs)
                        .await
                    {
                        Ok(()) => PrefetchState::Done,
                        Err(error) => PrefetchState::Failed {
                            reason: error.to_string(),
                        },
                    },
                };
                next.end(&device.id, &device.name, state);
            });
        }
    }

    /// Revokes a device, which nothing refuses.
    ///
    /// Its row goes with its attachments and its projects, whose conversations
    /// stay outside any project, and its runner hears that it was revoked and
    /// removes itself. The user's revocation and the runner's own request both
    /// come here.
    pub async fn revoke_device(self: &Rc<Self>, device: DeviceId) -> Result<DeviceRemoval, StorageError> {
        let removal = self
            .services()
            .control
            .delete_device(device.clone())
            .await?;
        self.mark(Part::Devices);
        if !removal.projects.is_empty() {
            self.mark(Part::Projects);
        }
        for conversation in &removal.conversations {
            self.mark(Part::Conversation(conversation.clone()));
        }
        let projects = removal
            .projects
            .iter()
            .map(|project| project.name.clone())
            .collect();
        self.devices().revoke(&device, projects);
        // The commands recorded on it that the backend no longer holds, as
        // after a restart, are taken up to be lost with it.
        self.take_up(removal.running.iter().cloned());
        // The forwards it was an end of went with it, and its exposed
        // ports.
        self.forwards_changed().await;
        self.mark(Part::ExposedPorts);
        Ok(removal)
    }

    /// Ends the connection of every device of the user, the Cloud included:
    /// the account was disabled, and its runners are refused until it is
    /// enabled again. Their runners connect again by themselves and are turned
    /// away without a word.
    pub async fn disconnect_runners(&self, reason: &str) -> Result<(), StorageError> {
        let control = &self.services().control;
        let mut devices = control.paired_devices(self.user().clone()).await?;
        devices.extend(control.clouds(self.user().clone()).await?);
        for device in devices {
            self.devices().disconnect(&device.id, reason);
        }
        Ok(())
    }

    /// Changes the user's paired device `device`, its name or whether pages
    /// may reach it directly, which every page of the user's sees; none once
    /// the device is gone. The conversations that reach it go by its new
    /// name, which their pages and their agents' next context block see.
    pub async fn change_device(
        &self,
        device: DeviceId,
        change: DeviceChange,
    ) -> Result<Option<DeviceDto>, StorageError> {
        let Some(changed) = self.services().control.change_device(device, change).await? else {
            return Ok(None);
        };
        self.mark(Part::Devices);
        for conversation in changed.conversations {
            self.mark(Part::Conversation(conversation));
        }
        // Its route decides whether its forwards' runners talk directly.
        self.forwards_changed().await;
        Ok(Some(
            self.devices()
                .dto(changed.device, self.services().public_url.get()),
        ))
    }

    /// The user's devices as the web app sees them.
    pub async fn device_list(&self) -> Result<Vec<DeviceDto>, StorageError> {
        self.devices()
            .device_list(
                &self.services().control,
                self.user(),
                self.services().public_url.get(),
            )
            .await
    }
}

/// Has the runner of `access` resolve each directory `first` names, then
/// records the device's first home (`crates/server/src/main.rs`). A
/// directory gone is kept as it is; a runner that went away leaves the home
/// unrecorded, which its next hello settles again.
async fn settle_first_home(access: Option<RemoteHost>, seen: DeviceRecorder, device: DeviceId, first: FirstHome) {
    let Some(access) = access else {
        return;
    };
    let mut resolved = HashMap::new();
    for directory in &first.directories {
        match access.realpath(directory).await {
            Ok(real) => {
                resolved.insert(directory.clone(), real);
            }
            Err(error) if matches!(error.kind, HostErrorKind::Failed { .. }) => {}
            Err(_) => return,
        }
    }
    seen.settle_first_home(device, first, resolved).await;
}
