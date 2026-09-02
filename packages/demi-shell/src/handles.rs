//! The handle table: every open resource is an integer with an explicit
//! `close`, so a leak is countable and nothing depends on GC finalisers.
//!
//! Two kinds of resource share the `read`/`write`/`close` calls of
//! `demishell:fs`:
//!
//! - `File`: a regular file or one of the standard streams. Operations run on
//!   the blocking pool through a cloned descriptor, so concurrent operations
//!   on one file are serialised by the kernel and need no bookkeeping.
//! - `Stream`: a pipe or socket driven by tokio. One read and one write may
//!   be in flight at a time; a second one of the same direction is `EBUSY`.
//!   `close` while an operation is pending cancels it with `ECANCELED`.

use std::collections::HashMap;
use std::os::fd::FromRawFd;
use std::rc::Rc;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Notify;

pub type Reader = Box<dyn AsyncRead + Unpin>;
pub type Writer = Box<dyn AsyncWrite + Unpin>;

pub enum Slot<T> {
    Absent,
    Busy,
    Ready(T),
}

pub struct Stream {
    pub reader: Slot<Reader>,
    pub writer: Slot<Writer>,
    /// Signalled by `close`; a pending operation wakes and fails.
    pub cancel: Rc<Notify>,
}

impl Stream {
    pub fn new(reader: Option<Reader>, writer: Option<Writer>) -> Self {
        Stream {
            reader: reader.map_or(Slot::Absent, Slot::Ready),
            writer: writer.map_or(Slot::Absent, Slot::Ready),
            cancel: Rc::new(Notify::new()),
        }
    }
}

pub enum Resource {
    File(Arc<std::fs::File>),
    Stream(Stream),
    /// A UDS listener; the notify cancels a pending `accept` on close.
    Listener(Rc<tokio::net::UnixListener>, Rc<Notify>),
    Ws(crate::net::WsResource),
}

pub struct Handles {
    next: i32,
    map: HashMap<i32, Resource>,
}

impl Handles {
    /// Starts with the standard streams at 0, 1 and 2.
    pub fn new() -> Self {
        let mut map = HashMap::new();
        for fd in 0..3 {
            // SAFETY: the standard descriptors are open for the life of the
            // process; the table owns them from here on.
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            map.insert(fd, Resource::File(Arc::new(file)));
        }
        Handles { next: 3, map }
    }

    pub fn insert(&mut self, resource: Resource) -> i32 {
        let id = self.next;
        self.next += 1;
        self.map.insert(id, resource);
        id
    }

    pub fn get_mut(&mut self, id: i32) -> Option<&mut Resource> {
        self.map.get_mut(&id)
    }

    pub fn remove(&mut self, id: i32) -> Option<Resource> {
        self.map.remove(&id)
    }

    /// Open handles above the standard streams, for leak checks in tests.
    pub fn open_count(&self) -> usize {
        self.map.keys().filter(|id| **id >= 3).count()
    }
}
