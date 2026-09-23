//! Filesystem, working-tree and process requests for one backend connection.

use std::{collections::BTreeMap, io, path::PathBuf, sync::Arc};

use crate::connection::wire::{self as wire, Inbound};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    files::FileTransfers,
    git::GitService,
    net::NetStreams,
    pipes::PipeClient,
    tasks::{TaskCommand, TaskKind, TaskSpec, TaskTable},
};

pub struct HostServer {
    pub tasks: TaskTable,
    default_cwd: PathBuf,
    device_env: BTreeMap<String, String>,
    output: mpsc::Sender<wire::Frame>,
    filesystem: TaskTracker,
    filesystem_capacity: Arc<Semaphore>,
    git: GitService,
    git_capacity: Arc<Semaphore>,
    net: NetStreams,
    files: FileTransfers,
    cancel: CancellationToken,
}

impl Drop for HostServer {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.tasks.cancel();
        self.filesystem.close();
    }
}

impl HostServer {
    pub fn new(
        output: mpsc::Sender<wire::Frame>,
        dispatcher: Option<Arc<crate::commands::dispatch::Dispatcher>>,
        output_dir: PathBuf,
        default_cwd: PathBuf,
        device_env: BTreeMap<String, String>,
        pipes: PipeClient,
    ) -> Self {
        let cancel = CancellationToken::new();
        Self {
            tasks: TaskTable::new(output.clone(), dispatcher, output_dir, pipes.clone()),
            default_cwd,
            device_env,
            net: NetStreams::new(output.clone(), pipes.clone(), cancel.clone()),
            files: FileTransfers::new(output.clone(), pipes, cancel.clone()),
            output,
            filesystem: TaskTracker::new(),
            filesystem_capacity: Arc::new(Semaphore::new(32)),
            git: GitService::default(),
            git_capacity: Arc::new(Semaphore::new(8)),
            cancel,
        }
    }

