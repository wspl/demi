//! Explicit execution context for in-process utility invocations.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Execution ownership injected by an embedding host.
pub trait Control: Send + Sync {
    fn open(&self, path: &Path, options: &std::fs::OpenOptions, writing: bool) -> std::io::Result<File> {
        let _ = writing;
        options.open(path)
    }
    /// A guard spans one bounded file operation, including its error paths.
    fn edit(&self, _path: &Path) -> Option<Box<dyn Send>> {
        None
    }
    fn edit_file(&self, _file: &File) -> Option<Box<dyn Send>> {
        None
    }
    fn check(&self) -> std::io::Result<()>;
    fn read(&self, file: &File, bytes: &mut [u8]) -> std::io::Result<usize>;
    fn write(&self, file: &File, bytes: &[u8]) -> std::io::Result<usize>;
    fn sleep(&self, duration: std::time::Duration) -> std::io::Result<()>;
    fn resolve(&self, path: &Path, cwd: &Path) -> PathBuf;
    fn task_guard(&self) -> Box<dyn Send + Sync>;
}

/// Cancellation unwinds one utility invocation, never the embedding process.
#[derive(Debug)]
pub struct Cancelled;

pub fn control() -> Option<Arc<dyn Control>> {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .and_then(|context| context.control.clone())
    })
}

pub fn check_cancelled() {
    if control().is_some_and(|control| control.check().is_err()) {
        std::panic::resume_unwind(Box::new(Cancelled));
    }
}

pub fn read(file: &File, bytes: &mut [u8]) -> std::io::Result<usize> {
    use std::io::Read;
    check_cancelled();
    match control() {
        Some(control) => control.read(file, bytes),
        None => (&*file).read(bytes),
    }
}

pub fn write(file: &File, bytes: &[u8]) -> std::io::Result<usize> {
    use std::io::Write;
    check_cancelled();
    match control() {
        Some(control) => control.write(file, bytes),
        None => (&*file).write(bytes),
    }
}

pub fn edit(path: &Path) -> Option<Box<dyn Send>> {
    control().and_then(|control| control.edit(path))
}

#[derive(Clone)]
pub struct Context {
    pub name: &'static str,
    pub control: Option<Arc<dyn Control>>,
    pub live_input: bool,
    pub umask: u32,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub descriptors: BTreeMap<i32, Arc<File>>,
    pub stdin: Arc<File>,
    pub stdout: Arc<File>,
    pub stderr: Arc<File>,
}

thread_local! {
    static CURRENT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

pub fn with<T>(context: Context, function: impl FnOnce() -> T) -> T {
    struct Restore(Option<Context>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|current| {
                current.replace(self.0.take());
            });
        }
    }
    let _restore = Restore(CURRENT.with(|current| current.replace(Some(context))));
    crate::error::set_exit_code(0);
    function()
}

pub fn utility_name() -> Option<&'static str> {
    CURRENT.with(|current| current.borrow().as_ref().map(|context| context.name))
}

pub fn resolve(path: impl AsRef<Path>) -> PathBuf {
    check_cancelled();
    let path = path.as_ref();
    #[cfg(unix)]
    if let Some(file) = descriptor(path) {
        use std::os::fd::AsRawFd;
        return PathBuf::from(format!("/dev/fd/{}", file.as_raw_fd()));
    }
    CURRENT.with(|current| {
        let current = current.borrow();
        let context = current.as_ref().expect("utility execution context");
        match &context.control {
            Some(control) => control.resolve(path, &context.cwd),
            None => context.cwd.join(path),
        }
    })
}

/// Shell descriptor paths refer to this invocation's descriptor table.
pub fn descriptor(path: &Path) -> Option<Arc<File>> {
    let fd = match path.to_str()? {
        "/dev/stdin" => 0,
        "/dev/stdout" => 1,
        "/dev/stderr" => 2,
        path => path.strip_prefix("/dev/fd/")?.parse::<i32>().ok()?,
    };
    CURRENT.with(|current| {
        let current = current.borrow();
        let context = current.as_ref()?;
        match fd {
            0 => Some(context.stdin.clone()),
            1 => Some(context.stdout.clone()),
            2 => Some(context.stderr.clone()),
            _ => context.descriptors.get(&fd).cloned(),
        }
    })
}

