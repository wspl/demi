//! Runner adoption (`runner.md` § Connection and identity): the shard of a
//! device's owner takes the socket the edge accepted, binds it to the device
//! with the connection's policy, and serves it until either end closes it;
//! and the revocation that ends a device's connection for good.

use std::rc::Rc;

use axum::extract::ws::{Message, WebSocket};
use demi_backend_database::StorageError;
use demi_backend_database::devices::{DeviceRecord, DeviceRemoval};
use demi_backend_page_sync::Part;
use demi_backend_remote_host::{Link, LinkOptions, host_identity};
use demi_backend_runners::devices::{DeviceRecorder, Serving, send};
use demi_host_interface::HostIdentity;
use demi_runner_protocol::wire::{HelloErrorCode, Inbound, RunnerInfo};
use demi_web_api_protocol::devices::DeviceDto;
use demi_web_api_protocol::ids::DeviceId;
use tokio::sync::oneshot;

use super::Shard;
use super::policy::ShardPolicy;

/// Why a connection that did not answer its ping ended when a new one of
/// its device arrived.
const REPLACED: &str = "replaced by a new connection of the device's runner";

impl Shard {
    /// Takes the socket of a runner that presented `device`'s token.
    pub async fn adopt_runner(
        self: Rc<Self>,
        device: DeviceRecord,
        runner: RunnerInfo,
        mut socket: WebSocket,
    ) {
        #[cfg(feature = "testing")]
        self.services()
            .hellos
            .pass(crate::holds::HelloStep::Bind)
            .await;
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
        let identity = host_identity(&runner.identity);
        let welcome = Inbound::HelloOk {
            device_id: device.id.to_string(),
            device_name: device.name.clone(),
        };
        // A closing shard takes no runner; dropping the socket closes it.
        let Some(serving) = self.bind(&device.id, identity) else {
            return;
        };
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
        socket: WebSocket,
        bound: oneshot::Sender<DeviceDto>,
    ) {
        // A closing shard takes no runner, and the claim that waits for the
        // answer deletes the device it made.
        let Some(serving) = self.bind(&device.id, host_identity(&runner.identity)) else {
            return;
        };
        // The claim that waits for this answer may have gone; the runner is
        // paired all the same.
        let _ = bound.send(self.devices().dto(device));
        serving.serve(socket).await;
    }

    /// Publishes a new connection of the device under the shard's policy,
    /// and records that the device was seen. None once the shard is
    /// closing, whose close ends every connection it published before.
    fn bind(self: &Rc<Self>, device: &DeviceId, identity: HostIdentity) -> Option<Serving> {
        if self.is_closing() {
            return None;
        }
        let (link, driver) = Link::new(LinkOptions {
            device: device.to_string(),
            identity,
            pipes: self.pipes().clone(),
            policy: Rc::new(ShardPolicy::new(self, device.clone())),
            ping: self.services().runners.ping,
        });
        let seen = DeviceRecorder::new(
            self.services().control.clone(),
            self.services().sync.of(self.user()),
        );
        let serving = self.devices().bind(device, link, driver, seen.clone());
        let device = device.clone();
        self.tasks()
            .spawn_local(async move { seen.touch(device).await });
        Some(serving)
    }

    /// Revokes a device, which nothing refuses: its exposes end with their
    /// connections, its row goes with its attachments and its workspaces,
    /// whose conversations stay outside any workspace, and its runner hears
    /// that it was revoked and removes itself. The user's revocation and the
    /// runner's own request both come here.
    pub async fn revoke_device(&self, device: DeviceId) -> Result<DeviceRemoval, StorageError> {
        self.expose_shard().destroy_exposes_on(&device).await;
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

    /// The user's devices as the web app sees them.
    pub async fn device_list(&self) -> Result<Vec<DeviceDto>, StorageError> {
        self.devices()
            .device_list(&self.services().control, self.user())
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
        let identity = HostIdentity {
            uid: 501,
            gid: 20,
            hostname: "test".into(),
            home_dir: home.into(),
        };
        self.bind(device, identity)
            .expect("an open shard binds")
            .into_driver()
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
        let (answers, answered) = tokio::sync::mpsc::channel::<Result<Vec<u8>, String>>(8);
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
pub(crate) struct PlayedRunner(tokio::sync::mpsc::Sender<Result<Vec<u8>, String>>);

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
