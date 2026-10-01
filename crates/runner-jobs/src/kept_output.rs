//! A job's kept output (`runner.md` § Pipes and output): every read of its
//! stdout and stderr, in the order the runner read them, as records of the
//! wire's kept output, within `JOB_KEPT_BYTES` of records. The first
//! `JOB_KEPT_PART_BYTES` stay whole in `head`. The rest goes to segments, of
//! which the newest hold at most `JOB_KEPT_PART_BYTES`: the oldest segment
//! goes as new output comes, and the output it held is left out. `job_read`
//! streams a snapshot of it, while the job runs and after.

use std::{
    collections::VecDeque,
    io,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use bytes::Bytes;
use futures_util::{Stream, StreamExt, stream::BoxStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use demi_runner_protocol::wire::{self, JOB_KEPT_PART_BYTES, KeptRecord, OutputStream, WireBytes};

/// How many bytes of records one segment of the kept output's end holds
/// before the next one begins: the granularity at which the end lets old
/// output go.
const SEGMENT_BYTES: u64 = 1024 * 1024;

/// The size of the chunks a snapshot is read in.
const CHUNK_BYTES: usize = 64 * 1024;

/// The job's writer of its kept output.
pub struct KeptOutput {
    directory: PathBuf,
    /// The head, until a read did not fit in it; the end follows.
    head: Option<tokio::fs::File>,
    /// The segment being written, once output went past the head.
    segment: Option<tokio::fs::File>,
    layout: Arc<Mutex<Layout>>,
}

/// What of the kept output is written, which snapshots read.
struct Layout {
    head: PathBuf,
    /// The length of `head`, in bytes of records.
    head_length: u64,
    /// The segments of the end, oldest first.
    segments: VecDeque<Segment>,
    /// The length of the segments, in bytes of records.
    end_length: u64,
    /// The output left out between the head and the segments.
    left_out: u64,
    next_segment: u64,
}

struct Segment {
    path: PathBuf,
    /// Its length, in bytes of records.
    length: u64,
    /// The output it holds.
    output: u64,
}

impl KeptOutput {
    /// Starts an empty kept output in `directory`, which it makes.
    pub async fn create(directory: PathBuf, cancel: &CancellationToken) -> io::Result<Self> {
        tokio::fs::create_dir(&directory).await?;
        let head = directory.join("head");
        // Out of open files, the output waits for one (`runner.md` § Load).
        let file = demi_command_sdk::descriptors::retry(cancel, || {
            tokio::fs::File::create(&head)
        })
        .await?;
        Ok(Self {
            directory,
            head: Some(file),
            segment: None,
            layout: Arc::new(Mutex::new(Layout {
                head,
                head_length: 0,
                segments: VecDeque::new(),
                end_length: 0,
                left_out: 0,
                next_segment: 0,
            })),
        })
    }

    /// Keeps one read of `stream`: in the head while its record fits there,
    /// and in the end after.
    pub async fn write(&mut self, stream: OutputStream, bytes: &[u8]) -> io::Result<()> {
        let Some(head) = self.head.as_mut() else {
            return self.write_end(stream, bytes).await;
        };
        let room = (JOB_KEPT_PART_BYTES as u64).saturating_sub(lock(&self.layout).head_length);
        let (taken, record) = fitting(stream, bytes, usize::try_from(room).unwrap_or(usize::MAX))?;
        if taken > 0 {
            // Flushed before the layout counts it, so a snapshot reads only
            // what the file holds.
            head.write_all(&record).await?;
            head.flush().await?;
            lock(&self.layout).head_length += record.len() as u64;
        }
        if taken < bytes.len() {
            // The head is full; every later read goes to the end, after it.
            self.head = None;
            self.write_end(stream, &bytes[taken..]).await?;
        }
        Ok(())
    }

    async fn write_end(&mut self, stream: OutputStream, bytes: &[u8]) -> io::Result<()> {
        let full = lock(&self.layout)
            .segments
            .back()
            .is_none_or(|segment| segment.length >= SEGMENT_BYTES);
        if full || self.segment.is_none() {
            let path = {
                let mut layout = lock(&self.layout);
                let path = self.directory.join(format!("end-{}", layout.next_segment));
                layout.next_segment += 1;
                path
            };
            let file = tokio::fs::File::create(&path).await?;
            lock(&self.layout).segments.push_back(Segment {
                path,
                length: 0,
                output: 0,
            });
            self.segment = Some(file);
        }
        let record = record(stream, bytes)?;
        let file = self.segment.as_mut().expect("a segment is open");
        file.write_all(&record).await?;
        file.flush().await?;
        let dropped = {
            let mut layout = lock(&self.layout);
            let segment = layout.segments.back_mut().expect("a segment is open");
            segment.length += record.len() as u64;
            segment.output += bytes.len() as u64;
            layout.end_length += record.len() as u64;
            let mut dropped = Vec::new();
            while layout.end_length > JOB_KEPT_PART_BYTES as u64 && layout.segments.len() > 1 {
                let oldest = layout.segments.pop_front().expect("more than one segment");
                layout.end_length -= oldest.length;
                layout.left_out += oldest.output;
                dropped.push(oldest.path);
            }
            dropped
        };
        // A snapshot that opened a dropped segment reads it to its end all
        // the same.
        for path in dropped {
            tokio::fs::remove_file(path).await?;
        }
        Ok(())
    }

    /// Reads the kept output while the job runs, and after it ended.
    pub fn reader(&self) -> KeptReader {
        KeptReader(self.layout.clone())
    }
}

/// Takes snapshots of a job's kept output.
#[derive(Clone)]
pub struct KeptReader(Arc<Mutex<Layout>>);

impl KeptReader {
    /// The kept output as it stands: its files, opened while the writer
    /// waits, each up to the length written so far. It opens files, so it
    /// runs on a blocking thread.
    pub fn snapshot(&self) -> io::Result<Snapshot> {
        let layout = lock(&self.0);
        let head = (std::fs::File::open(&layout.head)?, layout.head_length);
        let segments = layout
            .segments
            .iter()
            .map(|segment| Ok((std::fs::File::open(&segment.path)?, segment.length)))
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Snapshot {
            head,
            left_out: layout.left_out,
            segments,
        })
    }
}

/// A kept output as it stood when it was taken.
pub struct Snapshot {
    head: (std::fs::File, u64),
    left_out: u64,
    segments: Vec<(std::fs::File, u64)>,
}

impl Snapshot {
    /// The snapshot's records: the head, where output was left out the
    /// record that says how much, then the end.
    pub fn into_stream(self) -> io::Result<impl Stream<Item = io::Result<Bytes>> + Send + 'static> {
        let mut parts = vec![part(self.head)];
        if self.left_out > 0 {
            let record = wire::encode_record(&KeptRecord::LeftOut(self.left_out))
                .map_err(io::Error::other)?;
            parts.push(futures_util::stream::once(async move { Ok(Bytes::from(record)) }).boxed());
        }
        parts.extend(self.segments.into_iter().map(part));
        Ok(futures_util::stream::iter(parts).flatten())
    }
}

