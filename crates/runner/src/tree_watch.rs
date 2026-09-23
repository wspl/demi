//! A watch of directory trees that reports each path something changed at
//! (`runner.md` § Working tree); reading a file changes nothing. On macOS it
//! is an FSEvents stream, one per watch whatever the size of the trees:
//! notify's recommended backend there is kqueue, which the vendored `tail`
//! selects and which holds a descriptor for every file of a tree, more than
//! a repository and the usual limit of 256 allow; with kqueue selected,
//! notify builds no FSEvents watcher. `tail` needs kqueue: FSEvents reports
//! an append through an open descriptor only once the writer closes the
//! file, and names a file by its real path, so `tail -f` on FSEvents prints
//! nothing while a program keeps its log open, or for a file under `/tmp`.
//! Elsewhere it is notify's recommended backend.

use std::path::{Path, PathBuf};

/// What a watch saw.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WatchEvent {
    /// Something changed at `path`; `metadata` when only its metadata did,
    /// such as its mode or its times.
    Changed { path: PathBuf, metadata: bool },
    /// The watch lost events: what it reported no longer tells what changed.
    Lost,
    /// The watch failed and reports nothing more.
    #[cfg_attr(
        target_os = "macos",
        allow(dead_code, reason = "FSEvents reports no failure once its stream runs")
    )]
    Failed,
}

type Report = Box<dyn FnMut(WatchEvent) + Send>;

/// A running watch of some directory trees; it stops when dropped.
pub(crate) struct TreeWatch {
    _watch: platform::Watch,
}

impl TreeWatch {
    /// Watches each of `trees` and what is under it, handing `report` what
    /// changes; `None` when the watch cannot be set up.
    pub(crate) fn start(
        trees: &[&Path],
        report: impl FnMut(WatchEvent) + Send + 'static,
    ) -> Option<TreeWatch> {
        platform::Watch::start(trees, Box::new(report)).map(|watch| TreeWatch { _watch: watch })
    }
}

/// What a notify event reports: nothing for a file opened, read or closed.
#[cfg(any(test, not(target_os = "macos")))]
fn from_notify(event: notify::Event) -> Vec<WatchEvent> {
    use notify::{EventKind, event::ModifyKind};
    if event.need_rescan() {
        return vec![WatchEvent::Lost];
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return Vec::new();
    }
    let metadata = matches!(event.kind, EventKind::Modify(ModifyKind::Metadata(_)));
    event
        .paths
        .into_iter()
        .map(|path| WatchEvent::Changed { path, metadata })
        .collect()
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{Report, WatchEvent, from_notify};
    use notify::Watcher as _;
    use std::path::Path;

    pub(super) struct Watch {
        _watcher: notify::RecommendedWatcher,
    }

    impl Watch {
        pub(super) fn start(trees: &[&Path], mut report: Report) -> Option<Watch> {
            let mut watcher = notify::recommended_watcher(
                move |event: notify::Result<notify::Event>| match event {
                    Ok(event) => from_notify(event).into_iter().for_each(&mut report),
                    Err(_) => report(WatchEvent::Failed),
                },
            )
            .ok()?;
            for tree in trees {
                watcher.watch(tree, notify::RecursiveMode::Recursive).ok()?;
            }
            Some(Watch { _watcher: watcher })
        }
    }
}

