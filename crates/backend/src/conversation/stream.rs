//! User streams and one-shot user calls (`native-runtime.md` § User streams;
//! `sessions-and-targets.md` § Host operations): an operation of a native
//! package that serves the conversation's user directly, in the resident
//! service that holds the conversation's state. A stream's bytes go both
//! ways for as long as the page keeps it open, and the edge relays them
//! under the stream's lease; a one-shot call sends no input and reads its
//! JSON answer to its end. Neither wakes a stopped Cloud unless the call is
//! work the user starts.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bytes::Bytes;
use demi_command_service::protocol::{CommandCaller, PackageDescriptor};
use demi_command_tree::NativeOperation;
use demi_host_remote::{Pipe, PipeReader, PipeWriter, ServiceCallError, ServiceRequest, ServiceStream};
use demi_shell::HostErrorKind;
use demi_web_api::error::ErrorCode;
use demi_web_api::ids::ConversationId;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use super::host_access::{ConversationHost, HostAccessError, Waits};
use super::transfer::OpenTransfer;
use crate::runner::command_context::command_context;
use crate::runner::native::NativeCatalog;
use crate::shard::Shard;
use crate::shard::lease::Lease;

/// The native operation a user stream or a one-shot user call runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceBinding {
    pub(crate) package: PackageDescriptor,
    pub(crate) operation: String,
}

/// The user streams a page may open, by name. Each binds an operation of a
/// published package; they are fixed with the command tree for the
/// backend's lifetime.
#[derive(Debug, Clone, Default)]
pub(crate) struct UserStreams(Arc<HashMap<String, ServiceBinding>>);

impl UserStreams {
    /// The `declared` streams that the published packages serve; a binding
    /// no package provides declares nothing.
    pub(crate) fn new(declared: &BTreeMap<String, NativeOperation>, native: &NativeCatalog) -> Self {
        let bound = declared.iter().filter_map(|(name, binding)| {
            if !native.serves(&binding.package, &[binding.operation.as_str()]) {
                return None;
            }
            let binding = ServiceBinding {
                package: native.package(&binding.package)?.clone(),
                operation: binding.operation.clone(),
            };
            Some((name.clone(), binding))
        });
        Self(Arc::new(bound.collect()))
    }

    pub(crate) fn get(&self, name: &str) -> Option<&ServiceBinding> {
        self.0.get(name)
    }
}

/// An open user stream, as the edge relays it. `Send`.
pub(crate) struct UserStream {
    /// The page's bytes, the invocation's input.
    pub(crate) to_host: PipeWriter,
    /// The invocation's output, which ends when the invocation completes.
    pub(crate) from_host: PipeReader,
    pub(crate) lease: Lease,
}

/// Why a user stream did not open.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StreamError {
    #[error(transparent)]
    Access(#[from] HostAccessError),
    /// The Host could not open the stream: its service failed to start, or
    /// refused it.
    #[error("{0}")]
    Failed(String),
}

impl StreamError {
    /// The code and HTTP status the error answers with.
    pub(crate) fn code(&self) -> (ErrorCode, u16) {
        match self {
            Self::Access(error) => error.code(),
            Self::Failed(_) => (ErrorCode::StreamFailed, 502),
        }
    }
}

/// Whether a one-shot user call wakes a stopped Cloud: work the user starts,
/// such as opening a browser tab, does; a look at what runs there, such as
/// listing the tabs, does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(dead_code, reason = "the conversation browser's tab routes call one-shot user calls")]
pub(crate) enum Wake {
    Yes,
    No,
}

/// A one-shot user call: the operation, its arguments, and the most its
/// JSON answer may be.
#[derive(Debug, Clone)]
#[expect(dead_code, reason = "the conversation browser's tab routes call one-shot user calls")]
pub(crate) struct ServiceCall {
    pub(crate) binding: ServiceBinding,
    pub(crate) args: Map<String, Value>,
    pub(crate) max_bytes: usize,
}