fn part((file, length): (std::fs::File, u64)) -> BoxStream<'static, io::Result<Bytes>> {
    let file = tokio::fs::File::from_std(file).take(length);
    tokio_util::io::ReaderStream::with_capacity(file, CHUNK_BYTES).boxed()
}

/// The longest start of `bytes` whose record takes at most `room` bytes, and
/// that record: how much of a read still fits in the head.
fn fitting(stream: OutputStream, bytes: &[u8], room: usize) -> io::Result<(usize, Vec<u8>)> {
    let mut taken = bytes.len().min(room);
    while taken > 0 {
        let record = record(stream, &bytes[..taken])?;
        if record.len() <= room {
            return Ok((taken, record));
        }
        // A record's framing does not grow as its read shrinks, so a read
        // shorter by the excess fits.
        taken = taken.saturating_sub(record.len() - room);
    }
    Ok((0, Vec::new()))
}

fn record(stream: OutputStream, bytes: &[u8]) -> io::Result<Vec<u8>> {
    wire::encode_record(&KeptRecord::Output(stream, WireBytes(bytes.to_vec())))
        .map_err(io::Error::other)
}

fn lock(layout: &Mutex<Layout>) -> MutexGuard<'_, Layout> {
    // No section panics while it holds the lock, so a poisoned one still
    // holds a whole layout.
    layout.lock().unwrap_or_else(PoisonError::into_inner)
}
