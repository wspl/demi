//! An object store that counts what reaches it, for the scenarios that prove
//! what the backend reads and writes: each put with its bytes, each read,
//! each HEAD, the most reads in flight at once, each listing and each
//! deletion; and that holds its reads while a scenario says so, to stop a
//! call while it waits for one.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use object_store::path::Path;
use object_store::{
    CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
    PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions, Result,
};

/// The counts of one object store, which clones share.
#[derive(Debug, Clone, Default)]
pub struct ObjectCounts(Arc<Counters>);

#[derive(Debug, Default)]
struct Counters {
    puts: AtomicU64,
    bytes_put: AtomicU64,
    gets: AtomicU64,
    bytes_got: AtomicU64,
    heads: AtomicU64,
    reading: AtomicU64,
    most_reading: AtomicU64,
    lists: AtomicU64,
    deletes: AtomicU64,
    /// The hold the reads wait at, while one is held.
    hold: std::sync::Mutex<Option<Arc<HoldState>>>,
}

/// A hold's state: how many reads wait at it, and whether it let them go.
#[derive(Debug)]
struct HoldState {
    waiting: tokio::sync::watch::Sender<usize>,
    released: tokio::sync::watch::Sender<bool>,
}

/// A hold on the reads of an object's content: from [`ObjectCounts::hold_gets`]
/// until it is released or dropped, each such read waits before it reaches
/// the store.
pub struct GetHold {
    counts: ObjectCounts,
    state: Arc<HoldState>,
}

impl GetHold {
    /// Waits until `count` reads wait at the hold.
    pub async fn until_waiting(&self, count: usize) {
        let mut waiting = self.state.waiting.subscribe();
        // The hold keeps its sender, so the channel stays open.
        let _ = waiting.wait_for(|waiting| *waiting >= count).await;
    }

    /// Lets the waiting reads and every later one through.
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for GetHold {
    fn drop(&mut self) {
        let mut hold = self.counts.0.hold.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *hold = None;
        self.state.released.send_replace(true);
    }
}

/// What reached the object store up to one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ObjectTally {
    pub puts: u64,
    /// The bytes the puts sent.
    pub bytes_put: u64,
    /// Reads of an object's bytes.
    pub gets: u64,
    /// The bytes the reads' answers carry.
    pub bytes_got: u64,
    /// Asks whether an object exists, which transfer no bytes.
    pub heads: u64,
    /// The most reads that were in flight at once.
    pub most_gets_at_once: u64,
    /// Listings of a prefix.
    pub lists: u64,
    /// Objects asked to be deleted.
    pub deletes: u64,
}

impl ObjectTally {
    /// What reached the object store since `earlier`; the most reads at
    /// once is this tally's.
    pub fn since(&self, earlier: &Self) -> Self {
        Self {
            puts: self.puts - earlier.puts,
            bytes_put: self.bytes_put - earlier.bytes_put,
            gets: self.gets - earlier.gets,
            bytes_got: self.bytes_got - earlier.bytes_got,
            heads: self.heads - earlier.heads,
            most_gets_at_once: self.most_gets_at_once,
            lists: self.lists - earlier.lists,
            deletes: self.deletes - earlier.deletes,
        }
    }
}

impl ObjectCounts {
    pub fn tally(&self) -> ObjectTally {
        let counters = &self.0;
        ObjectTally {
            puts: counters.puts.load(Ordering::SeqCst),
            bytes_put: counters.bytes_put.load(Ordering::SeqCst),
            gets: counters.gets.load(Ordering::SeqCst),
            bytes_got: counters.bytes_got.load(Ordering::SeqCst),
            heads: counters.heads.load(Ordering::SeqCst),
            most_gets_at_once: counters.most_reading.load(Ordering::SeqCst),
            lists: counters.lists.load(Ordering::SeqCst),
            deletes: counters.deletes.load(Ordering::SeqCst),
        }
    }

    /// Holds every read of an object's content from now on, until the hold
    /// is released.
    pub fn hold_gets(&self) -> GetHold {
        let state = Arc::new(HoldState {
            waiting: tokio::sync::watch::Sender::new(0),
            released: tokio::sync::watch::Sender::new(false),
        });
        *self.0.hold.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(state.clone());
        GetHold { counts: self.clone(), state }
    }

    /// `objects`, with what reaches it counted here.
    pub fn observe(&self, objects: Arc<dyn ObjectStore>) -> Arc<dyn ObjectStore> {
        Arc::new(Counted {
            inner: objects,
            counts: self.clone(),
        })
    }

    /// Counts a read that is in flight until the answer is dropped.
    fn read(&self) -> Reading<'_> {
        let counters = &self.0;
        counters.gets.fetch_add(1, Ordering::SeqCst);
        let now = counters.reading.fetch_add(1, Ordering::SeqCst) + 1;
        counters.most_reading.fetch_max(now, Ordering::SeqCst);
        Reading(counters)
    }
}

/// A read in flight, which ends when dropped: answered, failed or
/// abandoned.
struct Reading<'a>(&'a Counters);

impl Drop for Reading<'_> {
    fn drop(&mut self) {
        self.0.reading.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct Counted {
    inner: Arc<dyn ObjectStore>,
    counts: ObjectCounts,
}

impl fmt::Display for Counted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Counted({})", self.inner)
    }
}

#[async_trait::async_trait]
impl ObjectStore for Counted {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> Result<PutResult> {
        let counters = &self.counts.0;
        counters.puts.fetch_add(1, Ordering::SeqCst);
        let bytes =
            u64::try_from(payload.content_length()).expect("a payload's length fits in 64 bits");
        counters.bytes_put.fetch_add(bytes, Ordering::SeqCst);
        self.inner.put_opts(location, payload, opts).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> Result<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(location, opts).await
    }

    async fn get_opts(&self, location: &Path, options: GetOptions) -> Result<GetResult> {
        // `ObjectStoreExt::head` is a get that asks for no content.
        if options.head {
            self.counts.0.heads.fetch_add(1, Ordering::SeqCst);
            return self.inner.get_opts(location, options).await;
        }
        let held = self.counts.0.hold.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        if let Some(held) = held {
            let mut released = held.released.subscribe();
            held.waiting.send_modify(|waiting| *waiting += 1);
            // The state keeps its sender, so the channel stays open.
            let _ = released.wait_for(|released| *released).await;
        }
        let _reading = self.counts.read();
        // A read takes a moment, as it does over a network. A local file
        // can answer on its blocking thread before the caller starts its
        // next read, and a loaded machine makes that likely, so without
        // this yield reads that start together would count as one at a
        // time.
        tokio::task::yield_now().await;
        let answer = self.inner.get_opts(location, options).await?;
        let carried = answer.range.end - answer.range.start;
        self.counts.0.bytes_got.fetch_add(carried, Ordering::SeqCst);
        Ok(answer)
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, Result<Path>>,
    ) -> BoxStream<'static, Result<Path>> {
        let counts = self.counts.clone();
        let counted = locations.inspect(move |_| {
            counts.0.deletes.fetch_add(1, Ordering::SeqCst);
        });
        self.inner.delete_stream(counted.boxed())
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, Result<ObjectMeta>> {
        self.counts.0.lists.fetch_add(1, Ordering::SeqCst);
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> Result<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(&self, from: &Path, to: &Path, options: CopyOptions) -> Result<()> {
        self.inner.copy_opts(from, to, options).await
    }

    async fn rename_opts(&self, from: &Path, to: &Path, options: RenameOptions) -> Result<()> {
        self.inner.rename_opts(from, to, options).await
    }
}
