//! An object store that counts what reaches it, for the scenarios that prove
//! what the backend reads and writes (`storage.md` § The object store,
//! § Retention): each put with its bytes, each read, each HEAD, the most
//! reads in flight at once, each listing and each deletion.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;
use object_store::path::Path;
use object_store::{
    CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore, PutMultipartOptions,
    PutOptions, PutPayload, PutResult, RenameOptions, Result,
};

/// The counts of one object store, which clones share.
#[derive(Debug, Clone, Default)]
pub struct ObjectCounts(Arc<Counters>);

#[derive(Debug, Default)]
struct Counters {
    puts: AtomicU64,
    bytes_put: AtomicU64,
    gets: AtomicU64,
    heads: AtomicU64,
    reading: AtomicU64,
    most_reading: AtomicU64,
    lists: AtomicU64,
    deletes: AtomicU64,
}

/// What reached the object store up to one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ObjectTally {
    pub puts: u64,
    /// The bytes the puts sent.
    pub bytes_put: u64,
    /// Reads of an object's bytes.
    pub gets: u64,
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
            heads: counters.heads.load(Ordering::SeqCst),
            most_gets_at_once: counters.most_reading.load(Ordering::SeqCst),
            lists: counters.lists.load(Ordering::SeqCst),
            deletes: counters.deletes.load(Ordering::SeqCst),
        }
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
    async fn put_opts(&self, location: &Path, payload: PutPayload, opts: PutOptions) -> Result<PutResult> {
        let counters = &self.counts.0;
        counters.puts.fetch_add(1, Ordering::SeqCst);
        let bytes = u64::try_from(payload.content_length()).expect("a payload's length fits in 64 bits");
        counters.bytes_put.fetch_add(bytes, Ordering::SeqCst);
        self.inner.put_opts(location, payload, opts).await
    }

    async fn put_multipart_opts(&self, location: &Path, opts: PutMultipartOptions) -> Result<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(location, opts).await
    }

    async fn get_opts(&self, location: &Path, options: GetOptions) -> Result<GetResult> {
        // `ObjectStoreExt::head` is a get that asks for no content.
        if options.head {
            self.counts.0.heads.fetch_add(1, Ordering::SeqCst);
            return self.inner.get_opts(location, options).await;
        }
        let _reading = self.counts.read();
        // A read takes a moment, as it does over a network. A local file
        // can answer on its blocking thread before the caller starts its
        // next read, and a loaded machine makes that likely, so without
        // this yield reads that start together would count as one at a
        // time.
        tokio::task::yield_now().await;
        self.inner.get_opts(location, options).await
    }

    fn delete_stream(&self, locations: BoxStream<'static, Result<Path>>) -> BoxStream<'static, Result<Path>> {
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