pub mod io {
    pub use std::io::*;
    use std::{fs::File, sync::Arc};

    #[derive(Clone)]
    pub struct Stdio<const N: u8>(Arc<File>);
    pub type Stdin = Stdio<0>;
    pub type Stdout = Stdio<1>;
    pub type Stderr = Stdio<2>;
    pub struct StdinLock<'a> {
        reader: BufReader<Stdin>,
        marker: std::marker::PhantomData<&'a Stdin>,
    }
    pub struct OutputLock<'a, const N: u8> {
        stream: Stdio<N>,
        marker: std::marker::PhantomData<&'a Stdio<N>>,
    }
    pub type StdoutLock<'a> = OutputLock<'a, 1>;
    pub type StderrLock<'a> = OutputLock<'a, 2>;

    pub fn stdin() -> Stdin {
        stream()
    }
    pub fn stdout() -> Stdout {
        stream()
    }
    pub fn stderr() -> Stderr {
        stream()
    }

    fn stream<const N: u8>() -> Stdio<N> {
        super::CURRENT.with(|current| {
            let current = current.borrow();
            let context = current.as_ref().expect("utility execution context");
            Stdio(match N {
                0 => context.stdin.clone(),
                1 => context.stdout.clone(),
                _ => context.stderr.clone(),
            })
        })
    }

    impl Stdin {
        pub fn lock(&self) -> StdinLock<'static> {
            StdinLock {
                reader: BufReader::new(self.clone()),
                marker: std::marker::PhantomData,
            }
        }
        pub fn read_line(&self, line: &mut String) -> Result<usize> {
            let mut bytes = Vec::new();
            let mut byte = [0];
            loop {
                let size = super::read(&self.0, &mut byte)?;
                if size == 0 {
                    break;
                }
                bytes.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            let size = bytes.len();
            line.push_str(
                std::str::from_utf8(&bytes)
                    .map_err(|error| Error::new(ErrorKind::InvalidData, error))?,
            );
            Ok(size)
        }
    }
    impl Read for StdinLock<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
            self.reader.read(buffer)
        }
    }
    impl BufRead for StdinLock<'_> {
        fn fill_buf(&mut self) -> Result<&[u8]> {
            self.reader.fill_buf()
        }
        fn consume(&mut self, amount: usize) {
            self.reader.consume(amount);
        }
    }
    #[cfg(unix)]
    impl std::os::fd::AsFd for StdinLock<'_> {
        fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
            self.reader.get_ref().as_fd()
        }
    }
    #[cfg(unix)]
    impl std::os::fd::AsRawFd for StdinLock<'_> {
        fn as_raw_fd(&self) -> std::os::fd::RawFd {
            self.reader.get_ref().as_raw_fd()
        }
    }
    #[cfg(windows)]
    impl std::os::windows::io::AsHandle for StdinLock<'_> {
        fn as_handle(&self) -> std::os::windows::io::BorrowedHandle<'_> {
            self.reader.get_ref().as_handle()
        }
    }
    #[cfg(windows)]
    impl std::os::windows::io::AsRawHandle for StdinLock<'_> {
        fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
            self.reader.get_ref().as_raw_handle()
        }
    }
    impl Stdout {
        pub fn lock(&self) -> StdoutLock<'static> {
            OutputLock {
                stream: self.clone(),
                marker: std::marker::PhantomData,
            }
        }
    }
    impl Stderr {
        pub fn lock(&self) -> StderrLock<'static> {
            OutputLock {
                stream: self.clone(),
                marker: std::marker::PhantomData,
            }
        }
    }
    impl<const N: u8> Read for Stdio<N> {
        fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
            super::read(&self.0, buffer)
        }
    }
    impl<const N: u8> Write for Stdio<N> {
        fn write(&mut self, buffer: &[u8]) -> Result<usize> {
            super::write(&self.0, buffer)
        }
        fn flush(&mut self) -> Result<()> {
            (&*self.0).flush()
        }
    }
    pub trait IsTerminal {
        fn is_terminal(&self) -> bool;
    }
    impl<T: std::io::IsTerminal> IsTerminal for T {
        fn is_terminal(&self) -> bool {
            std::io::IsTerminal::is_terminal(self)
        }
    }
    impl<const N: u8> IsTerminal for Stdio<N> {
        fn is_terminal(&self) -> bool {
            std::io::IsTerminal::is_terminal(&*self.0)
        }
    }
    impl IsTerminal for super::fs::File {
        fn is_terminal(&self) -> bool {
            std::io::IsTerminal::is_terminal(&**self)
        }
    }
    impl IsTerminal for StdinLock<'_> {
        fn is_terminal(&self) -> bool {
            self.reader.get_ref().is_terminal()
        }
    }
    impl<const N: u8> IsTerminal for OutputLock<'_, N> {
        fn is_terminal(&self) -> bool {
            self.stream.is_terminal()
        }
    }
    impl<const N: u8> Write for OutputLock<'_, N> {
        fn write(&mut self, buffer: &[u8]) -> Result<usize> {
            self.stream.write(buffer)
        }
        fn flush(&mut self) -> Result<()> {
            self.stream.flush()
        }
    }
    #[cfg(unix)]
    impl<const N: u8> std::os::fd::AsFd for OutputLock<'_, N> {
        fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
            self.stream.as_fd()
        }
    }
    #[cfg(unix)]
    impl<const N: u8> std::os::fd::AsRawFd for OutputLock<'_, N> {
        fn as_raw_fd(&self) -> std::os::fd::RawFd {
            self.stream.as_raw_fd()
        }
    }
    #[cfg(windows)]
    impl<const N: u8> std::os::windows::io::AsHandle for OutputLock<'_, N> {
        fn as_handle(&self) -> std::os::windows::io::BorrowedHandle<'_> {
            self.stream.as_handle()
        }
    }
    #[cfg(windows)]
    impl<const N: u8> std::os::windows::io::AsRawHandle for OutputLock<'_, N> {
        fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
            self.stream.as_raw_handle()
        }
    }
    #[cfg(unix)]
    impl<const N: u8> std::os::fd::AsFd for Stdio<N> {
        fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
            self.0.as_fd()
        }
    }
    #[cfg(unix)]
    impl<const N: u8> std::os::fd::AsRawFd for Stdio<N> {
        fn as_raw_fd(&self) -> std::os::fd::RawFd {
            self.0.as_raw_fd()
        }
    }
    #[cfg(windows)]
    impl<const N: u8> std::os::windows::io::AsHandle for Stdio<N> {
        fn as_handle(&self) -> std::os::windows::io::BorrowedHandle<'_> {
            self.0.as_handle()
        }
    }
    #[cfg(windows)]
    impl<const N: u8> std::os::windows::io::AsRawHandle for Stdio<N> {
        fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
            self.0.as_raw_handle()
        }
    }
}

