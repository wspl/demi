//! One shell job owns interpreter work, utilities, IO and external children.

use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};

use brush_core::{
    execution_host::{ChildAttributes, ExecutionHost, FileControl},
    processes::ChildProcess,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::commands::{contexts::ExecutionContext, dispatch::Dispatcher};

#[derive(Debug, thiserror::Error)]
#[error("shell job cancelled")]
pub struct Cancelled;

/// What a test sees of a job's work, so that it can cancel the job at a
/// known point: how many of the job's units wait inside an interruptible
/// read, write or sleep, and how often the job has checked for cancellation,
/// which a loop does at every step.
#[cfg(feature = "test-fixtures")]
#[derive(Default)]
pub struct Activity {
    waiting: std::sync::atomic::AtomicUsize,
    checks: std::sync::atomic::AtomicUsize,
}

#[cfg(feature = "test-fixtures")]
impl Activity {
    pub fn waiting(&self) -> usize {
        self.waiting.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn checks(&self) -> usize {
        self.checks.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Counts a unit as waiting until the guard drops.
    fn wait(&self) -> Waiting<'_> {
        self.waiting.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Waiting(self)
    }
}

#[cfg(feature = "test-fixtures")]
struct Waiting<'a>(&'a Activity);

#[cfg(feature = "test-fixtures")]
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.waiting.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// A pipe that becomes readable when a token is cancelled: a task closes its
/// write end then. It lives as long as the scope that made it.
#[cfg(unix)]
struct Interrupt {
    reader: std::io::PipeReader,
    _closer: tokio_util::task::AbortOnDropHandle<()>,
}

#[cfg(unix)]
impl Interrupt {
    /// Made on a shell thread, inside the shell runtime, from a new pipe.
    fn new(
        cancellation: &CancellationToken,
        (reader, writer): (std::io::PipeReader, std::io::PipeWriter),
    ) -> io::Result<Self> {
        let cancelled = cancellation.clone();
        let closer = tokio::runtime::Handle::try_current()
            .map_err(io::Error::other)?
            .spawn(async move {
                cancelled.cancelled().await;
                drop(writer);
            });
        Ok(Self {
            reader,
            _closer: tokio_util::task::AbortOnDropHandle::new(closer),
        })
    }
}

#[derive(Clone)]
pub struct CommandContext {
    pub dispatcher: Arc<Dispatcher>,
    pub execution: Arc<ExecutionContext>,
}

#[derive(Clone)]
pub struct Scope {
    pub cancellation: CancellationToken,
    pub tasks: TaskTracker,
    pub commands: Option<CommandContext>,
    pub edits: Option<demi_command_service::edits::Recorder>,
    /// Readable once `cancellation` is, so blocking IO polls it with its
    /// file; made when blocking IO first needs it. A std mutex: the job's
    /// unit threads share it for the moment it takes to make.
    #[cfg(unix)]
    interrupt: Arc<Mutex<Option<Arc<Interrupt>>>>,
    /// The paths of the files the job opened for writing, by identity. A std
    /// mutex: the synchronous hooks of the job's interpreter and utility
    /// threads share it for short sections that never await.
    files: Arc<Mutex<HashMap<FileIdentity, PathBuf>>>,
    #[cfg(feature = "test-fixtures")]
    activity: Arc<Activity>,
}

impl Scope {
    pub fn new(cancellation: CancellationToken, commands: Option<CommandContext>) -> Self {
        Self {
            cancellation,
            tasks: TaskTracker::new(),
            commands,
            edits: None,
            files: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(unix)]
            interrupt: Arc::new(Mutex::new(None)),
            #[cfg(feature = "test-fixtures")]
            activity: Arc::default(),
        }
    }

    /// What the job's units are doing, for a test.
    #[cfg(feature = "test-fixtures")]
    pub fn activity(&self) -> &Activity {
        &self.activity
    }

    pub fn check(&self) -> io::Result<()> {
        #[cfg(feature = "test-fixtures")]
        self.activity
            .checks
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.cancellation.is_cancelled() {
            // Read/Write helpers retry Interrupted; cancellation must escape them.
            Err(io::Error::other(Cancelled))
        } else {
            Ok(())
        }
    }

    /// Runs `make`, which makes descriptors; out of open files it tries
    /// again until one closes or the job ends (`runner.md` § Load). It runs
    /// on a shell thread, which may block.
    pub(crate) fn descriptors<T>(&self, mut make: impl FnMut() -> io::Result<T>) -> io::Result<T> {
        demi_command_service::descriptors::retry_blocking(|| {
            self.check()?;
            make()
        })
    }

    /// A copy of `file`'s descriptor, made as `descriptors` makes one.
    pub(crate) fn duplicate(&self, file: &File) -> io::Result<File> {
        self.descriptors(|| file.try_clone())
    }

    pub async fn finish(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }

    pub fn with_cancellation(&self, cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            #[cfg(unix)]
            interrupt: Arc::new(Mutex::new(None)),
            ..self.clone()
        }
    }

    /// The pipe that tells blocking IO about cancellation.
    #[cfg(unix)]
    fn interrupt(&self) -> io::Result<Arc<Interrupt>> {
        let mut slot = self.interrupt.lock().expect("the interrupt slot is intact");
        if let Some(interrupt) = &*slot {
            return Ok(interrupt.clone());
        }
        let interrupt = Arc::new(Interrupt::new(
            &self.cancellation,
            self.descriptors(std::io::pipe)?,
        )?);
        *slot = Some(interrupt.clone());
        Ok(interrupt)
    }

    pub fn read(&self, file: &File, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        #[cfg(feature = "test-fixtures")]
        let _waiting = self.activity.wait();
        self.ready(file, false)?;
        #[cfg(windows)]
        let _operation = self.interruptible_io()?;
        (&*file).read(buffer)
    }

    pub fn write(&self, file: &File, buffer: &[u8]) -> io::Result<usize> {
        self.write_with_tracking(file, buffer, true)
    }

    fn write_with_tracking(&self, file: &File, buffer: &[u8], tracking: bool) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        #[cfg(feature = "test-fixtures")]
        let _waiting = self.activity.wait();
        self.ready(file, true)?;
        // Bound pipe writes so cancellation remains observable under backpressure.
        #[cfg(windows)]
        let _operation = self.interruptible_io()?;
        if tracking && let (Some(edits), Some(path)) = (&self.edits, self.file_path(file)) {
            return edits.record(&path, || (&*file).write(buffer));
        }
        // A pipe that polls writable takes this much without blocking, so
        // cancellation stays observable under backpressure.
        #[cfg(unix)]
        let bound = libc::PIPE_BUF;
        #[cfg(windows)]
        let bound = 512;
        (&*file).write(&buffer[..buffer.len().min(bound)])
    }

    fn open_file(
        &self,
        path: &Path,
        options: &std::fs::OpenOptions,
        writing: bool,
    ) -> io::Result<File> {
        self.check()?;
        let open = || self.descriptors(|| options.open(path));
        let file = match (&self.edits, writing) {
            (Some(edits), true) => edits.record(path, open)?,
            _ => open()?,
        };
        if writing
            && self.edits.is_some()
            && file.metadata().is_ok_and(|metadata| metadata.is_file())
            && let Ok(identity) = file_identity(&file)
        {
            let mut files = self.files.lock().unwrap();
            files.retain(|_, previous| previous != path);
            if files.len() < demi_command_service::protocol::EDIT_JOB_FILES {
                files.insert(identity, path.to_owned());
            }
        }
        Ok(file)
    }

    fn file_path(&self, file: &File) -> Option<PathBuf> {
        let identity = file_identity(file).ok()?;
        let path = self.files.lock().unwrap().get(&identity).cloned()?;
        // A rename can replace the path while the old descriptor remains open.
        let current = File::open(&path).ok()?;
        (file_identity(&current).ok()? == identity).then_some(path)
    }

    fn edit(&self, path: &Path) -> Option<Box<dyn Send>> {
        if std::fs::metadata(path).is_ok_and(|metadata| !metadata.is_file()) {
            return None;
        }
        let mut recording = self.edits.as_ref()?.begin()?;
        recording.track(path);
        Some(Box::new(recording))
    }

    /// Waits until `file` can be read or written, or the scope is cancelled.
    #[cfg(unix)]
    fn ready(&self, file: &File, writing: bool) -> io::Result<()> {
        use rustix::event::{PollFd, PollFlags};
        self.check()?;
        let interrupt = self.interrupt()?;
        let events = if writing { PollFlags::OUT } else { PollFlags::IN };
        loop {
            let mut descriptors = [
                PollFd::new(file, events),
                PollFd::new(&interrupt.reader, PollFlags::IN),
            ];
            match rustix::event::poll(&mut descriptors, None) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            if !descriptors[1].revents().is_empty() {
                return Err(io::Error::other(Cancelled));
            }
            if !descriptors[0].revents().is_empty() {
                return Ok(());
            }
        }
    }

    #[cfg(windows)]
    fn ready(&self, file: &File, writing: bool) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
            System::Pipes::PeekNamedPipe,
        };
        self.check()?;
        if writing || unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_PIPE {
            return Ok(());
        }
        loop {
            self.check()?;
            let mut available = 0;
            if unsafe {
                PeekNamedPipe(
                    file.as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                // Read reports EOF or the concrete pipe error.
                return Ok(());
            }
            if available > 0 {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[cfg(windows)]
    fn interruptible_io(&self) -> io::Result<tokio_util::sync::DropGuard> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::{
            IO::CancelSynchronousIo,
            Threading::{GetCurrentThreadId, OpenThread, THREAD_TERMINATE},
        };
        self.check()?;
        let thread = unsafe { OpenThread(THREAD_TERMINATE, 0, GetCurrentThreadId()) };
        if thread.is_null() {
            return Err(io::Error::last_os_error());
        }
        let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
        let finished = CancellationToken::new();
        let guard = finished.clone().drop_guard();
        let cancellation = self.cancellation.clone();
        self.tasks.spawn(async move {
            tokio::select! {
                biased;
                _ = finished.cancelled() => return,
                _ = cancellation.cancelled() => {},
            }
            loop {
                // Cancellation may arrive just before the worker enters ReadFile
                // or WriteFile. Retry until the operation's guard is released.
                // ERROR_NOT_FOUND means that no synchronous operation is pending.
                if unsafe { CancelSynchronousIo(thread.as_raw_handle()) } == 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error()
                        != Some(windows_sys::Win32::Foundation::ERROR_NOT_FOUND as i32)
                    {
                        tracing::warn!(
                            "shell IO cancellation failed: {error}"
                        );
                        return;
                    }
                }
                tokio::select! {
                    _ = finished.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {},
                }
            }
        });
        Ok(guard)
    }

    /// Sleeps for `duration` unless the scope is cancelled first.
    #[cfg(unix)]
    pub fn sleep(&self, duration: Duration) -> io::Result<()> {
        use rustix::event::{PollFd, PollFlags, Timespec};
        #[cfg(feature = "test-fixtures")]
        let _waiting = self.activity.wait();
        self.check()?;
        let interrupt = self.interrupt()?;
        let deadline = std::time::Instant::now() + duration;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            let timeout = Timespec::try_from(remaining).map_err(io::Error::other)?;
            let mut descriptors = [PollFd::new(&interrupt.reader, PollFlags::IN)];
            match rustix::event::poll(&mut descriptors, Some(&timeout)) {
                Ok(0) | Err(rustix::io::Errno::INTR) => {}
                Ok(_) => return Err(io::Error::other(Cancelled)),
                Err(error) => return Err(error.into()),
            }
        }
    }

    #[cfg(windows)]
    pub fn sleep(&self, duration: Duration) -> io::Result<()> {
        #[cfg(feature = "test-fixtures")]
        let _waiting = self.activity.wait();
        let started = std::time::Instant::now();
        loop {
            self.check()?;
            let remaining = duration.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Ok(());
            }
            std::thread::sleep(remaining.min(Duration::from_millis(25)));
        }
    }
}

