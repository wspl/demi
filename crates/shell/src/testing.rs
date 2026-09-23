//! Test support: the Host conformance cases every [`Host`] passes, the
//! command context test work carries, and an in-memory port for `rpc`
//! handlers.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    rc::Rc,
};

use bytes::Bytes;
use demi_command_service::protocol::{CommandCaller, CommandContext, CommandLocale};
use demi_core::{B64Bytes, StreamKind, Timestamp};
use futures_util::{StreamExt, future::LocalBoxFuture, stream};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{
    ByteRange, CpOptions, FileContents, FileKind, Host, HostError, MkdirOptions, PortError,
    PortRequest, PortResponse, PortTransport, Process, ProcessEnd, Revision, RmOptions, RpcPort,
    Signal, SpawnEnv, SpawnErrorKind, SpawnRequest, StorageOp, StorageReply, WriteOptions,
};

/// The command context test work carries (`native-runtime.md` § Command
/// context). Its locale differs from the backend's default, so a command
/// that reports it shows where it came from.
pub fn test_command_context() -> CommandContext {
    CommandContext {
        conversation: "test-conversation".into(),
        caller: CommandCaller::agent("test-session"),
        locale: CommandLocale {
            time_zone: "Asia/Shanghai".into(),
            languages: vec!["zh-CN".into(), "en".into()],
        },
    }
}

/// One conformance case: what every Host must do. It fails with a text that
/// names the check.
pub struct ConformanceCase {
    pub name: &'static str,
    run: Box<dyn FnOnce() -> LocalBoxFuture<'static, Result<(), String>>>,
}

impl ConformanceCase {
    pub async fn run(self) -> Result<(), String> {
        (self.run)().await
    }
}

