//! The platform's file system watches (`runner.md` § Watching files,
//! § Working tree), each numbering the events it receives as it receives
//! them (`crate::numbers`); reading a file changes nothing. A watch reaches
//! one of three ways:
//!
//! - **A tree**, everything below each of its trees. On macOS it is one
//!   FSEvents stream whatever the trees' size, which leaves out up to eight
//!   large folders git ignores: notify's recommended backend there is
//!   kqueue, which the vendored `tail` selects and which holds a descriptor
//!   for every file of a tree, more than a repository and the usual limit of
//!   256 allow; with kqueue selected, notify builds no FSEvents watcher. On
//!   Linux it is inotify on each folder git does not ignore; elsewhere
//!   notify's recommended backend.
//! - **A node**, one file or folder alone: on macOS a kqueue of its own,
//!   watched again under its name when a rename replaces it; elsewhere
//!   notify on its folder, or on the folder of a file.
//! - **Entries**, a folder and its entries' contents: on macOS an FSEvents
//!   stream over the folder, never over its parent's tree; elsewhere notify
//!   on the folder.
//!
//! A tree in a repository says of each path whether git ignores it, by the
//! repository's rules (`crate::ignore`).

use std::path::{Path, PathBuf};

use crate::ignore::IgnoreFilter;

/// What a watch saw, numbered as it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Event {
    pub(crate) at: u64,
    pub(crate) change: Change,
}

/// What changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Change {
    /// Something changed at `path`; `metadata` when only its metadata did,
    /// such as its mode or its times; `entry` when the path came, went or
    /// was renamed, or the platform's event leaves that in doubt; `ignored`
    /// when git ignores it in the tree's repository.
    Changed {
        path: PathBuf,
        metadata: bool,
        entry: bool,
        ignored: bool,
    },
    /// The watch lost events: below `under`, or anywhere it watches without
    /// one. `ignored` when git ignores what lies there.
    Lost { under: Option<PathBuf>, ignored: bool },
    /// The watch failed, for this reason, and reports nothing more.
    Failed(String),
}

/// What a watch hears, on the watch's own thread.
pub(crate) type Report = Box<dyn FnMut(&Event) + Send>;

/// How far a watch reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Reach {
    /// Each tree and everything below it.
    Tree,
    /// One file, or one folder's names; one path only.
    Node,
    /// One folder, its entries and their contents; one path only.
    Entries,
}

/// The most folders a tree's stream leaves out
/// (`FSEventStreamSetExclusionPaths` takes eight at most).
pub(crate) const MAX_LEFT_OUT: usize = 8;

/// A running watch; it stops when dropped.
pub(crate) struct TreeWatch {
    _watch: platform::Watch,
    left_out: Vec<PathBuf>,
}

impl TreeWatch {
    /// Watches `trees` as far as `reach` says, handing `report` what
    /// changes, judged by `filter` for a tree in a repository; why it could
    /// not when the watch cannot be set up.
    pub(crate) fn start(
        trees: &[PathBuf],
        reach: Reach,
        filter: Option<IgnoreFilter>,
        report: Report,
    ) -> Result<TreeWatch, String> {
        let mut listener = Listener {
            report,
            filter,
            #[cfg(target_os = "macos")]
            arrivals: (reach != Reach::Node).then(Arrivals::default),
            scope: match (reach, trees) {
                (Reach::Tree, _) => None,
                (_, [path]) => Some(path.clone()),
                _ => return Err("a watch of one path covers one path".into()),
            },
        };
        let left_out = match (reach, listener.filter.as_mut(), trees.first()) {
            (Reach::Tree, Some(filter), Some(root)) => filter.large_ignored(root, MAX_LEFT_OUT),
            _ => Vec::new(),
        };
        let watch = platform::Watch::start(trees, reach, &left_out, listener)?;
        Ok(TreeWatch {
            _watch: watch,
            left_out,
        })
    }

    /// The folders below the tree the watch leaves out: what lies in them
    /// is watched alone.
    pub(crate) fn left_out(&self) -> &[PathBuf] {
        &self.left_out
    }
}

/// What the platform said of one path, before it is judged and numbered.
#[derive(Debug)]
enum Raw {
    Changed {
        path: PathBuf,
        metadata: bool,
        entry: bool,
        /// Whether it is a folder, when the platform says.
        folder: Option<bool>,
    },
    Lost {
        under: Option<PathBuf>,
    },
    #[cfg_attr(
        target_os = "macos",
        allow(dead_code, reason = "FSEvents and kqueue report no failure once they run")
    )]
    Failed(String),
}