    /// Registers work synchronously so following input and kill messages observe it.
    /// Connection-owned command context values override caller environment values.
    pub fn handle_task(
        &self,
        message: &Inbound,
        execution_env: &BTreeMap<String, String>,
        lifetime: Option<Box<dyn Send>>,
    ) -> io::Result<bool> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("host connection closed"));
        }
        match message {
            Inbound::Spawn {
                spawn_id,
                command,
                args,
                cwd,
                env,
                kill_process_group,
                inherit_env,
            } => {
                let mut values = if env.is_none() || inherit_env == &Some(true) {
                    self.device_env.clone()
                } else {
                    BTreeMap::new()
                };
                if let Some(env) = env {
                    for (name, value) in env {
                        match value {
                            Some(value) => {
                                values.insert(name.clone(), value.clone());
                            }
                            None => {
                                values.remove(name);
                            }
                        }
                    }
                }
                let env = Self::environment(values, execution_env);
                self.tasks.start(TaskSpec {
                    id: spawn_id.clone(),
                    cwd: cwd
                        .as_ref()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| self.default_cwd.clone()),
                    env,
                    lifetime,
                    command: TaskCommand::Process {
                        command: command.clone(),
                        args: args.clone().unwrap_or_default(),
                        process_group: kill_process_group.unwrap_or(false),
                    },
                })?;
            }
            Inbound::JobStart {
                job_id,
                script,
                cwd,
                env,
                stdin,
                stdout,
                ..
            } => {
                let mut job_env = self.device_env.clone();
                job_env.extend(env.clone());
                self.tasks.start(TaskSpec {
                    id: job_id.clone(),
                    cwd: PathBuf::from(cwd),
                    env: Self::environment(job_env, execution_env),
                    lifetime,
                    command: TaskCommand::Shell {
                        script: script.clone(),
                        stdin: stdin.clone(),
                        stdout: stdout.clone(),
                    },
                })?;
            }
            Inbound::SpawnStdin { spawn_id, bytes } => {
                self.tasks
                    .input(TaskKind::Spawn, spawn_id, bytes.0.clone().into())?
            }
            Inbound::JobStdin { job_id, bytes } => {
                self.tasks
                    .input(TaskKind::Job, job_id, bytes.0.clone().into())?
            }
            Inbound::SpawnStdinEnd { spawn_id } => {
                self.tasks.end_input(TaskKind::Spawn, spawn_id)?
            }
            Inbound::JobStdinEnd { job_id } => self.tasks.end_input(TaskKind::Job, job_id)?,
            Inbound::SpawnKill { spawn_id, signal } => self.tasks.signal(
                TaskKind::Spawn,
                spawn_id,
                signal.clone().unwrap_or_else(|| "SIGTERM".into()),
            )?,
            Inbound::JobKill { job_id, signal } => self.tasks.signal(
                TaskKind::Job,
                job_id,
                signal.clone().unwrap_or_else(|| "SIGTERM".into()),
            )?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Filesystem work never blocks the connection's control-message loop.
    /// A file's contents go to the transfers, whose pipes pace them.
    pub fn handle_filesystem(&self, message: Inbound) -> io::Result<()> {
        match message {
            Inbound::FsReadFile { .. } => return self.files.read(message, &self.default_cwd),
            Inbound::FsWriteFile { .. } => return self.files.write(message, &self.default_cwd),
            _ => {}
        }
        if message.fs_request_id().is_none() {
            return Err(io::Error::other("not a filesystem request"));
        }
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("host connection closed"));
        }
        // Past the capacity a request waits for a slot (`runner.md` § Load).
        let capacity = self.filesystem_capacity.clone();
        let cancel = self.cancel.clone();
        let output = self.output.clone();
        let cwd = self.default_cwd.clone();
        self.filesystem.spawn(async move {
            let _permit = tokio::select! {
                permit = capacity.acquire_owned() => permit.expect("filesystem capacity is never closed"),
                _ = cancel.cancelled() => return,
            };
            match crate::fs::handle(&message, &cwd, &cancel)
                .await
                .expect("validated filesystem request")
            {
                Ok(reply) => {
                    tokio::select! {
                        _ = cancel.cancelled() => {},
                        result = output.send(reply) => {
                            if result.is_err() {
                                // The connection owner has closed its receiver.
                                cancel.cancel();
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        "filesystem response encoding failed: {error}"
                    );
                    cancel.cancel();
                }
            }
        });
        Ok(())
    }

    /// Working-tree work rides the filesystem tracker with its own admission:
    /// past eight requests in flight, later ones wait for a slot.
    pub fn handle_git(&self, message: Inbound) -> io::Result<()> {
        if message.git_request_id().is_none() {
            return Err(io::Error::other("not a working-tree request"));
        }
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("host connection closed"));
        }
        let capacity = self.git_capacity.clone();
        if let Inbound::GitShow { .. } = message {
            return self
                .files
                .show(message, &self.default_cwd, self.git.clone(), capacity);
        }
        let cancel = self.cancel.clone();
        let output = self.output.clone();
        let cwd = self.default_cwd.clone();
        let git = self.git.clone();
        self.filesystem.spawn(async move {
            let _permit = tokio::select! {
                permit = capacity.acquire_owned() => permit.expect("working-tree capacity is never closed"),
                _ = cancel.cancelled() => return,
            };
            match crate::git::handle(&git, &message, &cwd, &cancel)
                .await
                .expect("validated working-tree request")
            {
                Ok(reply) => {
                    tokio::select! {
                        _ = cancel.cancelled() => {},
                        result = output.send(reply) => {
                            if result.is_err() {
                                // The connection owner has closed its receiver.
                                cancel.cancel();
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        "working-tree response encoding failed: {error}"
                    );
                    cancel.cancel();
                }
            }
        });
        Ok(())
    }

    /// One network stream request (`runner.md` § Network streams): the
    /// tracker owns the socket until the connection closes or both pipes end.
    pub fn handle_net(&self, message: Inbound) -> io::Result<()> {
        self.net.handle_open(message)
    }

    pub async fn close(&self) {
        self.cancel.cancel();
        self.filesystem.close();
        tokio::join!(
            self.tasks.close(),
            self.filesystem.wait(),
            self.net.close(),
            self.files.close()
        );
    }

    fn environment(
        mut env: BTreeMap<String, String>,
        execution: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        env.extend(execution.clone());
        env
    }
}
