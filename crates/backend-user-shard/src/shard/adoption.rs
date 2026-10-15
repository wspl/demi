//! Runner adoption (`runner.md` § Connection and identity): the shard of a
//! device's owner takes the socket the edge accepted, binds it to the device
//! with the connection's policy, and serves it until either end closes it;
//! the revocation that ends a device's connection for good; and a device's
//! new name.

use std::rc::Rc;

use axum::extract::ws::{Message, WebSocket};
use demi_backend_database::StorageError;
use demi_backend_database::devices::{DeviceChange, DeviceRecord, DeviceRemoval};
use demi_backend_host_access::target::cloud_home;
use demi_backend_page_sync::Part;
use demi_backend_plugins::Lost;
use demi_backend_remote_host::{Hello, Link, LinkOptions, host_identity};
use demi_backend_runners::devices::{DeviceRecorder, Serving, Updating, send};
use demi_backend_runners::native::system_targets;
use demi_runner_protocol::wire::{HelloErrorCode, Inbound, KeptJob, RunnerInfo};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::devices::DeviceDto;
use demi_web_api_protocol::ids::DeviceId;
use tokio::sync::oneshot;
use tokio_util::task::AbortOnDropHandle;

use super::Shard;
use super::policy::ShardPolicy;

/// Why a connection that did not answer its ping ended when a new one of
/// its device arrived.
const REPLACED: &str = "replaced by a new connection of the device's runner";

/// What a runner's hello says of the jobs it keeps (`runner.md` § Command
/// lifetime).
pub struct KeptJobs {
    /// The number this start of the runner drew.
    pub instance: u64,
    pub jobs: Vec<KeptJob>,
}