/// A watch's end on its own thread: it keeps what concerns the watch,
/// judges it by git's rules and numbers it.
struct Listener {
    report: Report,
    filter: Option<IgnoreFilter>,
    /// What an FSEvents stream saw come, which tells its flags' doubt; a
    /// kqueue's events leave none.
    #[cfg(target_os = "macos")]
    arrivals: Option<Arrivals>,
    /// The one path a node or entries watch covers; none for a tree.
    scope: Option<PathBuf>,
}

impl Listener {
    /// Hands on what the platform said, in its order, each numbered as it
    /// goes.
    fn deliver(&mut self, raws: Vec<Raw>) {
        let raws: Vec<Raw> = match &self.scope {
            None => raws,
            Some(scope) => raws.into_iter().filter(|raw| concerns(scope, raw)).collect(),
        };
        if raws.is_empty() {
            return;
        }
        let ignored = match &mut self.filter {
            Some(filter) => {
                let asked: Vec<(&Path, Option<bool>)> = raws
                    .iter()
                    .filter_map(|raw| match raw {
                        Raw::Changed { path, folder, .. } => Some((path.as_path(), *folder)),
                        Raw::Lost { under: Some(under) } => Some((under.as_path(), Some(true))),
                        Raw::Lost { under: None } | Raw::Failed(_) => None,
                    })
                    .collect();
                filter.ignored(&asked)
            }
            None => Vec::new(),
        };
        let mut ignored = ignored.into_iter();
        for raw in raws {
            let change = match raw {
                Raw::Changed {
                    path,
                    metadata,
                    entry,
                    ..
                } => {
                    let ignored = ignored.next().unwrap_or(false);
                    // An ignored path's doubt is not worth a look at the disk.
                    #[cfg(target_os = "macos")]
                    let entry = match &mut self.arrivals {
                        Some(arrivals) if entry && !ignored => arrivals.confirm(&path),
                        _ => entry,
                    };
                    Change::Changed {
                        path,
                        metadata,
                        entry,
                        ignored,
                    }
                }
                Raw::Lost { under } => {
                    let ignored = under.is_some() && ignored.next().unwrap_or(false);
                    Change::Lost { under, ignored }
                }
                Raw::Failed(reason) => Change::Failed(reason),
            };
            let event = Event {
                at: crate::numbers::take(),
                change,
            };
            (self.report)(&event);
        }
    }
}

/// Whether what the platform said concerns the one path a watch covers:
/// the path, or an entry of it.
fn concerns(scope: &Path, raw: &Raw) -> bool {
    match raw {
        Raw::Changed { path, .. } => path == scope || path.parent() == Some(scope),
        Raw::Lost { under: Some(under) } => under == scope || scope.starts_with(under) || under.parent() == Some(scope),
        Raw::Lost { under: None } | Raw::Failed(_) => true,
    }
}