/// Why a one-shot user call failed: its admission, or the call, whose
/// failure carries the operation's own words.
#[derive(Debug, thiserror::Error)]
#[expect(dead_code, reason = "the conversation browser's tab routes call one-shot user calls")]
pub(crate) enum UserCallError {
    #[error(transparent)]
    Access(#[from] HostAccessError),
    #[error(transparent)]
    Call(#[from] ServiceCallError),
}

impl Shard {
    /// Opens the user stream `binding` names on the conversation's main
    /// Host: admitted without waking a stopped Cloud, and open until the
    /// edge drops its lease, the invocation completes, or a transition ends
    /// it. Nothing moves before the runner opened it.
    pub(crate) async fn open_user_stream(
        &self,
        id: &ConversationId,
        binding: &ServiceBinding,
        cancel: &CancellationToken,
    ) -> Result<UserStream, StreamError> {
        let access = self.admit_stream(id, cancel).await?;
        let request = self
            .service_request(&access.conversation, &access.host, binding, None)
            .await?;
        let device = access.device.as_str();
        let input = self.pipes().to_device(device);
        let output = self.pipes().from_device(device);
        let to_host = input.writer().expect("a pipe just made has its source free");
        let from_host = output.reader().expect("a pipe just made has its sink free");
        let waits = Waits {
            cancel,
            ended: Some(&access.open.ended),
        };
        let opened = waits
            .wait(access.host.host.open_service(request, input.wire_ref(), output.wire_ref()))
            .await;
        let refused = match opened {
            Ok(Ok(service)) => {
                let lease = self.lease_stream(access.open, service, input, output);
                return Ok(UserStream {
                    to_host,
                    from_host,
                    lease,
                });
            }
            Err(error) => StreamError::Access(error),
            Ok(Err(error)) if matches!(error.kind, HostErrorKind::Offline) => HostAccessError::Host(error).into(),
            Ok(Err(error)) => StreamError::Failed(error.message),
        };
        input.fail("the user stream never opened");
        output.fail("the user stream never opened");
        Err(refused)
    }

    /// Runs a one-shot user call on the conversation's main Host and returns
    /// its JSON answer. With `Wake::Yes` it is an ordinary operation of the
    /// conversation's host access; with `Wake::No` it is admitted as a user
    /// stream is, and a transition ends it instead of waiting for it.
    #[expect(dead_code, reason = "the conversation browser's tab routes call one-shot user calls")]
    pub(crate) async fn user_call(
        &self,
        id: &ConversationId,
        wake: Wake,
        call: &ServiceCall,
        cancel: &CancellationToken,
    ) -> Result<Bytes, UserCallError> {
        match wake {
            Wake::Yes => {
                let record = self.owned_conversation(id).await?;
                self.with_host(&record.id, None, cancel, async |host| {
                    let request = self
                        .service_request(&record.id, host, &call.binding, Some(&call.args))
                        .await?;
                    let answer = host.host.call_service(request, Bytes::new(), call.max_bytes).await?;
                    Ok::<_, UserCallError>(answer)
                })
                .await?
            }
            Wake::No => {
                // Registered with the conversation's transfers until the
                // answer is in, so a transition ends the call.
                let access = self.admit_stream(id, cancel).await?;
                let request = self
                    .service_request(&access.conversation, &access.host, &call.binding, Some(&call.args))
                    .await?;
                let waits = Waits {
                    cancel,
                    ended: Some(&access.open.ended),
                };
                let answer = waits
                    .wait(access.host.host.call_service(request, Bytes::new(), call.max_bytes))
                    .await??;
                Ok(answer)
            }
        }
    }

    /// What the runner invokes for the user: the operation in the package
    /// the conversation's jobs use, with a `user` caller, in the
    /// conversation's directory, with `args` for a one-shot call, which
    /// answers in JSON. The runner fetches the service's executable from
    /// where the published catalog says, while the work is open.
    async fn service_request(
        &self,
        conversation: &ConversationId,
        host: &ConversationHost,
        binding: &ServiceBinding,
        args: Option<&Map<String, Value>>,
    ) -> Result<ServiceRequest, HostAccessError> {
        let context = command_context(&self.services().control, self.user(), conversation, CommandCaller::User {}).await?;
        Ok(ServiceRequest {
            context,
            package: binding.package.clone(),
            operation: binding.operation.clone(),
            args: args.cloned(),
            json: args.map(|_| true),
            cwd: host.root.clone(),
            resolver: self.services().native.resolver(),
        })
    }

    /// Hands the edge a lease on an open user stream. The shard's owner
    /// task keeps the stream registered with the conversation's transfers
    /// until the edge drops the lease or a transition ends the stream; then
    /// both pipes fail, which cancels an invocation still running, and the
    /// service stops serving the stream's artifact requests. A pipe that
    /// completed stays complete.
    fn lease_stream(&self, open: OpenTransfer, service: ServiceStream, input: Pipe, output: Pipe) -> Lease {
        let (lease, released) = Lease::new(open.ended.clone());
        let ended = open.ended.clone();
        self.tasks().spawn_local(async move {
            tokio::select! {
                () = released => {}
                () = ended.cancelled() => {}
            }
            input.fail("the user stream ended");
            output.fail("the user stream ended");
            drop(service);
            drop(open);
        });
        lease
    }
}