/// The Host conformance cases. `host`'s default working directory is
/// `root`, a fresh, writable, absolute directory every case works under.
/// The process cases start `sh`, `printf`, `sleep`, `printenv`, `cat` and
/// `echo` from `path`.
pub fn host_conformance_cases(host: Rc<dyn Host>, root: &str, path: &str) -> Vec<ConformanceCase> {
    let env = SpawnEnv::Exactly(BTreeMap::from([("PATH".into(), path.into())]));
    let mut cases = Vec::new();
    let mut case =
        |name: &'static str, run: fn(Context) -> LocalBoxFuture<'static, Result<(), String>>| {
            let context = Context {
                host: host.clone(),
                root: root.to_owned(),
                env: env.clone(),
            };
            cases.push(ConformanceCase {
                name,
                run: Box::new(move || run(context)),
            });
        };
    case(
        "host: the default working directory is absolute; the identity names a host",
        |c| {
            Box::pin(async move {
                ok(
                    c.host.default_cwd().starts_with('/'),
                    "the default working directory is absolute",
                )?;
                ok(
                    c.host.default_cwd() == c.root,
                    "the default working directory is the root",
                )?;
                ok(
                    !c.host.identity().hostname.is_empty(),
                    "the identity names a hostname",
                )
            })
        },
    );
    case("process: spawn captures stdout and the exit code", |c| {
        Box::pin(async move {
            let output = c.run("printf", &["hello\\n"], None).await?;
            equal(&output.stdout, "hello\n", "stdout")?;
            equal(&output.end, &ProcessEnd::Exited(0), "the end")
        })
    });
    case(
        "process: stdout and stderr are apart; a nonzero exit is reported",
        |c| {
            Box::pin(async move {
                let output = c
                    .run("sh", &["-c", "echo out; echo err >&2; exit 3"], None)
                    .await?;
                equal(&output.stdout, "out\n", "stdout")?;
                equal(&output.stderr, "err\n", "stderr")?;
                equal(&output.end, &ProcessEnd::Exited(3), "the end")
            })
        },
    );
    case(
        "process: stdin reaches the child and ends when closed",
        |c| {
            Box::pin(async move {
                let process = c
                    .spawn(
                        "sh",
                        &["-c", "IFS= read -r line; printf '%s' \"$line\""],
                        None,
                    )
                    .await?;
                process
                    .control
                    .write_stdin(Bytes::from_static(b"from stdin\n"))
                    .await
                    .map_err(text)?;
                process.control.close_stdin().await.map_err(text)?;
                let output = collect(process).await;
                equal(&output.stdout, "from stdin", "stdout")?;
                equal(&output.end, &ProcessEnd::Exited(0), "the end")
            })
        },
    );
    case("process: terminating a process ends it with SIGTERM", |c| {
        Box::pin(async move {
            let process = c.spawn("sleep", &["10"], None).await?;
            process
                .control
                .kill(Signal::Terminate)
                .await
                .map_err(text)?;
            let output = collect(process).await;
            equal(
                &output.end,
                &ProcessEnd::Signalled("SIGTERM".into()),
                "the end",
            )
        })
    });
    case(
        "process: a child receives exactly the environment it was given",
        |c| {
            Box::pin(async move {
                let output = c.run("printenv", &["HOME"], None).await?;
                equal(
                    &output.end,
                    &ProcessEnd::Exited(1),
                    "printenv of an unset name",
                )?;
                equal(&output.stdout, "", "stdout")
            })
        },
    );
    case("process: the working directory is honoured", |c| {
        Box::pin(async move {
            let directory = format!("{}/cwd-honoured", c.root);
            c.host
                .fs()
                .mkdir(&directory, MkdirOptions { recursive: true })
                .await
                .map_err(text)?;
            let output = c.run("sh", &["-c", "pwd"], Some(&directory)).await?;
            equal(&output.end, &ProcessEnd::Exited(0), "the end")?;
            ok(
                output.stdout.trim_end().ends_with("/cwd-honoured"),
                "pwd is inside the directory",
            )
        })
    });
    case(
        "process: a missing program never starts: executable_not_found",
        |c| {
            Box::pin(async move {
                let output = c.run("definitely-not-a-host-binary", &[], None).await?;
                spawn_error(&output.end, SpawnErrorKind::ExecutableNotFound)
            })
        },
    );
    case(
        "process: a missing working directory is cwd_unusable, not a missing program",
        |c| {
            Box::pin(async move {
                let missing = format!("{}/never-created", c.root);
                let output = c.run("echo", &["ok"], Some(&missing)).await?;
                spawn_error(&output.end, SpawnErrorKind::CwdUnusable)
            })
        },
    );
    case(
        "fs: write, replace, read, list, stat, exists, remove; relative paths",
        |c| {
            Box::pin(async move {
                let fs = c.host.fs();
                fs.mkdir("basic/src", MkdirOptions { recursive: true })
                    .await
                    .map_err(text)?;
                let file = format!("{}/basic/src/file.txt", c.root);
                fs.write_file(
                    "basic/src/file.txt",
                    bytes("first\n"),
                    WriteOptions::default(),
                )
                .await
                .map_err(text)?;
                fs.write_file(&file, bytes("hello\ntail\n"), WriteOptions::default())
                    .await
                    .map_err(text)?;
                equal(
                    &read(&c, &file).await?,
                    "hello\ntail\n",
                    "a write replaces the contents",
                )?;
                let entries = fs.read_dir("basic/src").await.map_err(text)?;
                let listed: Vec<_> = entries
                    .iter()
                    .map(|entry| (entry.name.as_str(), entry.kind))
                    .collect();
                equal(&listed, &vec![("file.txt", FileKind::File)], "the listing")?;
                let stat = fs.stat(&file).await.map_err(text)?;
                equal(
                    &(stat.kind, stat.size),
                    &(FileKind::File, 11),
                    "the file's stat",
                )?;
                ok(
                    stat.modified > Timestamp::UNIX_EPOCH,
                    "the file has a modification time",
                )?;
                let directory = fs.stat("basic/src").await.map_err(text)?;
                equal(&directory.kind, &FileKind::Directory, "a directory's stat")?;
                fs.rm(
                    &file,
                    RmOptions {
                        force: true,
                        ..RmOptions::default()
                    },
                )
                .await
                .map_err(text)?;
                equal(
                    &fs.exists(&file).await.map_err(text)?,
                    &false,
                    "exists after rm",
                )?;
                equal(
                    &fs.exists("basic/src").await.map_err(text)?,
                    &true,
                    "exists for a directory",
                )
            })
        },
    );
    case(
        "fs: parents, recursive copy, move, recursive remove, force",
        |c| {
            Box::pin(async move {
                let fs = c.host.fs();
                let root = format!("{}/tree", c.root);
                fs.mkdir(&format!("{root}/a/b"), MkdirOptions { recursive: true })
                    .await
                    .map_err(text)?;
                fs.write_file(
                    &format!("{root}/a/b/f.txt"),
                    bytes("x"),
                    WriteOptions::default(),
                )
                .await
                .map_err(text)?;
                fs.write_file(
                    &format!("{root}/a/new/dir/g.txt"),
                    bytes("y"),
                    WriteOptions {
                        create_parents: true,
                    },
                )
                .await
                .map_err(text)?;
                equal(
                    &read(&c, &format!("{root}/a/new/dir/g.txt")).await?,
                    "y",
                    "create_parents",
                )?;
                fs.cp(
                    &format!("{root}/a"),
                    &format!("{root}/c"),
                    CpOptions { recursive: true },
                )
                .await
                .map_err(text)?;
                equal(
                    &read(&c, &format!("{root}/c/b/f.txt")).await?,
                    "x",
                    "a recursive copy",
                )?;
                equal(
                    &read(&c, &format!("{root}/a/b/f.txt")).await?,
                    "x",
                    "the copy's source",
                )?;
                fs.mv(&format!("{root}/c"), &format!("{root}/d"))
                    .await
                    .map_err(text)?;
                equal(
                    &fs.exists(&format!("{root}/c")).await.map_err(text)?,
                    &false,
                    "the move's source",
                )?;
                equal(
                    &read(&c, &format!("{root}/d/b/f.txt")).await?,
                    "x",
                    "the moved tree",
                )?;
                fs.rm(
                    &format!("{root}/d"),
                    RmOptions {
                        recursive: true,
                        ..RmOptions::default()
                    },
                )
                .await
                .map_err(text)?;
                equal(
                    &fs.exists(&format!("{root}/d")).await.map_err(text)?,
                    &false,
                    "a recursive remove",
                )?;
                fs.rm(
                    &format!("{root}/absent"),
                    RmOptions {
                        force: true,
                        ..RmOptions::default()
                    },
                )
                .await
                .map_err(text)?;
                let missing = fs.rm(&format!("{root}/absent"), RmOptions::default()).await;
                code(
                    missing,
                    "ENOENT",
                    "a remove of a missing path without force",
                )
            })
        },
    );
    case(
        "fs: symlink, readlink, lstat, realpath, link, chmod, utimes",
        |c| {
            Box::pin(async move {
                let fs = c.host.fs();
                let root = format!("{}/links", c.root);
                fs.mkdir(&root, MkdirOptions { recursive: true })
                    .await
                    .map_err(text)?;
                fs.write_file(
                    &format!("{root}/target.txt"),
                    bytes("t"),
                    WriteOptions::default(),
                )
                .await
                .map_err(text)?;
                fs.symlink("target.txt", &format!("{root}/link"))
                    .await
                    .map_err(text)?;
                equal(
                    &fs.readlink(&format!("{root}/link")).await.map_err(text)?,
                    &"target.txt".to_owned(),
                    "readlink",
                )?;
                equal(
                    &fs.lstat(&format!("{root}/link")).await.map_err(text)?.kind,
                    &FileKind::Symlink,
                    "lstat",
                )?;
                equal(
                    &fs.stat(&format!("{root}/link")).await.map_err(text)?.kind,
                    &FileKind::File,
                    "stat follows",
                )?;
                let real = fs.realpath(&format!("{root}/link")).await.map_err(text)?;
                ok(
                    real.ends_with("/links/target.txt"),
                    "realpath resolves the link",
                )?;
                let entries = fs.read_dir(&root).await.map_err(text)?;
                let link = entries
                    .iter()
                    .find(|entry| entry.name == "link")
                    .map(|entry| entry.kind);
                equal(
                    &link,
                    &Some(FileKind::Symlink),
                    "the listing marks the link",
                )?;
                fs.link(&format!("{root}/target.txt"), &format!("{root}/hard"))
                    .await
                    .map_err(text)?;
                equal(
                    &read(&c, &format!("{root}/hard")).await?,
                    "t",
                    "a hard link's contents",
                )?;
                fs.chmod(&format!("{root}/target.txt"), 0o600)
                    .await
                    .map_err(text)?;
                let mode = fs
                    .stat(&format!("{root}/target.txt"))
                    .await
                    .map_err(text)?
                    .mode
                    & 0o777;
                equal(&mode, &0o600, "chmod")?;
                let when = Timestamp::from_millisecond(1_600_000_000_000)
                    .map_err(|error| error.to_string())?;
                fs.utimes(&format!("{root}/target.txt"), when, when)
                    .await
                    .map_err(text)?;
                let modified = fs
                    .stat(&format!("{root}/target.txt"))
                    .await
                    .map_err(text)?
                    .modified;
                equal(&modified, &when, "utimes")
            })
        },
    );
    case(
        "fs: a stream reads the whole file or one byte range; a missing file fails first",
        |c| {
            Box::pin(async move {
                let fs = c.host.fs();
                let file = format!("{}/stream/digits.txt", c.root);
                fs.write_file(
                    &file,
                    bytes("0123456789"),
                    WriteOptions {
                        create_parents: true,
                    },
                )
                .await
                .map_err(text)?;
                let streamed = |range: ByteRange| {
                    let (host, file) = (c.host.clone(), file.clone());
                    async move {
                        let mut stream = host.fs().read_stream(&file, range).await.map_err(text)?;
                        let mut read = Vec::new();
                        while let Some(chunk) = stream.next().await {
                            read.extend_from_slice(&chunk.map_err(text)?);
                        }
                        String::from_utf8(read).map_err(|error| error.to_string())
                    }
                };
                equal(
                    &streamed(ByteRange::default()).await?,
                    "0123456789",
                    "the whole file",
                )?;
                let range = ByteRange {
                    offset: 3,
                    length: Some(4),
                };
                equal(&streamed(range).await?, "3456", "a range")?;
                let rest = ByteRange {
                    offset: 7,
                    length: None,
                };
                equal(&streamed(rest).await?, "789", "from an offset to the end")?;
                let missing = fs
                    .read_stream(&format!("{}/stream/missing", c.root), ByteRange::default())
                    .await;
                code(missing.map(|_| ()), "ENOENT", "a missing file")
            })
        },
    );
    case(
        "fs: a write takes a stream; one that fails leaves the file as it was",
        |c| {
            Box::pin(async move {
                let fs = c.host.fs();
                let root = format!("{}/write-stream", c.root);
                fs.mkdir(&root, MkdirOptions { recursive: true })
                    .await
                    .map_err(text)?;
                let file = format!("{root}/joined.txt");
                let parts =
                    stream::iter(["ab", "cd", "ef"].map(|part| Ok(Bytes::from(part)))).boxed();
                fs.write_file(&file, FileContents::Stream(parts), WriteOptions::default())
                    .await
                    .map_err(text)?;
                equal(&read(&c, &file).await?, "abcdef", "the chunks in order")?;
                let broken = HostError::interrupted("the stream broke");
                let failing =
                    stream::iter([Ok(Bytes::from("partial")), Err(broken.clone())]).boxed();
                let written = fs
                    .write_file(
                        &file,
                        FileContents::Stream(failing),
                        WriteOptions::default(),
                    )
                    .await;
                equal(
                    &written,
                    &Err(broken),
                    "the stream's own failure is the one returned",
                )?;
                equal(&read(&c, &file).await?, "abcdef", "the file is as it was")?;
                let names: Vec<_> = fs
                    .read_dir(&root)
                    .await
                    .map_err(text)?
                    .into_iter()
                    .map(|entry| entry.name)
                    .collect();
                equal(
                    &names,
                    &vec!["joined.txt".to_owned()],
                    "no partial copy beside it",
                )
            })
        },
    );
    case("fs: errors carry the operating system's codes", |c| {
        Box::pin(async move {
            let fs = c.host.fs();
            let root = format!("{}/errors", c.root);
            fs.mkdir(&root, MkdirOptions { recursive: true })
                .await
                .map_err(text)?;
            code(
                fs.read_file(&format!("{root}/missing")).await.map(|_| ()),
                "ENOENT",
                "read_file",
            )?;
            code(
                fs.stat(&format!("{root}/missing")).await.map(|_| ()),
                "ENOENT",
                "stat",
            )?;
            code(
                fs.mkdir(&root, MkdirOptions::default()).await,
                "EEXIST",
                "mkdir of a directory that exists",
            )?;
            code(
                fs.read_dir(&format!("{root}/missing")).await.map(|_| ()),
                "ENOENT",
                "read_dir",
            )
        })
    });
    case("fs and process share one namespace", |c| {
        Box::pin(async move {
            let fs = c.host.fs();
            let root = format!("{}/shared", c.root);
            fs.mkdir(&root, MkdirOptions { recursive: true })
                .await
                .map_err(text)?;
            fs.write_file(
                &format!("{root}/seen.txt"),
                bytes("seen by cat\n"),
                WriteOptions::default(),
            )
            .await
            .map_err(text)?;
            let output = c.run("cat", &["seen.txt"], Some(&root)).await?;
            equal(&output.stdout, "seen by cat\n", "cat's output")?;
            let wrote = c
                .run("sh", &["-c", "printf back > written.txt"], Some(&root))
                .await?;
            equal(&wrote.end, &ProcessEnd::Exited(0), "the shell wrote")?;
            equal(
                &read(&c, &format!("{root}/written.txt")).await?,
                "back",
                "read through fs",
            )
        })
    });
    cases
}