/// What a notify event reports: nothing for a file opened, read or closed.
#[cfg(not(target_os = "macos"))]
fn from_notify(event: notify::Event) -> Vec<Raw> {
    use notify::{EventKind, event::ModifyKind};
    if event.need_rescan() {
        return vec![Raw::Lost { under: None }];
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return Vec::new();
    }
    let metadata = matches!(event.kind, EventKind::Modify(ModifyKind::Metadata(_)));
    // Only a write of content or of metadata is sure to leave the name in
    // place; any other kind, an unknown one included, may have moved it.
    let entry = !matches!(
        event.kind,
        EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_))
    );
    let folder = match event.kind {
        EventKind::Create(notify::event::CreateKind::Folder)
        | EventKind::Remove(notify::event::RemoveKind::Folder) => Some(true),
        EventKind::Create(notify::event::CreateKind::File)
        | EventKind::Remove(notify::event::RemoveKind::File) => Some(false),
        _ => None,
    };
    event
        .paths
        .into_iter()
        .map(|path| Raw::Changed {
            path,
            metadata,
            entry,
            folder,
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{Listener, Raw, Reach, from_notify};
    use notify::Watcher as _;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, mpsc};

    pub(super) struct Watch {
        _watcher: Arc<Mutex<notify::RecommendedWatcher>>,
        /// On Linux, the thread that watches each folder a tree gains.
        _folders: Option<FolderAdder>,
    }

    /// Adds the watches of the folders a tree gains, off notify's own
    /// thread, which would wait for itself.
    struct FolderAdder {
        added: mpsc::SyncSender<FolderChange>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    enum FolderChange {
        Added(PathBuf),
        Stop,
    }

    impl Drop for FolderAdder {
        fn drop(&mut self) {
            // The worker owns a watcher whose callback owns another sender,
            // so dropping our sender cannot end its receive loop. Watch
            // destruction runs on the blocking pool, where sending and
            // joining may wait without stopping the control runtime.
            if self.added.send(FolderChange::Stop).is_err() {
                // The worker already exited, so there is nothing to stop.
            }
            if let Some(thread) = self.thread.take() {
                // A thread that panicked has nothing left to release.
                let _ = thread.join();
            }
        }
    }

    impl Watch {
        pub(super) fn start(
            trees: &[PathBuf],
            reach: Reach,
            _left_out: &[PathBuf],
            listener: Listener,
        ) -> Result<Watch, String> {
            let listener = Arc::new(Mutex::new(listener));
            let tree_by_folder = cfg!(target_os = "linux") && reach == Reach::Tree;
            let (added, adding) = mpsc::sync_channel::<FolderChange>(128);
            let handler = {
                let listener = listener.clone();
                let added = added.clone();
                move |event: notify::Result<notify::Event>| {
                    let raws = match event {
                        Ok(event) => from_notify(event),
                        Err(error) => vec![Raw::Failed(error.to_string())],
                    };
                    if tree_by_folder {
                        for raw in &raws {
                            if let Raw::Changed { path, entry: true, .. } = raw
                                && path.is_dir()
                            {
                                match added.try_send(FolderChange::Added(path.clone())) {
                                    Ok(()) => {}
                                    Err(mpsc::TrySendError::Full(_)) => {
                                        lock(&listener).deliver(vec![Raw::Failed(
                                            "The folder watch queue is full".into(),
                                        )]);
                                        return;
                                    }
                                    Err(mpsc::TrySendError::Disconnected(_)) => {
                                        lock(&listener).deliver(vec![Raw::Failed(
                                            "The folder watch thread stopped".into(),
                                        )]);
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    lock(&listener).deliver(raws);
                }
            };
            let watcher = notify::recommended_watcher(handler).map_err(|error| error.to_string())?;
            let watcher = Arc::new(Mutex::new(watcher));
            let mode = if reach == Reach::Tree && !tree_by_folder {
                notify::RecursiveMode::Recursive
            } else {
                notify::RecursiveMode::NonRecursive
            };
            for tree in trees {
                let watched = match reach {
                    // A file is watched through its folder, so a rename
                    // that replaces it is seen under its name.
                    Reach::Node if !tree.is_dir() => tree.parent().unwrap_or(tree).to_path_buf(),
                    _ => tree.clone(),
                };
                lock(&watcher)
                    .watch(&watched, mode)
                    .map_err(|error| error.to_string())?;
                if tree_by_folder {
                    add_below(&watcher, &listener, &watched, false);
                }
            }
            let folders = if tree_by_folder {
                let watcher = watcher.clone();
                let listener = listener.clone();
                let thread = std::thread::Builder::new()
                    .name("inotify-folders".into())
                    .spawn(move || {
                        while let Ok(FolderChange::Added(folder)) = adding.recv() {
                            if lock(&watcher)
                                .watch(&folder, notify::RecursiveMode::NonRecursive)
                                .is_ok()
                            {
                                add_below(&watcher, &listener, &folder, true);
                            }
                        }
                    })
                    .map_err(|error| error.to_string())?;
                Some(FolderAdder {
                    added,
                    thread: Some(thread),
                })
            } else {
                None
            };
            Ok(Watch {
                _watcher: watcher,
                _folders: folders,
            })
        }
    }

    /// Watches each folder below `folder` that git does not ignore; one
    /// that came after its parent's watch also reports what it holds, which
    /// may have come before its own watch did.
    fn add_below(
        watcher: &Mutex<notify::RecommendedWatcher>,
        listener: &Mutex<Listener>,
        folder: &Path,
        came: bool,
    ) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        let mut found = Vec::new();
        let mut folders = Vec::new();
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let is_folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
            if is_folder {
                folders.push(path.clone());
            }
            if came {
                found.push(Raw::Changed {
                    path,
                    metadata: false,
                    entry: true,
                    folder: Some(is_folder),
                });
            }
        }
        if !found.is_empty() {
            lock(listener).deliver(found);
        }
        let kept: Vec<PathBuf> = {
            let mut listener = lock(listener);
            match &mut listener.filter {
                Some(filter) => {
                    let asked: Vec<(&Path, Option<bool>)> =
                        folders.iter().map(|path| (path.as_path(), Some(true))).collect();
                    let ignored = filter.ignored(&asked);
                    folders
                        .into_iter()
                        .zip(ignored)
                        // Git metadata is not ignored: its index and refs
                        // announce staging and commits as worktree edits do.
                        .filter(|(_, ignored)| !ignored)
                        .map(|(path, _)| path)
                        .collect()
                }
                None => folders,
            }
        };
        for folder in kept {
            if lock(watcher)
                .watch(&folder, notify::RecursiveMode::NonRecursive)
                .is_ok()
            {
                add_below(watcher, listener, &folder, false);
            }
        }
    }

    /// A poisoned lock means a panic mid-update, which nothing may paper
    /// over.
    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().expect("the watch is intact")
    }
}

/// What FSEvents' flags for one path report.
#[cfg(target_os = "macos")]
fn from_flags(path: PathBuf, flags: fsevent_sys::FSEventStreamEventFlags) -> Raw {
    use fsevent_sys as fs;
    const STREAM_LOST: fs::FSEventStreamEventFlags = fs::kFSEventStreamEventFlagUserDropped
        | fs::kFSEventStreamEventFlagKernelDropped
        | fs::kFSEventStreamEventFlagRootChanged
        | fs::kFSEventStreamEventFlagMount
        | fs::kFSEventStreamEventFlagUnmount;
    const CONTENT: fs::FSEventStreamEventFlags = fs::kFSEventStreamEventFlagItemCreated
        | fs::kFSEventStreamEventFlagItemRemoved
        | fs::kFSEventStreamEventFlagItemRenamed
        | fs::kFSEventStreamEventFlagItemModified;
    const METADATA: fs::FSEventStreamEventFlags = fs::kFSEventStreamEventFlagItemInodeMetaMod
        | fs::kFSEventStreamEventFlagItemFinderInfoMod
        | fs::kFSEventStreamEventFlagItemChangeOwner
        | fs::kFSEventStreamEventFlagItemXattrMod;
    const ENTRY: fs::FSEventStreamEventFlags = fs::kFSEventStreamEventFlagItemCreated
        | fs::kFSEventStreamEventFlagItemRemoved
        | fs::kFSEventStreamEventFlagItemRenamed;
    if flags & STREAM_LOST != 0 {
        return Raw::Lost { under: None };
    }
    if flags & fs::kFSEventStreamEventFlagMustScanSubDirs != 0 {
        return Raw::Lost { under: Some(path) };
    }
    let metadata = flags & METADATA != 0 && flags & CONTENT == 0;
    // FSEvents merges an item's flags over a short history, so a file
    // created and then written shows both: it counts as having come, unless
    // the stream's `Arrivals` tell it is the file that came before.
    let entry = flags & ENTRY != 0;
    let folder = if flags & fs::kFSEventStreamEventFlagItemIsDir != 0 {
        Some(true)
    } else if flags & (fs::kFSEventStreamEventFlagItemIsFile | fs::kFSEventStreamEventFlagItemIsSymlink) != 0 {
        Some(false)
    } else {
        None
    };
    Raw::Changed {
        path,
        metadata,
        entry,
        folder,
    }
}

/// The most paths a stream remembers the file of; past it, it forgets them
/// all, and the next doubtful event of each counts as one that came.
#[cfg(target_os = "macos")]
const ARRIVALS_KEPT: usize = 65_536;

/// The file each path flagged as come, gone or renamed was, by device and
/// inode, as one stream last saw it. FSEvents keeps a file's creation in
/// its flags for as long as writes to it come without a pause, so a log a
/// build creates and then appends to says on every write that it came
/// (measured: 120 of 120 writes, 250 ms apart, over 30 s). A path still
/// holding the file last seen there has not come, gone or been renamed
/// since, so its folder's listing is not read again for a write in place.
#[cfg(target_os = "macos")]
#[derive(Default)]
struct Arrivals {
    seen: std::collections::HashMap<PathBuf, (u64, u64)>,
}

#[cfg(target_os = "macos")]
impl Arrivals {
    /// Whether the path flagged in doubt came, went or was renamed, by what
    /// is at it now.
    fn confirm(&mut self, path: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        // A path that cannot be read is gone, or not what was seen there.
        let now = std::fs::symlink_metadata(path).ok().map(|file| (file.dev(), file.ino()));
        let same = now.is_some() && self.seen.get(path) == now.as_ref();
        match now {
            Some(file) => {
                if self.seen.len() >= ARRIVALS_KEPT && !self.seen.contains_key(path) {
                    self.seen.clear();
                }
                self.seen.insert(path.to_path_buf(), file);
            }
            None => {
                self.seen.remove(path);
            }
        }
        !same
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{Listener, Reach};
    use std::path::PathBuf;

    /// A stream over trees, or a kqueue of one node, each held for its
    /// drop, which stops it.
    pub(super) enum Watch {
        Stream(#[cfg_attr(not(feature = "testing"), allow(dead_code, reason = "held for its drop"))] super::fsevents::Stream),
        Node(#[allow(dead_code, reason = "held for its drop")] super::kqueue::Node),
    }

    impl Watch {
        pub(super) fn start(
            trees: &[PathBuf],
            reach: Reach,
            left_out: &[PathBuf],
            listener: Listener,
        ) -> Result<Watch, String> {
            match (reach, trees) {
                (Reach::Tree, _) => super::fsevents::Stream::start(trees, left_out, listener).map(Watch::Stream),
                (Reach::Entries, [folder]) => {
                    super::fsevents::Stream::start(std::slice::from_ref(folder), &[], listener).map(Watch::Stream)
                }
                (Reach::Node, [path]) => super::kqueue::Node::start(path.clone(), listener).map(Watch::Node),
                _ => Err("a watch of one path covers one path".into()),
            }
        }
    }
}

/// FSEvents streams, each run by a thread of its own.
#[cfg(target_os = "macos")]
mod fsevents {
    use super::{Listener, Raw, from_flags};
    use fsevent_sys as fs;
    use fsevent_sys::core_foundation as cf;
    use std::ffi::{CStr, OsStr, c_char, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::ptr;
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;

    unsafe extern "C" {
        /// Whether the run loop sits waiting for its next event.
        fn CFRunLoopIsWaiting(run_loop: cf::CFRunLoopRef) -> cf::Boolean;
        /// The folders a stream leaves out, eight at most; fsevent-sys does
        /// not declare it.
        fn FSEventStreamSetExclusionPaths(
            stream: fs::FSEventStreamRef,
            paths: cf::CFMutableArrayRef,
        ) -> cf::Boolean;
    }

    /// A run loop, handed from its own thread to the one that stops it.
    struct RunLoop(cf::CFRunLoopRef);

    // SAFETY: CFRunLoopIsWaiting and CFRunLoopStop may be called from any
    // thread; nothing else is done with the run loop outside its own.
    unsafe impl Send for RunLoop {}

    /// A stream over the trees, run by a thread of its own until dropped.
    pub(crate) struct Stream {
        run_loop: RunLoop,
        thread: Option<thread::JoinHandle<()>>,
        /// The stream's thread holds it too, each callback for its events.
        #[cfg_attr(not(feature = "testing"), allow(dead_code, reason = "only a test speaks for FSEvents"))]
        listener: Arc<Mutex<Listener>>,
    }

    impl Stream {
        pub(super) fn start(trees: &[PathBuf], left_out: &[PathBuf], listener: Listener) -> Result<Stream, String> {
            let (sender, receiver) = mpsc::channel();
            let trees = trees.to_vec();
            let left_out = left_out.to_vec();
            let listener = Arc::new(Mutex::new(listener));
            let thread = {
                let listener = listener.clone();
                thread::Builder::new()
                    .name("fsevents".into())
                    .spawn(move || run(&trees, &left_out, listener, &sender))
                    .map_err(|error| error.to_string())?
            };
            match receiver.recv() {
                Ok(Some(run_loop)) => Ok(Stream {
                    run_loop,
                    thread: Some(thread),
                    listener,
                }),
                _ => {
                    // The thread has ended or is about to, having sent that
                    // its stream did not start.
                    let _ = thread.join();
                    Err("FSEvents could not start a stream for the path".into())
                }
            }
        }
    }

    impl Drop for Stream {
        fn drop(&mut self) {
            // SAFETY: the run loop lives until its thread ends, which is
            // after this stop. A stop before the loop runs would be lost, so
            // it waits until the loop waits.
            unsafe {
                while CFRunLoopIsWaiting(self.run_loop.0) == 0 {
                    thread::yield_now();
                }
                cf::CFRunLoopStop(self.run_loop.0);
            }
            if let Some(thread) = self.thread.take() {
                // A thread that panicked has nothing left to release.
                let _ = thread.join();
            }
        }
    }

    /// Runs a stream over `trees` on this thread until its run loop is
    /// stopped, after sending the run loop; sends `None` instead when the
    /// stream cannot start.
    fn run(
        trees: &[PathBuf],
        left_out: &[PathBuf],
        listener: Arc<Mutex<Listener>>,
        started: &mpsc::Sender<Option<RunLoop>>,
    ) {
        // `start` waits for the one message this sends: sending cannot fail.
        let Some(stream) = create_stream(trees, left_out, listener) else {
            let _ = started.send(None);
            return;
        };
        // SAFETY: the stream is this thread's own, scheduled on this
        // thread's run loop, and released once, after its loop has stopped.
        unsafe {
            let run_loop = cf::CFRunLoopGetCurrent();
            fs::FSEventStreamScheduleWithRunLoop(stream, run_loop, cf::kCFRunLoopDefaultMode);
            if fs::FSEventStreamStart(stream) != 0 {
                let _ = started.send(Some(RunLoop(run_loop)));
                cf::CFRunLoopRun();
                fs::FSEventStreamStop(stream);
            } else {
                let _ = started.send(None);
            }
            fs::FSEventStreamInvalidate(stream);
            fs::FSEventStreamRelease(stream);
        }
    }

    /// The paths as a Core Foundation array the caller releases; `None`
    /// when one of them cannot be one.
    ///
    /// # Safety
    ///
    /// Core Foundation is called as its documentation says.
    unsafe fn paths_array(paths: &[PathBuf]) -> Option<cf::CFMutableArrayRef> {
        // SAFETY: every object made here is released here, but the array.
        unsafe {
            let array = cf::CFArrayCreateMutable(cf::kCFAllocatorDefault, 0, &cf::kCFTypeArrayCallBacks);
            for tree in paths {
                let path = tree.to_str().map(|tree| {
                    let mut error: cf::CFErrorRef = ptr::null_mut();
                    let path = cf::str_path_to_cfstring_ref(tree, &mut error);
                    if !error.is_null() {
                        cf::CFRelease(error as cf::CFRef);
                    }
                    path
                });
                match path {
                    Some(path) if !path.is_null() => {
                        cf::CFArrayAppendValue(array, path);
                        cf::CFRelease(path);
                    }
                    _ => {
                        cf::CFRelease(array);
                        return None;
                    }
                }
            }
            Some(array)
        }
    }

    /// A stream over `trees`, without what lies in `left_out`, handing the
    /// listener what changes under them; `None` when FSEvents cannot be
    /// given a tree.
    fn create_stream(
        trees: &[PathBuf],
        left_out: &[PathBuf],
        listener: Arc<Mutex<Listener>>,
    ) -> Option<fs::FSEventStreamRef> {
        // SAFETY: every Core Foundation object made here is released here,
        // except the listener, which the stream releases with itself.
        unsafe {
            let paths = paths_array(trees)?;
            let info = Arc::into_raw(listener).cast_mut().cast::<c_void>();
            let context = fs::FSEventStreamContext {
                version: 0,
                info,
                retain: None,
                release: Some(release_listener),
                copy_description: None,
            };
            let stream = fs::FSEventStreamCreate(
                cf::kCFAllocatorDefault,
                callback,
                &context,
                paths,
                fs::kFSEventStreamEventIdSinceNow,
                0.0,
                fs::kFSEventStreamCreateFlagFileEvents | fs::kFSEventStreamCreateFlagNoDefer,
            );
            cf::CFRelease(paths);
            if stream.is_null() {
                // No stream took the listener.
                drop(Arc::from_raw(info.cast::<Mutex<Listener>>()));
                return None;
            }
            if !left_out.is_empty()
                && let Some(excluded) = paths_array(left_out)
            {
                // A stream that keeps them is only busier: nothing depends
                // on its leaving them out but its cost.
                if FSEventStreamSetExclusionPaths(stream, excluded) == 0 {
                    tracing::warn!("FSEvents did not leave out the folders git ignores");
                }
                cf::CFRelease(excluded);
            }
            Some(stream)
        }
    }

    extern "C" fn callback(
        _stream: fs::FSEventStreamRef,
        info: *mut c_void,
        count: usize,
        paths: *mut c_void,
        flags: *const fs::FSEventStreamEventFlags,
        _ids: *const fs::FSEventStreamEventId,
    ) {
        // SAFETY: `info` is the listener the stream was made with, which
        // lives until the stream releases it; FSEvents passes `count` C
        // paths and flags.
        let listener = unsafe { &*info.cast::<Mutex<Listener>>() };
        let paths = paths.cast::<*const c_char>();
        let raws: Vec<Raw> = (0..count)
            .map(|index| {
                let (path, flags) = unsafe { (CStr::from_ptr(*paths.add(index)), *flags.add(index)) };
                from_flags(PathBuf::from(OsStr::from_bytes(path.to_bytes())), flags)
            })
            .collect();
        lock(listener).deliver(raws);
    }

    extern "C" fn release_listener(info: *const c_void) {
        // SAFETY: the stream releases the listener it was made with once, as
        // it is released itself.
        drop(unsafe { Arc::from_raw(info.cast::<Mutex<Listener>>()) });
    }

    /// A poisoned lock means a panic mid-delivery, which nothing may paper
    /// over.
    fn lock(listener: &Mutex<Listener>) -> std::sync::MutexGuard<'_, Listener> {
        listener.lock().expect("the stream's listener is intact")
    }

    #[cfg(feature = "testing")]
    impl Stream {
        /// What FSEvents would say of `path` with `flags`, through the way
        /// the stream's own events take.
        pub(crate) fn said(&self, path: PathBuf, flags: fs::FSEventStreamEventFlags) {
            lock(&self.listener).deliver(vec![from_flags(path, flags)]);
        }
    }
}

/// A kqueue of one file or folder, run by a thread of its own.
#[cfg(target_os = "macos")]
mod kqueue {
    use super::{Listener, Raw};
    use rustix::event::kqueue::{self as kq, Event, EventFilter, EventFlags, UserDefinedFlags, UserFlags, VnodeEvents};
    use rustix::fd::{AsRawFd, OwnedFd};
    use std::path::{Path, PathBuf};
    use std::thread;

    /// The user event that stops a node's thread.
    const STOP: isize = 1;

    /// What a node's descriptor is watched for.
    fn vnode_events() -> VnodeEvents {
        VnodeEvents::DELETE
            | VnodeEvents::WRITE
            | VnodeEvents::EXTEND
            | VnodeEvents::ATTRIBUTES
            | VnodeEvents::RENAME
            | VnodeEvents::REVOKE
            | VnodeEvents::LINK
    }

    pub(crate) struct Node {
        queue: std::sync::Arc<OwnedFd>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl Node {
        pub(super) fn start(path: PathBuf, listener: Listener) -> Result<Node, String> {
            let queue = std::sync::Arc::new(kq::kqueue().map_err(|error| error.to_string())?);
            let stop = Event::new(
                EventFilter::User {
                    ident: STOP,
                    flags: UserFlags::empty(),
                    user_flags: UserDefinedFlags::new(0),
                },
                EventFlags::ADD | EventFlags::CLEAR,
                std::ptr::null_mut(),
            );
            // SAFETY: the change names no descriptor.
            unsafe { kq::kevent(&*queue, &[stop], &mut Vec::<Event>::new(), None) }.map_err(|error| error.to_string())?;
            let mut watched = Watched::arm(&queue, &path)?;
            let thread = {
                let queue = queue.clone();
                thread::Builder::new()
                    .name("kqueue".into())
                    .spawn(move || {
                        let mut listener = listener;
                        run(&queue, &path, &mut watched, &mut listener);
                    })
                    .map_err(|error| error.to_string())?
            };
            Ok(Node {
                queue,
                thread: Some(thread),
            })
        }
    }

    impl Drop for Node {
        fn drop(&mut self) {
            let stop = Event::new(
                EventFilter::User {
                    ident: STOP,
                    flags: UserFlags::TRIGGER,
                    user_flags: UserDefinedFlags::new(0),
                },
                EventFlags::empty(),
                std::ptr::null_mut(),
            );
            // SAFETY: the change names no descriptor. A queue that refuses it
            // has a thread that ended already.
            let _ = unsafe { kq::kevent(&*self.queue, &[stop], &mut Vec::<Event>::new(), None) };
            if let Some(thread) = self.thread.take() {
                // A thread that panicked has nothing left to release.
                let _ = thread.join();
            }
        }
    }

    /// What a node's queue watches now: the node itself, or, while it is
    /// missing, the nearest folder above it that is there, so its coming is
    /// seen.
    struct Watched {
        /// Held for its registration, which goes when it closes.
        _descriptor: OwnedFd,
        present: bool,
    }

    impl Watched {
        fn arm(queue: &OwnedFd, path: &Path) -> Result<Watched, String> {
            let (descriptor, present) = match open(path) {
                Ok(descriptor) => (descriptor, true),
                Err(_) => {
                    let above = path
                        .ancestors()
                        .skip(1)
                        .find_map(|folder| open(folder).ok())
                        .ok_or_else(|| "no folder above the path can be watched".to_owned())?;
                    (above, false)
                }
            };
            let change = Event::new(
                EventFilter::Vnode {
                    vnode: descriptor.as_raw_fd(),
                    flags: vnode_events(),
                },
                EventFlags::ADD | EventFlags::CLEAR,
                std::ptr::null_mut(),
            );
            // SAFETY: the descriptor lives as long as its registration: the
            // registration goes when the descriptor closes.
            unsafe { kq::kevent(queue, &[change], &mut Vec::<Event>::new(), None) }.map_err(|error| error.to_string())?;
            Ok(Watched {
                _descriptor: descriptor,
                present,
            })
        }
    }

    /// Opens `path` for its events alone; rustix names no `O_EVTONLY`.
    fn open(path: &Path) -> rustix::io::Result<OwnedFd> {
        let events_only = rustix::fs::OFlags::from_bits_retain(libc::O_EVTONLY as u32);
        rustix::fs::open(path, events_only | rustix::fs::OFlags::CLOEXEC, rustix::fs::Mode::empty())
    }

    fn run(queue: &OwnedFd, path: &Path, watched: &mut Watched, listener: &mut Listener) {
        let mut buffer = [std::mem::MaybeUninit::<Event>::uninit(); 8];
        loop {
            // SAFETY: every descriptor registered lives while it is.
            let Ok((events, _)) = (unsafe { kq::kevent(queue, &[], &mut buffer, None) }) else {
                // Interrupted: wait again.
                continue;
            };
            let mut raws = Vec::new();
            for event in events.iter() {
                match event.filter() {
                    EventFilter::User { .. } => return,
                    EventFilter::Vnode { flags, .. } => {
                        let gone = flags.intersects(VnodeEvents::DELETE | VnodeEvents::RENAME | VnodeEvents::REVOKE);
                        if watched.present {
                            let metadata = !flags.intersects(VnodeEvents::WRITE | VnodeEvents::EXTEND) && !gone;
                            // A folder written is one whose names changed.
                            let entry = gone || (flags.contains(VnodeEvents::WRITE) && path.is_dir());
                            raws.push(Raw::Changed {
                                path: path.to_path_buf(),
                                metadata,
                                entry,
                                folder: None,
                            });
                        }
                        if gone || !watched.present {
                            // Replaced, gone, or something moved above a
                            // missing node: watch it again under its name.
                            let was_present = watched.present;
                            match Watched::arm(queue, path) {
                                Ok(armed) => {
                                    if armed.present && !was_present {
                                        raws.push(Raw::Changed {
                                            path: path.to_path_buf(),
                                            metadata: false,
                                            entry: true,
                                            folder: None,
                                        });
                                    }
                                    *watched = armed;
                                }
                                Err(reason) => {
                                    raws.push(Raw::Lost { under: None });
                                    tracing::warn!("a watch of {} could not go on: {reason}", path.display());
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            if !raws.is_empty() {
                listener.deliver(raws);
            }
        }
    }
}

#[cfg(all(feature = "testing", target_os = "macos"))]
impl TreeWatch {
    /// What FSEvents would say of `path` with `flags`, a stream's lost
    /// events among them, which nothing else can make it say; false for a
    /// watch that is no stream.
    pub(crate) fn said(&self, path: PathBuf, flags: u32) -> bool {
        match &self._watch {
            platform::Watch::Stream(stream) => {
                stream.said(path, flags);
                true
            }
            platform::Watch::Node(_) => false,
        }
    }
}
