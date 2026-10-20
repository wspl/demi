//! User streams and one-shot user calls
//! (`crates/runner-jobs/src/commands/streams.rs`): an operation of a native
//! package that serves the conversation's user directly, in the resident
//! service that holds the conversation's state.
//!
//! A stream's bytes go both ways for as long as the page keeps it open, and the
//! edge relays them under the stream's lease, as it does a file transfer. A
//! one-shot call sends no input and reads its JSON answer to its end.
//!
//! Neither wakes a stopped Cloud unless the call is work the user starts: a
//! stopped Cloud holds none of the state a stream shows, so the page learns
//! that the Host is stopped. Work the user starts from the view, such as
//! opening a new conversation browser tab, is ordinary demand and wakes it.
//!
//! An open stream is the conversation's activity while its page watches it,
//! and so is a call that operates the Host. A stream the page keeps open
//! unwatched (`crates/runner-protocol/src/direct.rs`, `StreamWatched`) is not,
//! nor is a call that only looks
//! (`crates/backend-user-shard/src/lifecycle/mod.rs`).
//!
//! Once admitted, a stream does not hold the conversation's file gate, so a
//! transition ends it instead of waiting for it: an archive, a target or
//! directory change and a detach end the conversation's streams and one-shot
//! calls as they end its file transfers, and a Cloud stop or reset ends them
//! with the device's other work. The page reopens its streams by itself
//! afterwards.

use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use bytes::Bytes;
use demi_backend_remote_host::{
    Pipe, PipeReader, PipeWriter, ServiceCallError, ServiceRequest, ServiceStream,
};
use demi_backend_runners::command_context::command_context;
use demi_backend_runners::native::NativeCatalog;
use demi_command_declarations::NativeOperation;
use demi_command_protocol::CommandCaller;
use demi_host_interface::HostErrorKind;
use demi_shared_gates::{ActivityGate, GateLease, Purpose};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use serde_json::{Map, Value};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::HostShard;
use crate::access::{Attention, ConversationHost, HostAccessError, Waits};
use crate::lease::Lease;
use crate::transfer::OpenTransfer;

/// The native operation a user stream or a one-shot user call runs, as a
/// direct channel's introduction carries it too.
pub use demi_runner_protocol::direct::ServiceBinding;

/// The user streams a page may open, by name. Each binds an operation of a
/// published package; the plugins declare them, fixed for the backend's
/// lifetime.
#[derive(Debug, Clone, Default)]
pub struct UserStreams(Arc<HashMap<String, ServiceBinding>>);

impl UserStreams {
    /// The `declared` streams, each a name and its operation, that the
    /// published packages serve; a binding no package provides declares
    /// nothing.
    pub fn new<'a>(
        declared: impl IntoIterator<Item = (&'a str, &'a NativeOperation)>,
        native: &NativeCatalog,
    ) -> Self {
        let bound = declared.into_iter().filter_map(|(name, binding)| {
            if !native.serves(&binding.package, &[binding.operation.as_str()]) {
                return None;
            }
            let binding = ServiceBinding {
                package: native.package(&binding.package)?.clone(),
                operation: binding.operation.clone(),
            };
            Some((name.to_owned(), binding))
        });
        Self(Arc::new(bound.collect()))
    }

    pub fn get(&self, name: &str) -> Option<&ServiceBinding> {
        self.0.get(name)
    }

    /// Every stream, by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ServiceBinding)> {
        self.0.iter().map(|(name, binding)| (name.as_str(), binding))
    }
}

/// An open user stream, as the edge relays it. `Send`.
pub struct UserStream {
    /// The page's bytes, the invocation's input.
    pub to_host: PipeWriter,
    /// The invocation's output, which ends when the invocation completes.
    pub from_host: PipeReader,
    /// Whether the page watches the stream now, which it starts as: only a
    /// watched stream holds the conversation active.
    pub watched: watch::Sender<bool>,
    pub lease: Lease,
}

/// Why a user stream did not open.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error(transparent)]
    Access(#[from] HostAccessError),
    /// The Host could not open the stream: its service failed to start, or
    /// refused it.
    #[error("{0}")]
    Failed(String),
}

impl StreamError {
    /// The code and HTTP status the error answers with.
    pub fn code(&self) -> (ErrorCode, u16) {
        match self {
            Self::Access(error) => error.code(),
            Self::Failed(_) => (ErrorCode::StreamFailed, 503),
        }
    }
}

/// What a one-shot user call does on the conversation's Host, which decides
/// whether it wakes a stopped Cloud and whether it is activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserCallKind {
    /// Work the user starts, such as opening a conversation browser tab: an
    /// ordinary operation of the conversation's host access, which wakes a
    /// stopped Cloud.
    Starts,
    /// An operation of the user's on what runs there, such as closing a
    /// tab: activity, though a stopped Cloud is refused rather than woken.
    Operates,
    /// A look at what runs there, such as listing the tabs: a stopped Cloud
    /// is refused, and the look is no activity.
    Looks,
}