/// What a case works with.
#[derive(Clone)]
struct Context {
    host: Rc<dyn Host>,
    root: String,
    env: SpawnEnv,
}

impl Context {
    async fn spawn(
        &self,
        command: &str,
        args: &[&str],
        cwd: Option<&str>,
    ) -> Result<Process, String> {
        let request = SpawnRequest {
            command: command.into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            cwd: cwd.map(str::to_owned),
            env: self.env.clone(),
            retained: false,
        };
        self.host.process().spawn(request).await.map_err(text)
    }

    async fn run(&self, command: &str, args: &[&str], cwd: Option<&str>) -> Result<Output, String> {
        Ok(collect(self.spawn(command, args, cwd).await?).await)
    }
}

struct Output {
    stdout: String,
    stderr: String,
    end: ProcessEnd,
}

async fn collect(process: Process) -> Output {
    let Process {
        mut output, exit, ..
    } = process;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    while let Some(chunk) = output.next().await {
        match chunk.stream {
            StreamKind::Stdout => stdout.extend_from_slice(&chunk.bytes),
            StreamKind::Stderr => stderr.extend_from_slice(&chunk.bytes),
        }
    }
    Output {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        end: exit.await,
    }
}

async fn read(context: &Context, path: &str) -> Result<String, String> {
    let bytes = context.host.fs().read_file(path).await.map_err(text)?;
    String::from_utf8(bytes.to_vec()).map_err(|error| error.to_string())
}