/// What FSEvents' flags for one path report.
#[cfg(target_os = "macos")]
fn from_flags(path: PathBuf, flags: fsevent_sys::FSEventStreamEventFlags) -> WatchEvent {
    use fsevent_sys as fs;
    const LOST: fs::FSEventStreamEventFlags = fs::kFSEventStreamEventFlagMustScanSubDirs
        | fs::kFSEventStreamEventFlagUserDropped
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
    if flags & LOST != 0 {
        return WatchEvent::Lost;
    }
    let metadata = flags & METADATA != 0 && flags & CONTENT == 0;
    WatchEvent::Changed { path, metadata }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{Report, from_flags};
    use fsevent_sys as fs;
    use fsevent_sys::core_foundation as cf;
    use std::ffi::{CStr, OsStr, c_char, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr;
    use std::sync::mpsc;
    use std::thread;

    unsafe extern "C" {
        /// Whether the run loop sits waiting for its next event.
        fn CFRunLoopIsWaiting(run_loop: cf::CFRunLoopRef) -> cf::Boolean;
    }

    /// A run loop, handed from its own thread to the one that stops it.
    struct RunLoop(cf::CFRunLoopRef);

    // SAFETY: CFRunLoopIsWaiting and CFRunLoopStop may be called from any
    // thread; nothing else is done with the run loop outside its own.
    unsafe impl Send for RunLoop {}

    /// A stream over the trees, run by a thread of its own until dropped.
    pub(super) struct Watch {
        run_loop: RunLoop,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl Watch {
        pub(super) fn start(trees: &[&Path], report: Report) -> Option<Watch> {
            let trees: Vec<PathBuf> = trees.iter().map(|tree| tree.to_path_buf()).collect();
            let (sender, receiver) = mpsc::channel();
            let thread = thread::Builder::new()
                .name("fsevents".into())
                .spawn(move || run(&trees, report, &sender))
                .ok()?;
            match receiver.recv() {
                Ok(Some(run_loop)) => Some(Watch {
                    run_loop,
                    thread: Some(thread),
                }),
                _ => {
                    // The thread has ended or is about to, having sent that
                    // its stream did not start.
                    let _ = thread.join();
                    None
                }
            }
        }
    }

    impl Drop for Watch {
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
    fn run(trees: &[PathBuf], report: Report, started: &mpsc::Sender<Option<RunLoop>>) {
        // `start` waits for the one message this sends: sending cannot fail.
        let Some(stream) = create_stream(trees, report) else {
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

    /// A stream over `trees` handing `report` what changes under them;
    /// `None` when FSEvents cannot be given a tree.
    fn create_stream(trees: &[PathBuf], report: Report) -> Option<fs::FSEventStreamRef> {
        // SAFETY: every Core Foundation object made here is released here,
        // except the report, which the stream releases with itself.
        unsafe {
            let paths =
                cf::CFArrayCreateMutable(cf::kCFAllocatorDefault, 0, &cf::kCFTypeArrayCallBacks);
            for tree in trees {
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
                        cf::CFArrayAppendValue(paths, path);
                        cf::CFRelease(path);
                    }
                    _ => {
                        cf::CFRelease(paths);
                        return None;
                    }
                }
            }
            let info = Box::into_raw(Box::new(report)).cast::<c_void>();
            let context = fs::FSEventStreamContext {
                version: 0,
                info,
                retain: None,
                release: Some(release_report),
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
                // No stream took the report.
                drop(Box::from_raw(info.cast::<Report>()));
                return None;
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
        // SAFETY: `info` is the report the stream was made with, used only on
        // the stream's thread; FSEvents passes `count` C paths and flags.
        let report = unsafe { &mut *info.cast::<Report>() };
        let paths = paths.cast::<*const c_char>();
        for index in 0..count {
            let (path, flags) = unsafe { (CStr::from_ptr(*paths.add(index)), *flags.add(index)) };
            let path = PathBuf::from(OsStr::from_bytes(path.to_bytes()));
            report(from_flags(path, flags));
        }
    }

    extern "C" fn release_report(info: *const c_void) {
        // SAFETY: the stream releases the report it was made with once, as
        // it is released itself.
        drop(unsafe { Box::from_raw(info.cast_mut().cast::<Report>()) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notify_event_reports_changes_and_not_reads() {
        use notify::event::{AccessKind, AccessMode, DataChange, Flag, MetadataKind, ModifyKind};
        use notify::{Event, EventKind};
        let at = |kind: EventKind| Event::new(kind).add_path(PathBuf::from("/repo/a.txt"));
        let changed = |metadata| {
            vec![WatchEvent::Changed {
                path: PathBuf::from("/repo/a.txt"),
                metadata,
            }]
        };
        assert_eq!(
            from_notify(at(EventKind::Access(AccessKind::Open(AccessMode::Any)))),
            []
        );
        assert_eq!(
            from_notify(at(EventKind::Access(AccessKind::Close(AccessMode::Write)))),
            []
        );
        assert_eq!(
            from_notify(at(EventKind::Modify(ModifyKind::Data(DataChange::Content)))),
            changed(false)
        );
        assert_eq!(
            from_notify(at(EventKind::Modify(ModifyKind::Metadata(
                MetadataKind::Any
            )))),
            changed(true)
        );
        assert_eq!(
            from_notify(Event::new(EventKind::Any).set_flag(Flag::Rescan)),
            [WatchEvent::Lost]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn fsevents_flags_report_content_metadata_or_loss() {
        use fsevent_sys as fs;
        let at = |flags| from_flags(PathBuf::from("/repo/a.txt"), flags);
        let changed = |metadata| WatchEvent::Changed {
            path: PathBuf::from("/repo/a.txt"),
            metadata,
        };
        let file = fs::kFSEventStreamEventFlagItemIsFile;
        assert_eq!(
            at(file | fs::kFSEventStreamEventFlagItemModified),
            changed(false)
        );
        assert_eq!(
            at(file | fs::kFSEventStreamEventFlagItemInodeMetaMod),
            changed(true)
        );
        // Flags gather what happened to a path lately: a created file whose
        // mode then changed is more than its metadata.
        assert_eq!(
            at(file
                | fs::kFSEventStreamEventFlagItemCreated
                | fs::kFSEventStreamEventFlagItemInodeMetaMod),
            changed(false)
        );
        assert_eq!(
            at(fs::kFSEventStreamEventFlagMustScanSubDirs),
            WatchEvent::Lost
        );
    }
}