impl FileControl for Scope {
    fn duplicate(&self, file: &File) -> io::Result<File> {
        self.duplicate(file)
    }
    fn read(&self, file: &File, buffer: &mut [u8]) -> io::Result<usize> {
        self.read(file, buffer)
    }

    fn write(&self, file: &File, buffer: &[u8]) -> io::Result<usize> {
        self.write(file, buffer)
    }
}

impl ExecutionHost for Scope {
    fn open_file(
        &self,
        path: &Path,
        options: &std::fs::OpenOptions,
        writing: bool,
    ) -> io::Result<File> {
        self.open_file(path, options, writing)
    }

    fn external_output(
        &self,
        file: File,
    ) -> io::Result<(File, Option<brush_core::execution_host::OutputCompletion>)> {
        if self.file_path(&file).is_none() {
            return Ok((file, None));
        }
        let (reader, writer) = self.descriptors(super::job::pipe)?;
        let scope = self.clone();
        let completion = self.tasks.spawn_blocking(move || -> io::Result<()> {
            let mut buffer = vec![0; 64 * 1024];
            loop {
                let count = scope.read(&reader, &mut buffer)?;
                if count == 0 {
                    return Ok(());
                }
                let mut bytes = &buffer[..count];
                while !bytes.is_empty() {
                    let count = scope.write(&file, bytes)?;
                    if count == 0 {
                        return Err(io::ErrorKind::WriteZero.into());
                    }
                    bytes = &bytes[count..];
                }
            }
        });
        Ok((
            writer,
            Some(Box::pin(async move {
                completion.await.map_err(io::Error::other)?
            })),
        ))
    }
    fn check(&self) -> io::Result<()> {
        self.check()
    }