fn bytes(text: &'static str) -> FileContents {
    FileContents::Bytes(Bytes::from_static(text.as_bytes()))
}

fn text(error: HostError) -> String {
    error.to_string()
}

fn ok(condition: bool, what: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(what.to_owned())
    }
}

fn equal<A, E>(actual: &A, expected: &E, what: &str) -> Result<(), String>
where
    A: PartialEq<E> + std::fmt::Debug + ?Sized,
    E: std::fmt::Debug + ?Sized,
{
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{what}: expected {expected:?}, got {actual:?}"))
    }
}

fn code(result: Result<(), HostError>, expected: &str, what: &str) -> Result<(), String> {
    match result {
        Err(error) if error.code() == Some(expected) => Ok(()),
        Err(error) => Err(format!("{what}: expected {expected}, got {error:?}")),
        Ok(()) => Err(format!("{what}: expected {expected}, got success")),
    }
}

fn spawn_error(end: &ProcessEnd, expected: SpawnErrorKind) -> Result<(), String> {
    match end {
        ProcessEnd::NotStarted(error) if error.kind == expected => Ok(()),
        other => Err(format!(
            "expected a {expected:?} spawn error, got {other:?}"
        )),
    }
}

/// An in-memory port for `rpc` handler tests: standard input and live input
/// fed by the test, output kept, and command storage with revisions. Every
/// request yields once before it is served, as a real port's do, so two
/// handlers interleave.
#[derive(Default)]
pub struct MemoryPort {
    stdin: RefCell<VecDeque<Bytes>>,
    live: RefCell<VecDeque<Bytes>>,
    stdout: RefCell<Vec<u8>>,
    stderr: RefCell<Vec<u8>>,
    storage: Rc<MemoryStorage>,
}