pub mod env;
pub mod fs;
pub mod process;

/// An intentional utility exit unwinds only its invocation thread.
#[derive(Debug)]
pub struct ExitRequest(pub i32);

pub fn exit(code: i32) -> ! {
    std::panic::resume_unwind(Box::new(ExitRequest(code)))
}

pub fn print(arguments: std::fmt::Arguments<'_>, stderr: bool) {
    use std::io::Write;
    let result = if stderr {
        io::stderr().write_fmt(arguments)
    } else {
        io::stdout().write_fmt(arguments)
    };
    if let Err(error) = result {
        exit(if error.kind() == std::io::ErrorKind::BrokenPipe {
            141
        } else {
            1
        });
    }
}

#[macro_export]
macro_rules! context_print { ($($argument:tt)*) => { $crate::context::print(format_args!($($argument)*), false) }; }
#[macro_export]
macro_rules! context_println {
    () => { $crate::context_print!("\n") };
    ($($argument:tt)*) => { $crate::context_print!("{}\n", format_args!($($argument)*)) };
}
#[macro_export]
macro_rules! context_eprint { ($($argument:tt)*) => { $crate::context::print(format_args!($($argument)*), true) }; }
#[macro_export]
macro_rules! context_eprintln {
    () => { $crate::context_eprint!("\n") };
    ($($argument:tt)*) => { $crate::context_eprint!("{}\n", format_args!($($argument)*)) };
}