    fn task_guard(&self) -> Box<dyn Send + Sync> {
        Box::new(self.tasks.token())
    }

    fn file_control(&self) -> Arc<dyn FileControl> {
        Arc::new(self.clone())
    }

    fn resolve_path(&self, path: &Path, cwd: &Path) -> PathBuf {
        resolve_path(path, cwd)
    }

    fn pipe(&self) -> io::Result<(std::io::PipeReader, std::io::PipeWriter)> {
        self.descriptors(std::io::pipe)
    }

    fn temporary_file(&self) -> io::Result<File> {
        self.descriptors(tempfile::tempfile)
    }

    fn spawn(&self, mut command: Command, attributes: &ChildAttributes) -> io::Result<ChildProcess> {
        if let Some(context) = &self.commands {
            command.env(
                crate::commands::command_client::CONTEXT_ENV,
                &context.execution.id,
            );
        }
        let command = tokio::process::Command::from(command);
        let mut command = crate::process::wrap(command, true, attributes);
        // A start that waits (`crate::process::start`) ends with the job.
        let mut child = crate::process::start_blocking(|| {
            self.check()?;
            command.spawn()
        })?;
        let pid = child.id().expect("new child has a PID") as i32;
        let cancellation = self.cancellation.clone();
        let (result, receiver) = tokio::sync::oneshot::channel();
        self.tasks.spawn(async move {
            let mut failure = None;
            let status = tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    if let Err(error) = crate::process::kill(child.as_mut()) {
                        failure = Some(error);
                    }
                    child.wait().await
                }
                status = child.wait() => status,
            };
            if let Err(error) = crate::process::kill(child.as_mut()) {
                failure = Some(error);
            }
            let outcome = match (status, failure) {
                (Ok(status), None) => Ok(std::process::Output {
                    status,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                }),
                (Err(error), _) | (_, Some(error)) => Err(error),
            };
            // The shell may have failed before waiting; the owner still reaps the child.
            let _receiver_closed = result.send(outcome);
        });
        Ok(ChildProcess::managed(
            pid,
            Box::pin(async move { receiver.await.map_err(io::Error::other)? }),
        ))
    }
}