impl MemoryPort {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// A port over `storage`, which ports may share.
    pub fn with_storage(storage: Rc<MemoryStorage>) -> Rc<Self> {
        Rc::new(Self {
            storage,
            ..Self::default()
        })
    }

    /// Finite standard input, in chunks.
    pub fn feed_stdin(&self, chunk: impl Into<Bytes>) {
        self.stdin.borrow_mut().push_back(chunk.into());
    }

    /// An interactive write to the calling job.
    pub fn feed_live(&self, chunk: impl Into<Bytes>) {
        self.live.borrow_mut().push_back(chunk.into());
    }

    pub fn port(self: &Rc<Self>, cancel: CancellationToken) -> RpcPort {
        RpcPort::new(self.clone(), cancel)
    }

    pub fn stdout(&self) -> Vec<u8> {
        self.stdout.borrow().clone()
    }

    pub fn stderr(&self) -> Vec<u8> {
        self.stderr.borrow().clone()
    }
}

impl PortTransport for MemoryPort {
    fn request(&self, request: PortRequest) -> LocalBoxFuture<'_, Result<PortResponse, PortError>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            Ok(match request {
                PortRequest::Stdout { bytes } => {
                    self.stdout.borrow_mut().extend_from_slice(&bytes);
                    PortResponse::Written {}
                }
                PortRequest::Stderr { bytes } => {
                    self.stderr.borrow_mut().extend_from_slice(&bytes);
                    PortResponse::Written {}
                }
                PortRequest::ReadStdin {} => PortResponse::Input {
                    bytes: self.stdin.borrow_mut().pop_front().map(B64Bytes::new),
                },
                PortRequest::ReadLiveStdin {} => PortResponse::Input {
                    bytes: self.live.borrow_mut().pop_front().map(B64Bytes::new),
                },
                PortRequest::Storage { op } => PortResponse::Storage {
                    reply: self.storage.apply(op),
                },
            })
        })
    }
}

