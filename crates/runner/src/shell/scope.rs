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
    execution_host::{ExecutionHost, FileControl},
    processes::ChildProcess,
};
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::commands::{contexts::ExecutionContext, dispatch::Dispatcher};

#[derive(Debug, thiserror::Error)]
#[error("shell job cancelled")]
pub struct Cancelled;

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
    files: Arc<Mutex<HashMap<FileIdentity, PathBuf>>>,
}

impl Scope {
    pub fn new(cancellation: CancellationToken, commands: Option<CommandContext>) -> Self {
        Self {
            cancellation,
            tasks: TaskTracker::new(),
            commands,
            edits: None,
            files: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn check(&self) -> io::Result<()> {
        if self.cancellation.is_cancelled() {
            // Read/Write helpers retry Interrupted; cancellation must escape them.
            Err(io::Error::other(Cancelled))
        } else {
            Ok(())
        }
    }

    pub async fn finish(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }

    pub fn with_cancellation(&self, cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            ..self.clone()
        }
    }

    pub fn read(&self, file: &File, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
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
        self.ready(file, true)?;
        // Bound pipe writes so cancellation remains observable under backpressure.
        #[cfg(windows)]
        let _operation = self.interruptible_io()?;
        if tracking && let (Some(edits), Some(path)) = (&self.edits, self.file_path(file)) {
            return edits.record(&path, || (&*file).write(buffer));
        }
        (&*file).write(&buffer[..buffer.len().min(512)])
    }

    fn open_file(
        &self,
        path: &Path,
        options: &std::fs::OpenOptions,
        writing: bool,
    ) -> io::Result<File> {
        self.check()?;
        // Out of open files, a redirection or utility waits for one
        // (`runner.md` § Load).
        let open = || demi_command_service::descriptors::retry_blocking(|| options.open(path));
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

    #[cfg(unix)]
    fn ready(&self, file: &File, writing: bool) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        loop {
            self.check()?;
            let mut descriptor = libc::pollfd {
                fd: file.as_raw_fd(),
                events: if writing { libc::POLLOUT } else { libc::POLLIN },
                revents: 0,
            };
            let count = unsafe { libc::poll(&mut descriptor, 1, 25) };
            if count > 0 {
                return Ok(());
            }
            if count < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
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
                        eprintln!("shell IO cancellation failed: {error}");
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

    pub fn sleep(&self, duration: Duration) -> io::Result<()> {
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
        let (reader, writer) = super::job::pipe()?;
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

    /// Out of open files, a pipe waits for one (`runner.md` § Load).
    fn pipe(&self) -> io::Result<(std::io::PipeReader, std::io::PipeWriter)> {
        demi_command_service::descriptors::retry_blocking(std::io::pipe)
    }

    fn spawn(&self, mut command: Command) -> io::Result<ChildProcess> {
        self.check()?;
        if let Some(context) = &self.commands {
            command.env(
                crate::commands::command_client::CONTEXT_ENV,
                &context.execution.id,
            );
        }
        let mut command = CommandWrap::from(tokio::process::Command::from(command));
        command.wrap(KillOnDrop);
        #[cfg(unix)]
        command.wrap(process_wrap::tokio::ProcessGroup::leader());
        #[cfg(windows)]
        command.wrap(process_wrap::tokio::JobObject);
        // Out of open files, the external command waits for one (`runner.md` § Load).
        let mut child = demi_command_service::descriptors::retry_blocking(|| command.spawn())?;
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

impl uucore::context::Control for Scope {
    fn open(&self, path: &Path, options: &std::fs::OpenOptions, writing: bool) -> io::Result<File> {
        self.open_file(path, options, writing && tracked_utility())
    }
    fn edit(&self, path: &Path) -> Option<Box<dyn Send>> {
        tracked_utility().then(|| self.edit(path)).flatten()
    }
    fn edit_file(&self, file: &File) -> Option<Box<dyn Send>> {
        if !tracked_utility() {
            return None;
        }
        self.edit(&self.file_path(file)?)
    }
    fn check(&self) -> io::Result<()> {
        self.check()
    }
    fn read(&self, file: &File, bytes: &mut [u8]) -> io::Result<usize> {
        self.read(file, bytes)
    }
    fn write(&self, file: &File, bytes: &[u8]) -> io::Result<usize> {
        self.write_with_tracking(file, bytes, tracked_utility())
    }
    fn sleep(&self, duration: Duration) -> io::Result<()> {
        self.sleep(duration)
    }
    fn resolve(&self, path: &Path, cwd: &Path) -> PathBuf {
        resolve_path(path, cwd)
    }
    fn task_guard(&self) -> Box<dyn Send + Sync> {
        Box::new(self.tasks.token())
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