impl Shard {
    /// Takes the socket of a runner that presented `device`'s token.
    pub async fn adopt_runner(
        self: Rc<Self>,
        device: DeviceRecord,
        runner: RunnerInfo,
        kept: KeptJobs,
        mut socket: WebSocket,
    ) {
        #[cfg(feature = "testing")]
        self.services()
            .hellos
            .pass(crate::holds::HelloStep::Bind)
            .await;
        // The update the runner was sent to is over, whatever this hello
        // comes to.
        if self.devices().hello(&device.id) {
            self.mark(Part::Devices);
        }
        // The jobs the backend recorded on the device, which a backend that
        // started again takes up (`sessions-and-targets.md` § Recovery and
        // persistence).
        let recorded = match self.services().control.running_jobs(device.id.clone()).await {
            Ok(recorded) => recorded,
            Err(error) => {
                // Without them a restarted backend stops the device's jobs
                // as ones it has no command for, and their commands stay
                // running in their conversations' records.
                tracing::warn!(device = %device.id, error = &error as &dyn std::error::Error, "the device's running jobs could not be read");
                Vec::new()
            }
        };
        // The user's conversations, archived ones included, whose scratch
        // directories the runner keeps (`runner.md` § Scratch directory).
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
                tracing::warn!(device = %device.id, error = &error as &dyn std::error::Error, "the user's conversations could not be read; the runner connects again");
                return;
            }
        };
        // A held connection that answers its ping keeps the device; one
        // that does not is the same runner's, lost without a close, and
        // gives way.
        loop {
            self.devices().settled(&device.id).await;
            // From this check until the link is published nothing awaits, so
            // two runners of one device never both become online.
            let Some(held) = self.devices().link(&device.id) else {
                break;
            };
            if held.answers(self.services().runners.probe).await {
                tracing::warn!(
                    device = %device.id,
                    "runner hello refused (already_connected): device {} already has a live connection [{}, {}]",
                    device.id,
                    runner.name,
                    runner.platform
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
                device = %device.id,
                "the device's connection did not answer its ping and gives way to a new one [{}, {}]",
                runner.name,
                runner.platform
            );
            held.disconnect(REPLACED);
        }
        let welcome = Inbound::HelloOk {
            device_id: device.id.to_string(),
            device_name: device.name.clone(),
            conversations,
        };
        // A runner that started anew kept nothing of its packages' state
        // (`native-runtime.md` § Messages to the plugin).
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
            recorded: recorded.iter().map(|(job, _)| job.clone()).collect(),
        };
        // A closing shard takes no runner; dropping the socket closes it.
        let Some((serving, reported)) = self.bind(&device.id, runner, hello) else {
            return;
        };
        self.take_up(recorded.into_iter().map(|(_, conversation)| conversation));
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
        reported.await;
        // A runner that went away before its welcome ends its connection at
        // once, as the connection reads its socket. What the connection
        // queued meanwhile leaves after the welcome.
        let _ = send(&mut socket, &welcome).await;
        serving.serve(socket).await;
    }

    /// Takes the socket of a runner a claim just paired with `device`, and
    /// answers the device as it is once the runner is bound.
    pub async fn adopt_claimed(
        self: Rc<Self>,
        device: DeviceRecord,
        runner: RunnerInfo,
        instance: u64,
        socket: WebSocket,
        bound: oneshot::Sender<DeviceDto>,
    ) {
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
        reported.await;
        // The claim that waits for this answer may have gone; the runner is
        // paired all the same.
        let _ = bound.send(
            self.devices()
                .dto(device, self.services().public_url.get()),
        );
        serving.serve(socket).await;
    }

    /// Publishes a new connection of the device under the shard's policy,
    /// with what records its runner's hello: its system and release, and
    /// the advance of each execution context the report changes
    /// (`sessions-and-targets.md` § Resolve a target). None once the shard
    /// is closing, whose close ends every connection it published before.
    fn bind(
        self: &Rc<Self>,
        device: &DeviceId,
        runner: RunnerInfo,
        hello: Hello,
    ) -> Option<(Serving, impl Future<Output = ()> + 'static)> {
        if self.is_closing() {
            return None;
        }
        let hello_instance = hello.instance;
        // The home the device's execution contexts named until this report:
        // the Cloud's is `/home/demi` before its runner reported one.
        let before = self.devices().home(device);
        let named = if hello.managed {
            Some(cloud_home(before.as_deref()).to_owned())
        } else {
            before
        };
        let home_changed = named.as_deref() != Some(runner.identity.home_dir.as_str());
        // A Host of a target not stored for yet has its package executables
        // stored ahead of its first command (`native-runtime.md` § Publish
        // packages, then source artifacts on demand).
        let native = &self.services().native;
        match &runner.native_target {
            Some(target) => native.foresee([target.as_str()]),
            None => native.foresee(system_targets(runner.platform, Some(&runner.os.arch))),
        }
        let (link, driver) = Link::new(LinkOptions {
            device: device.to_string(),
            identity: host_identity(&runner),
            kernel: runner.kernel.clone(),
            shell: runner.shell.clone(),
            pipes: self.pipes().clone(),
            policy: Rc::new(ShardPolicy::new(self, device.clone())),
            ping: self.services().runners.ping,
            jobs: self.devices().jobs(device),
            hello,
        });
        let seen = DeviceRecorder::new(
            self.services().control.clone(),
            self.services().sync.of(self.user()),
        );
        let serving = self
            .devices()
            .bind(device, link, driver, seen.clone(), runner.installation);
        // What its conversations keep of its files is searched again on the
        // new connection.
        self.project_files().online(device);
        let device = device.clone();
        let reported = async move {
            seen.hello(device, runner.os, runner.version, hello_instance, home_changed)
                .await;
        };
        Some((serving, reported))
    }

    /// Restores the trees of the conversations whose commands ran on the
    /// user's Cloud `device`, which does not run as the backend starts: an
    /// upgrade stopped it. Each tree's agent takes its commands up, which
    /// starts the Cloud again, whose runner's hello then says the commands
    /// were lost, to the upgrade when the release changed
    /// (`sessions-and-targets.md` § Recovery and persistence).
    pub async fn take_up_stopped_cloud(self: Rc<Self>, device: DeviceId) {
        match self.services().control.running_jobs(device.clone()).await {
            Ok(recorded) => self.take_up(recorded.into_iter().map(|(_, conversation)| conversation)),
            Err(error) => {
                tracing::warn!(device = %device, error = &error as &dyn std::error::Error, "the stopped Cloud's running jobs could not be read");
            }
        }
    }

    /// Restores the tree of each of `conversations`, whose commands ran on a
    /// device whose runner just connected, as an `open` does, so the agent
    /// that ran each command takes it up again and hears of its end
    /// (`runtime.md` § Command reports). A tree that is live already is
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
                #[cfg(feature = "testing")]
                shard
                    .services()
                    .hellos
                    .pass(crate::holds::HelloStep::TakeUp)
                    .await;
                if shard.is_closing() {
                    return;
                }
                if let Err(error) = shard.restore_tree(&conversation).await {
                    tracing::warn!(%conversation, "the tree whose commands ran on the device was not restored: {error}");
                }
            });
        }
    }

    /// Shows `device` as updating: its runner was just sent to the
    /// backend's runner release (`runner.md` § Runner updates), from the
    /// first such 409 until its next hello or the window of the runners'
    /// tuning ends; a retry's 409 changes nothing.
    pub fn runner_updating(self: &Rc<Self>, device: &DeviceId) {
        let until = tokio::time::Instant::now() + self.services().runners.updating;
        let shard = Rc::downgrade(self);
        let began = self.devices().begin_update(device, || {
            // The device's pages see the window end; the slot's drop of the
            // window stops this first, as the next hello does.
            let expiry = tokio::task::spawn_local(async move {
                tokio::time::sleep_until(until).await;
                if let Some(shard) = shard.upgrade() {
                    shard.mark(Part::Devices);
                }
            });
            Updating::new(until, AbortOnDropHandle::new(expiry))
        });
        if began {
            self.mark(Part::Devices);
        }
    }

    /// Revokes a device, which nothing refuses: its row goes with its
    /// attachments and its workspaces, whose conversations stay outside any
    /// workspace, and its runner hears that it was revoked and removes
    /// itself. The user's revocation and the
    /// runner's own request both come here.
    pub async fn revoke_device(&self, device: DeviceId) -> Result<DeviceRemoval, StorageError> {
        let removal = self
            .services()
            .control
            .delete_device(device.clone())
            .await?;
        self.mark(Part::Devices);
        if !removal.workspaces.is_empty() {
            self.mark(Part::Workspaces);
        }
        for conversation in &removal.conversations {
            self.mark(Part::Conversation(conversation.clone()));
        }
        let projects = removal
            .workspaces
            .iter()
            .map(|workspace| workspace.name.clone())
            .collect();
        self.devices().revoke(&device, projects);
        Ok(removal)
    }

    /// Ends the connection of every device of the user, the Cloud included:
    /// the account was disabled, and its runners are refused until it is
    /// enabled again (`product.md` § User system). Their runners connect
    /// again by themselves and are turned away without a word.
    pub async fn disconnect_runners(&self, reason: &str) -> Result<(), StorageError> {
        let control = &self.services().control;
        let mut devices = control.paired_devices(self.user().clone()).await?;
        devices.extend(control.managed_device(self.user().clone()).await?);
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

#[cfg(test)]
impl Shard {
    /// Connects `device` through a link nothing serves, whose runner works
    /// in `home`, for a test that needs the device online; the device stays
    /// online while the answer is held.
    pub(crate) fn connect_for_tests(
        self: &Rc<Self>,
        device: &DeviceId,
        home: &str,
    ) -> demi_backend_remote_host::LinkDriver {
        use demi_runner_protocol::wire::{HostIdentity, OperatingSystem, RunnerPlatform};
        let runner = RunnerInfo {
            name: "test".into(),
            platform: RunnerPlatform::Darwin,
            os: OperatingSystem {
                name: "macOS 26.5".into(),
                distribution: None,
                arch: "aarch64".into(),
            },
            version: "0".into(),
            native_target: None,
            identity: HostIdentity {
                uid: 501,
                gid: 20,
                hostname: "test".into(),
                home_dir: home.into(),
            },
            kernel: None,
            shell: None,
            managed: None,
            installation: None,
        };
        let hello = Hello {
            instance: 0,
            release: runner.version.clone(),
            jobs: Vec::new(),
            last_release: None,
            last_instance: None,
            managed: false,
            recorded: Default::default(),
        };
        let (serving, reported) = self.bind(device, runner, hello).expect("an open shard binds");
        self.tasks().spawn_local(reported);
        serving.into_driver()
    }

    /// Connects `device` through a runner the test plays, working in `home`:
    /// it answers each ping, and each conversation release once
    /// `on_release` ran for the released conversation, so what that reads
    /// is the state before the release was answered. The answer says what
    /// the runner says of its own accord.
    pub(crate) fn play_runner_for_tests(
        self: &Rc<Self>,
        device: &DeviceId,
        home: &str,
        on_release: impl Fn(
            demi_web_api_protocol::ids::ConversationId,
        ) -> futures_util::future::LocalBoxFuture<'static, ()>
        + 'static,
    ) -> PlayedRunner {
        use demi_runner_protocol::wire::{self, Outbound};
        let driver = self.connect_for_tests(device, home);
        let (answers, answered) = tokio::sync::mpsc::channel::<Result<Vec<u8>, demi_backend_remote_host::SocketEnd>>(8);
        let (frames, mut sent) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
        let incoming = futures_util::stream::unfold(answered, |mut answered| async move {
            answered.recv().await.map(|frame| (frame, answered))
        });
        let outgoing = futures_util::sink::unfold(frames, |frames, frame: Vec<u8>| async move {
            frames
                .send(frame)
                .await
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(frames)
        });
        tokio::task::spawn_local(driver.serve(incoming, outgoing));
        let played = PlayedRunner(answers.clone());
        tokio::task::spawn_local(async move {
            while let Some(frame) = sent.recv().await {
                let answer = match wire::decode::<Inbound>(&frame) {
                    Ok(Inbound::Ping {}) => Outbound::Pong { jobs: 0 },
                    Ok(Inbound::ConversationRelease {
                        id,
                        conversation_id,
                    }) => {
                        let conversation =
                            demi_web_api_protocol::ids::ConversationId::try_from(conversation_id)
                                .expect("a release names a conversation");
                        on_release(conversation).await;
                        Outbound::ConversationReleased { id, error: None }
                    }
                    _ => continue,
                };
                let answer = wire::encode(&answer).expect("the test runner's answers encode");
                // The link ends with the test.
                let _ = answers.send(Ok(answer.into_bytes())).await;
            }
        });
        played
    }
}

/// A runner a test plays, which says what the test has it say.
#[cfg(test)]
pub(crate) struct PlayedRunner(tokio::sync::mpsc::Sender<Result<Vec<u8>, demi_backend_remote_host::SocketEnd>>);

#[cfg(test)]
impl PlayedRunner {
    pub(crate) async fn say(&self, message: &demi_runner_protocol::wire::Outbound) {
        let frame = demi_runner_protocol::wire::encode(message).expect("the test runner's messages encode");
        self.0
            .send(Ok(frame.into_bytes()))
            .await
            .expect("the link hears the test runner");
    }
}