/// One node's command storage in memory: every committed write advances the
/// revision.
#[derive(Default)]
pub struct MemoryStorage {
    values: RefCell<BTreeMap<String, Value>>,
    revision: Cell<u64>,
}

impl MemoryStorage {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    pub fn value(&self, key: &str) -> Option<Value> {
        self.values.borrow().get(key).cloned()
    }

    /// Serves one storage operation.
    pub fn apply(&self, op: StorageOp) -> StorageReply {
        let revision = Revision(self.revision.get());
        match op {
            StorageOp::Read { key } => StorageReply::Value {
                value: self.values.borrow().get(&key).cloned(),
                revision,
            },
            StorageOp::List { prefix } => StorageReply::Keys {
                keys: self
                    .values
                    .borrow()
                    .keys()
                    .filter(|key| key.starts_with(&prefix))
                    .cloned()
                    .collect(),
            },
            StorageOp::WriteIf { expected, .. }
                if expected.is_some_and(|expected| expected != revision) =>
            {
                StorageReply::Conflict { revision }
            }
            StorageOp::WriteIf { key, value, .. } => {
                match value {
                    Some(value) => self.values.borrow_mut().insert(key, value),
                    None => self.values.borrow_mut().remove(&key),
                };
                self.revision.set(revision.0 + 1);
                StorageReply::Committed {
                    revision: Revision(revision.0 + 1),
                }
            }
        }
    }
}