pub fn resolve_path(path: &Path, cwd: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(text) = path.to_str() {
        let bytes = text.as_bytes();
        if bytes.len() >= 2
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && (bytes.len() == 2 || bytes[2] == b'/')
        {
            return PathBuf::from(format!(
                "{}:/{}",
                bytes[1] as char,
                text.get(3..).unwrap_or("")
            ));
        }
    }
    cwd.join(path)
}

/// What a standard utility runs with: its job's scope, and the attributes of
/// the shell that ran it for the programs it starts.
pub(crate) struct UtilityControl {
    pub(crate) scope: Scope,
    pub(crate) attributes: ChildAttributes,
}

impl uucore::context::Control for UtilityControl {
    fn open(&self, path: &Path, options: &std::fs::OpenOptions, writing: bool) -> io::Result<File> {
        self.scope
            .open_file(path, options, writing && tracked_utility())
    }
    fn edit(&self, path: &Path) -> Option<Box<dyn Send>> {
        tracked_utility().then(|| self.scope.edit(path)).flatten()
    }
    fn edit_file(&self, file: &File) -> Option<Box<dyn Send>> {
        if !tracked_utility() {
            return None;
        }
        self.scope.edit(&self.scope.file_path(file)?)
    }
    fn check(&self) -> io::Result<()> {
        self.scope.check()
    }
    fn duplicate(&self, file: &File) -> io::Result<File> {
        self.scope.duplicate(file)
    }
    fn read(&self, file: &File, bytes: &mut [u8]) -> io::Result<usize> {
        self.scope.read(file, bytes)
    }
    fn write(&self, file: &File, bytes: &[u8]) -> io::Result<usize> {
        self.scope
            .write_with_tracking(file, bytes, tracked_utility())
    }
    fn sleep(&self, duration: Duration) -> io::Result<()> {
        self.scope.sleep(duration)
    }
    fn resolve(&self, path: &Path, cwd: &Path) -> PathBuf {
        resolve_path(path, cwd)
    }
    fn task_guard(&self) -> Box<dyn Send + Sync> {
        Box::new(self.scope.tasks.token())
    }
    /// A utility's child program, such as `env`'s or `xargs`'s, starts as
    /// the job's own commands do, with the attributes of the shell that ran
    /// the utility.
    fn spawn(
        &self,
        command: &mut process_wrap::std::CommandWrap,
    ) -> io::Result<Box<dyn process_wrap::std::ChildWrapper>> {
        #[cfg(unix)]
        crate::process::set_attributes(command.command_mut(), &self.attributes);
        crate::process::start_blocking(|| {
            self.scope.check()?;
            command.spawn()
        })
    }
}

fn tracked_utility() -> bool {
    !matches!(
        uucore::context::utility_name(),
        Some("cp" | "mv" | "mktemp" | "touch")
    )
}

#[derive(Eq, Hash, PartialEq)]
struct FileIdentity(u64, u64);

#[cfg(unix)]
fn file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok(FileIdentity(metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FileIdentity(
        u64::from(info.dwVolumeSerialNumber),
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    ))
}