/// Filesystem operations on lexical paths resolve against the invocation cwd.
pub trait PathExt {
    fn context_exists(&self) -> bool;
    fn context_try_exists(&self) -> std::io::Result<bool>;
    fn context_symlink_metadata(&self) -> std::io::Result<std::fs::Metadata>;
    fn context_canonicalize(&self) -> std::io::Result<PathBuf>;
    fn context_read_link(&self) -> std::io::Result<PathBuf>;
}
impl<T: AsRef<Path> + ?Sized> PathExt for T {
    fn context_exists(&self) -> bool {
        resolve(self).exists()
    }
    fn context_try_exists(&self) -> std::io::Result<bool> {
        resolve(self).try_exists()
    }
    fn context_symlink_metadata(&self) -> std::io::Result<std::fs::Metadata> {
        resolve(self).symlink_metadata()
    }
    fn context_canonicalize(&self) -> std::io::Result<PathBuf> {
        resolve(self).canonicalize()
    }
    fn context_read_link(&self) -> std::io::Result<PathBuf> {
        resolve(self).read_link()
    }
}

/// Utility-owned worker threads inherit explicit invocation resources.
pub mod thread {
    pub use std::thread::{JoinHandle, available_parallelism, current, yield_now};

    pub fn sleep(duration: std::time::Duration) {
        if let Some(control) = super::control() {
            if control.sleep(duration).is_err() {
                std::panic::resume_unwind(Box::new(super::Cancelled));
            }
        } else {
            std::thread::sleep(duration);
        }
    }

    pub fn spawn<F, T>(function: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let context =
            super::CURRENT.with(|current| current.borrow().clone().expect("utility context"));
        let guard = context.control.as_ref().map(|control| control.task_guard());
        std::thread::spawn(move || {
            super::with(context.clone(), || {
                let _guard = guard;
                crate::locale::setup_localization(context.name)
                    .expect("utility worker localization");
                function()
            })
        })
    }
}

/// File kind queries distinguish lexical paths from already-read metadata.
pub trait FileKindExt {
    fn context_is_dir(&self) -> bool;
    fn context_is_file(&self) -> bool;
    fn context_is_symlink(&self) -> bool;
}
impl FileKindExt for Path {
    fn context_is_dir(&self) -> bool {
        resolve(self).is_dir()
    }
    fn context_is_file(&self) -> bool {
        resolve(self).is_file()
    }
    fn context_is_symlink(&self) -> bool {
        resolve(self).is_symlink()
    }
}
impl FileKindExt for std::fs::Metadata {
    fn context_is_dir(&self) -> bool {
        self.is_dir()
    }
    fn context_is_file(&self) -> bool {
        self.is_file()
    }
    fn context_is_symlink(&self) -> bool {
        self.is_symlink()
    }
}
impl FileKindExt for std::fs::FileType {
    fn context_is_dir(&self) -> bool {
        self.is_dir()
    }
    fn context_is_file(&self) -> bool {
        self.is_file()
    }
    fn context_is_symlink(&self) -> bool {
        self.is_symlink()
    }
}

pub fn umask() -> u32 {
    CURRENT.with(|current| current.borrow().as_ref().expect("utility context").umask)
}

/// Capture resources when an upstream library creates its own worker threads.
pub fn snapshot() -> Context {
    CURRENT.with(|current| current.borrow().as_ref().expect("utility context").clone())
}
