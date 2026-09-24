//! Each agent node's Host and shell environments (`sessions-and-targets.md`
//! § Host operations, `commands.md` § Handle an rpc call). A node's Host is
//! the conversation's current main Host, which the host access admits and
//! lets go at once. A node's environment on it is host-remote's: each of its
//! jobs runs inside the conversation's host access and is refused once the
//! conversation's Host is another, each job's command context names the
//! conversation and the node, and while the environment lives the node's
//! commands answer its jobs' rpc calls.

use std::rc::{Rc, Weak};

use bytes::Bytes;
use demi_agent::{EnvironmentScope, ShellEnvironmentFactory};
use demi_command_service::protocol::{ArtifactLocation, CommandCaller, PackageArtifact};
use demi_host_remote::{
    ArtifactResolver, CommandCatalog, ContextSource, EnvironmentOptions, HostAccess, RemoteHost, RemoteShellEnvironment,
};
use demi_core::{CommandId, ShellId};
use demi_shell::{CommandStatus, ExecRequest, Host, HostError, HostErrorKind, HostKey, ShellEnvironment, ShellError};
use demi_web_api::ids::{ConversationId, DeviceId};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use super::conversation_of;
use super::host_access::{HostAccessError, Refusal};
use crate::runner::command_context::command_context;
use crate::runner::device_of;
use crate::runner::router::CommandRegistration;
use crate::shard::Shard;

impl Shard {
    /// A node's Host: the conversation's current main Host. The host access
    /// admits it and lets it go at once; its later operations take only a
    /// Cloud's per-operation admission.
    pub(crate) async fn conversation_host(&self, id: &ConversationId) -> Result<Rc<RemoteHost>, HostError> {
        let host = self
            .with_host(id, None, &CancellationToken::new(), async |host| host.host.clone())
            .await?;
        Ok(Rc::new(host))
    }

    /// Runs `job`, a shell job the conversation's agent started on `host`,
    /// inside the conversation's host access: it holds what the Host needs
    /// until the job ended, and it is refused once `host` is not the Host
    /// the conversation reaches on that device, as after a target switch.
    pub(crate) async fn run_job(
        &self,
        id: &ConversationId,
        host: &HostKey,
        cancel: &CancellationToken,
        job: LocalBoxFuture<'_, ()>,
    ) -> Result<(), HostError> {
        let device = device_of(host)
            .and_then(|device| DeviceId::try_from(device).ok())
            .ok_or_else(|| HostError::new(HostErrorKind::Protocol, "the job's Host is no conversation's"))?;
        self.with_host(id, Some(&device), cancel, async move |admitted| {
            if admitted.host.key() != *host {
                return Err(HostError::new(
                    HostErrorKind::Unavailable,
                    "the conversation's Host changed; the command did not run",
                ));
            }
            job.await;
            Ok(())
        })
        .await?
    }
}

impl From<HostAccessError> for HostError {
    fn from(error: HostAccessError) -> Self {
        match error {
            HostAccessError::Host(error) => error,
            HostAccessError::Cancelled => HostError::new(HostErrorKind::Interrupted, error.to_string()),
            HostAccessError::Storage(_) => HostError::failed(None, error.to_string()),
            HostAccessError::Missing
            | HostAccessError::Cloud(_)
            | HostAccessError::Refused(
                Refusal::Archived | Refusal::NotAttached | Refusal::Busy | Refusal::Stopped | Refusal::DeviceGone,
            ) => HostError::new(HostErrorKind::Unavailable, error.to_string()),
        }
    }
}

/// Makes each node's shell environments.
pub(crate) struct ShardShellEnvironments {
    /// Weak: the shard owns the agent server that holds this factory.
    shard: Weak<Shard>,
    /// The native packages a node's commands bind to.
    catalog: CommandCatalog,
}

impl ShardShellEnvironments {
    pub(crate) fn new(shard: Weak<Shard>) -> Self {
        let catalog = CommandCatalog::new(Vec::new(), Rc::new(Unpublished))
            .expect("a catalog of no packages is valid");
        Self { shard, catalog }
    }
}

impl ShellEnvironmentFactory<RemoteHost> for ShardShellEnvironments {
    fn create<'a>(
        &'a self,
        scope: EnvironmentScope<'a>,
        host: Rc<RemoteHost>,
    ) -> LocalBoxFuture<'a, Result<Rc<dyn ShellEnvironment>, HostError>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
            let conversation = conversation_of(scope.root);
            let context: ContextSource = {
                let shard = self.shard.clone();
                let conversation = conversation.clone();
                let node = scope.node.to_string();
                Rc::new(move || {
                    let shard = shard.clone();
                    let conversation = conversation.clone();
                    let caller = CommandCaller::agent(node.clone());
                    Box::pin(async move {
                        let shard = shard
                            .upgrade()
                            .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
                        command_context(&shard.services().control, shard.user(), &conversation, caller)
                            .await
                            .map_err(|error| HostError::failed(None, error.to_string()))
                    })
                })
            };
            let mut options = EnvironmentOptions::new((*host).clone(), context);
            options.access = Some(Rc::new(JobAccess {
                shard: self.shard.clone(),
                conversation: conversation.clone(),
            }));
            let selection = self
                .catalog
                .select(scope.commands)
                .map_err(|error| HostError::new(HostErrorKind::Protocol, error.to_string()))?;
            options.commands = Some(selection.clone());
            let environment = RemoteShellEnvironment::new(options);
            let registration =
                shard
                    .commands()
                    .register(scope.node.as_str(), &conversation, scope.commands.clone(), selection);
            Ok(Rc::new(Registered {
                environment,
                _registration: registration,
            }) as Rc<dyn ShellEnvironment>)
        })
    }
}