/// A one-shot user call: the operation, its arguments, and the most its
/// JSON answer may be.
#[derive(Debug, Clone)]
pub struct ServiceCall {
    pub binding: ServiceBinding,
    pub args: Map<String, Value>,
    pub max_bytes: usize,
}

/// Why a one-shot user call failed: its admission, or the call, whose
/// failure carries the operation's own words.
#[derive(Debug, thiserror::Error)]
pub enum UserCallError {
    #[error(transparent)]
    Access(#[from] HostAccessError),
    #[error(transparent)]
    Call(#[from] ServiceCallError),
}

impl dyn HostShard + '_ {
    /// Opens the user stream `binding` names on the conversation's primary
    /// Host. The stream is admitted without waking a stopped Cloud, and open
    /// until the edge drops its lease, the invocation completes, or a
    /// transition ends it. Nothing moves before the runner opened it.
    pub async fn open_user_stream(
        &self,
        id: &ConversationId,
        binding: &ServiceBinding,
        cancel: &CancellationToken,
    ) -> Result<UserStream, StreamError> {
        let access = self.admit_stream(id, None, Attention::Watches, cancel).await?;
        let request = self
            .service_request(&access.conversation, &access.host, binding, None)
            .await?;
        let device = access.device.as_str();
        let input = self.pipes().to_device(device);
        let output = self.pipes().from_device(device);
        let to_host = input
            .writer()
            .expect("a pipe just made has its source free");
        let from_host = output.reader().expect("a pipe just made has its sink free");
        let waits = Waits::until_ended(cancel, &access.open.ended);
        let opened = waits
            .wait(
                access
                    .host
                    .host
                    .open_service(request, input.wire_ref(), output.wire_ref()),
            )
            .await;
        let refused = match opened {
            Ok(Ok(service)) => {
                let (watched, watching) = watch::channel(true);
                let watching = access.watching.map(|lease| Watching {
                    lease: Some(lease),
                    gate: self.conversations().slot(&access.conversation).streams.clone(),
                    shard: Rc::downgrade(&self.this()),
                    conversation: access.conversation.clone(),
                    watched: watching,
                });
                let lease = self.lease_stream(access.open, watching, service, input, output);
                return Ok(UserStream {
                    to_host,
                    from_host,
                    watched,
                    lease,
                });
            }
            Err(error) => StreamError::Access(error),
            Ok(Err(error)) if matches!(error.kind, HostErrorKind::Offline) => {
                HostAccessError::Host(error).into()
            }
            Ok(Err(error)) => StreamError::Failed(error.message),
        };
        input.fail("the user stream never opened");
        output.fail("the user stream never opened");
        Err(refused)
    }

    /// Runs a one-shot user call on the conversation's primary Host and returns
    /// its JSON answer. Work the user starts is an ordinary operation of the
    /// conversation's host access. Any other call is admitted as a user stream
    /// is, and a transition ends it instead of waiting for it.
    pub async fn user_call(
        &self,
        id: &ConversationId,
        kind: UserCallKind,
        call: &ServiceCall,
        cancel: &CancellationToken,
    ) -> Result<Bytes, UserCallError> {
        let attention = match kind {
            UserCallKind::Starts => {
                let record = self.owned_conversation(id).await?;
                return self
                    .with_host(&record.id, None, cancel, async |host| {
                        let request = self
                            .service_request(&record.id, host, &call.binding, Some(&call.args))
                            .await?;
                        let answer = host
                            .host
                            .call_service(request, Bytes::new(), call.max_bytes)
                            .await?;
                        Ok::<_, UserCallError>(answer)
                    })
                    .await?;
            }
            UserCallKind::Operates => Attention::Operates,
            UserCallKind::Looks => Attention::Looks,
        };
        // Registered with the conversation's transfers until the answer is
        // in, so a transition ends the call.
        let access = self.admit_stream(id, None, attention, cancel).await?;
        let request = self
            .service_request(
                &access.conversation,
                &access.host,
                &call.binding,
                Some(&call.args),
            )
            .await?;
        let waits = Waits::until_ended(cancel, &access.open.ended);
        let answer = waits
            .wait(
                access
                    .host
                    .host
                    .call_service(request, Bytes::new(), call.max_bytes),
            )
            .await??;
        Ok(answer)
    }

    /// Runs `operation`, a look of the user's at the conversation's primary
    /// Host, such as which paths a message names are files
    /// (`crates/backend-host-access/src/access.rs`).
    ///
    /// The look is admitted as a user stream is: a stopped Cloud is refused
    /// rather than woken, the look is no activity, and a transition ends it
    /// instead of waiting for it.
    pub async fn look_at_host<T>(
        &self,
        id: &ConversationId,
        cancel: &CancellationToken,
        operation: impl AsyncFnOnce(&ConversationHost) -> T,
    ) -> Result<T, HostAccessError> {
        let access = self.admit_stream(id, None, Attention::Looks, cancel).await?;
        let waits = Waits::until_ended(cancel, &access.open.ended);
        waits.wait(operation(&access.host)).await
    }

    /// Runs `operation`, a look of the user's at the conversation's primary
    /// Host, only while that Host is `device`; none when the conversation runs
    /// on another one, such as after a switch of its target, so the look never
    /// reads the same path on another device.
    ///
    /// It is admitted as [`look_at_host`](Self::look_at_host) is: a stopped
    /// Cloud is refused rather than woken.
    pub async fn look_at_host_on<T>(
        &self,
        id: &ConversationId,
        device: &DeviceId,
        cancel: &CancellationToken,
        operation: impl AsyncFnOnce(&ConversationHost) -> T,
    ) -> Result<Option<T>, HostAccessError> {
        let access = self.admit_stream(id, None, Attention::Looks, cancel).await?;
        if access.device != *device {
            return Ok(None);
        }
        let waits = Waits::until_ended(cancel, &access.open.ended);
        waits.wait(operation(&access.host)).await.map(Some)
    }

    /// What the runner invokes for the user:
    ///
    /// - The operation in the package the conversation's jobs use, with a
    ///   `user` caller.
    /// - The conversation's directory as the working directory.
    /// - `args` for a one-shot call, which answers in JSON.
    ///
    /// The runner fetches the service's executable from where the published
    /// catalog says, while the work is open.
    async fn service_request(
        &self,
        conversation: &ConversationId,
        host: &ConversationHost,
        binding: &ServiceBinding,
        args: Option<&Map<String, Value>>,
    ) -> Result<ServiceRequest, HostAccessError> {
        let context = command_context(
            self.control(),
            self.user(),
            conversation,
            CommandCaller::User {},
        )
        .await?;
        Ok(ServiceRequest {
            context,
            package: binding.package.clone(),
            operation: binding.operation.clone(),
            args: args.cloned(),
            json: args.map(|_| true),
            cwd: host.root.clone(),
            resolver: self.native().resolver(self.public_url()),
            attached: Vec::new(),
        })
    }

    /// Hands the edge a lease on an open user stream.
    ///
    /// The shard's owner task keeps the stream registered with the
    /// conversation's transfers, and its lease of the stream gate, until the
    /// edge drops the lease or a transition ends the stream. Then both pipes
    /// fail, which cancels an invocation still running, and the service stops
    /// serving the stream's artifact requests. A pipe that completed stays
    /// complete.
    fn lease_stream(
        &self,
        open: OpenTransfer,
        watching: Option<Watching>,
        service: ServiceStream,
        input: Pipe,
        output: Pipe,
    ) -> Lease {
        let (lease, released) = Lease::new(open.ended.clone());
        let ended = open.ended.clone();
        self.tasks().spawn_local(async move {
            let follow = async {
                let Some(mut watching) = watching else {
                    return std::future::pending().await;
                };
                watching.follow().await;
            };
            tokio::select! {
                () = released => {}
                () = ended.cancelled() => {}
                () = follow => {}
            }
            input.fail("the user stream ended");
            output.fail("the user stream ended");
            drop(service);
            drop(open);
        });
        lease
    }
}

/// A stream's hold on its conversation's activity: its lease of the stream
/// gate while its page watches it.
struct Watching {
    lease: Option<GateLease>,
    gate: ActivityGate,
    /// Weak: the shard owns the task that holds this.
    shard: Weak<dyn HostShard>,
    conversation: ConversationId,
    watched: watch::Receiver<bool>,
}

impl Watching {
    /// Takes and lets go of the lease as the page says, until the edge lets
    /// go of the stream. The gate is never reserved
    /// (`crates/backend-host-access/src/access.rs`), so a lease comes at
    /// once.
    async fn follow(&mut self) {
        while self.watched.changed().await.is_ok() {
            let watched = *self.watched.borrow_and_update();
            match (watched, self.lease.is_some()) {
                (true, false) => {
                    self.lease = Some(self.gate.enter(Purpose::Demand).await);
                    // Unwatched, it may have let the conversation go idle,
                    // which ended its idle watch; the activity starts one,
                    // as a Host admission does.
                    if let Some(shard) = self.shard.upgrade() {
                        shard.track_idle(&self.conversation);
                    }
                }
                (false, true) => self.lease = None,
                _ => {}
            }
        }
    }
}