/// The native packages' artifacts (`native-runtime.md` § Publish artifacts
/// before enabling commands). This backend publishes none yet, so no
/// manifest names a native command and nothing asks.
struct Unpublished;

impl ArtifactResolver for Unpublished {
    fn resolve(
        &self,
        _artifact: &PackageArtifact,
        target: &str,
        _cancel: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<ArtifactLocation, String>> {
        let message = format!("no native package is published for {target}");
        Box::pin(async move { Err(message) })
    }
}

/// The conversation's host access around each job of a node's environment.
struct JobAccess {
    shard: Weak<Shard>,
    conversation: ConversationId,
}

impl HostAccess for JobAccess {
    fn run_job<'a>(
        &'a self,
        host: &'a HostKey,
        cancel: CancellationToken,
        job: LocalBoxFuture<'a, ()>,
    ) -> LocalBoxFuture<'a, Result<(), HostError>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| HostError::offline("the backend is shutting down"))?;
            shard.run_job(&self.conversation, host, &cancel, job).await
        })
    }
}

/// A node's environment, and the registration that answers its jobs' rpc
/// calls with the node's commands while it lives.
struct Registered {
    environment: RemoteShellEnvironment,
    _registration: CommandRegistration,
}

impl ShellEnvironment for Registered {
    fn exec(
        &self,
        request: ExecRequest,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'_, Result<CommandStatus, ShellError>> {
        self.environment.exec(request, cancel)
    }

    fn status(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        self.environment.status(command)
    }

    fn write<'a>(&'a self, command: &'a CommandId, stdin: Bytes) -> LocalBoxFuture<'a, Result<CommandStatus, ShellError>> {
        self.environment.write(command, stdin)
    }

    fn abort<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<CommandStatus, ShellError>> {
        self.environment.abort(command)
    }

    fn release_command<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, bool> {
        self.environment.release_command(command)
    }

    fn dispose_shell<'a>(&'a self, shell: &'a ShellId) -> LocalBoxFuture<'a, bool> {
        self.environment.dispose_shell(shell)
    }

    fn dispose_all(&self) -> LocalBoxFuture<'_, ()> {
        self.environment.dispose_all()
    }

    fn owns_shell(&self, shell: &ShellId) -> bool {
        self.environment.owns_shell(shell)
    }

    fn owns_command(&self, command: &CommandId) -> bool {
        self.environment.owns_command(command)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use demi_web_api::ids::UserId;

    use super::*;
    use crate::auth::sessions::TokenHash;
    use crate::backend::Services;
    use crate::shard::{ShardPlacement, ShardPool};
    use crate::storage::control::testing;
    use crate::storage::conversation_index::Creation;

    const ID: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";

    #[tokio::test(flavor = "local")]
    async fn a_job_runs_inside_the_host_access_and_is_refused_once_the_host_changed() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control.create_conversation(owner.clone(), id.clone()).await.unwrap(),
            Creation::Created(_)
        ));
        let laptop = control
            .create_device(owner.clone(), "laptop".into(), "linux".into(), TokenHash::of("laptop"))
            .await
            .unwrap()
            .id;
        testing::execute(
            &control,
            "UPDATE conversations SET target_kind = 'device', target_device_id = ?2, target_path = '/work'
             WHERE id = ?1",
            vec![ID.to_owned(), laptop.to_string()],
        )
        .await;
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        let answers = pool
            .shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let _connection = shard.connect_for_tests(&laptop, "/home/ana");
                let host = shard.conversation_host(&id).await.unwrap();
                assert_eq!(host.default_cwd(), "/work");
                let ran = Cell::new(0);
                let job = || {
                    let ran = &ran;
                    Box::pin(async move { ran.set(ran.get() + 1) }) as LocalBoxFuture<'_, ()>
                };
                let first = shard.run_job(&id, &host.key(), &CancellationToken::new(), job()).await;
                // The conversation moves to another directory of the device:
                // its old Host runs nothing more.
                testing::execute(
                    &shard.services().control,
                    "UPDATE conversations SET target_path = '/elsewhere' WHERE id = ?1",
                    vec![ID.to_owned()],
                )
                .await;
                let second = shard.run_job(&id, &host.key(), &CancellationToken::new(), job()).await;
                (first, second.map_err(|error| error.kind), ran.get())
            })
            .await
            .unwrap();
        assert_eq!(answers, (Ok(()), Err(HostErrorKind::Unavailable), 1));
        pool.close().await;
    }
}
