//! A job's kept output: every read of its stdout and stderr, in the order the
//! runner read them, one record per read in the wire's encoding, which the
//! backend decodes with the wire's types.
//!
//! The bound counts the records' bytes, so it bounds the disk exactly:
//!
//! - Up to `JOB_KEPT_BYTES` every read is kept.
//! - Beyond that the first half stays whole in `head`, and the last half in
//!   segments of 1 MiB, of which the oldest goes as new reads come.
//! - One record between them counts the bytes left out.
//!
//! A job that prints without end, such as `yes`, therefore holds at most 16 MiB
//! of the device's disk. `job_read` streams a snapshot, while the job runs and
//! after.

use std::{
    collections::VecDeque,
    io,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use bytes::Bytes;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use demi_runner_process::pipes::UploadBody;
use demi_runner_protocol::wire::{self, JOB_KEPT_PART_BYTES, KeptRecord, OutputStream, WireBytes};

/// How many bytes of records one segment of the kept output's end holds
/// before the next one begins: the granularity at which the end lets old
/// output go, so that the disk it takes stays bounded.
const SEGMENT_BYTES: u64 = 1024 * 1024;

/// The file that counts the output of each stream the segments that went
/// held, with the number of the first segment kept, which
/// [`KeptOutput::open`] reads. It is written whole, before the segments go,
/// so a writer that stopped between the two left segments the count already
/// holds, which the next open removes.
const LEFT_OUT: &str = "left-out";

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
    /// The output left out between the head and the segments, of each
    /// stream, stdout first.
    left_out: [u64; 2],
    next_segment: u64,
}

struct Segment {
    path: PathBuf,
    /// Its length, in bytes of records.
    length: u64,
    /// The output it holds, of each stream, stdout first.
    output: [u64; 2],
}

impl KeptOutput {
    /// Starts an empty kept output in `directory`, which it makes.
    pub async fn create(directory: PathBuf, cancel: &CancellationToken) -> io::Result<Self> {
        tokio::fs::create_dir(&directory).await?;
        let head = directory.join("head");
        // Out of open files, the output waits for one (`crates/command-sdk/src/descriptors.rs`).
        let file =
            demi_command_sdk::descriptors::retry(cancel, || tokio::fs::File::create(&head)).await?;
        Ok(Self {
            directory,
            head: Some(file),
            segment: None,
            layout: Arc::new(Mutex::new(Layout {
                head,
                head_length: 0,
                segments: VecDeque::new(),
                end_length: 0,
                left_out: [0; 2],
                next_segment: 0,
            })),
        })
    }

    /// Opens the kept output a runner before this one wrote in `directory`,
    /// to write on where it stopped, as a runner that takes a job over does
    /// (`crates/runner-shell/src/remote/mod.rs`). It answers each stream's
    /// length and its last `tail` bytes, stdout first. A record the writer
    /// left unfinished goes.
    pub async fn open(directory: PathBuf, tail: usize) -> io::Result<(Self, Reopened)> {
        tokio::task::spawn_blocking(move || Self::open_blocking(directory, tail))
            .await
            .map_err(io::Error::other)?
    }

    fn open_blocking(directory: PathBuf, tail: usize) -> io::Result<(Self, Reopened)> {
        let mut reopened = Reopened {
            lengths: [0; 2],
            tails: [Vec::new(), Vec::new()],
        };
        let mut read = |path: &std::path::Path| -> io::Result<(u64, [u64; 2])> {
            let bytes = std::fs::read(path)?;
            let (records, end) = wire::decode_whole_records(&bytes);
            if end < bytes.len() {
                std::fs::OpenOptions::new().write(true).open(path)?.set_len(end as u64)?;
            }
            let mut output = [0u64; 2];
            for record in records {
                if let KeptRecord::Output(stream, WireBytes(read)) = record {
                    let index = index(stream);
                    output[index] += read.len() as u64;
                    let kept = &mut reopened.tails[index];
                    kept.extend_from_slice(&read);
                    let excess = kept.len().saturating_sub(tail);
                    kept.drain(..excess);
                }
            }
            Ok((end as u64, output))
        };
        let head = directory.join("head");
        let (head_length, head_output) = read(&head)?;
        let (left_out, first) = match std::fs::read_to_string(directory.join(LEFT_OUT)) {
            Ok(text) => {
                let mut counts = text.split_whitespace().map(str::parse::<u64>);
                match (counts.next(), counts.next(), counts.next()) {
                    (Some(Ok(stdout)), Some(Ok(stderr)), Some(Ok(first))) => ([stdout, stderr], first),
                    _ => return Err(io::Error::other("the kept output's count of what went is unreadable")),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => ([0; 2], 0),
            Err(error) => return Err(error),
        };
        let mut numbers: Vec<u64> = std::fs::read_dir(&directory)?
            .flatten()
            .filter_map(|entry| segment_number(&entry.path()))
            .collect();
        numbers.sort_unstable();
        // Segments the count holds that a writer left on its way out.
        for number in numbers.iter().filter(|number| **number < first) {
            std::fs::remove_file(directory.join(format!("end-{number}")))?;
        }
        numbers.retain(|number| *number >= first);
        let mut segments = VecDeque::new();
        let mut end_length = 0;
        for number in &numbers {
            let path = directory.join(format!("end-{number}"));
            let (length, output) = read(&path)?;
            end_length += length;
            segments.push_back(Segment { path, length, output });
        }
        for index in 0..2 {
            reopened.lengths[index] = head_output[index]
                + left_out[index]
                + segments.iter().map(|segment| segment.output[index]).sum::<u64>();
        }
        let append = |path: &std::path::Path| -> io::Result<tokio::fs::File> {
            Ok(tokio::fs::File::from_std(std::fs::OpenOptions::new().append(true).open(path)?))
        };
        // The head takes reads until one did not fit; the end follows it.
        let (head_file, segment) = match segments.back() {
            None => (Some(append(&head)?), None),
            Some(last) => (None, Some(append(&last.path)?)),
        };
        let output = Self {
            directory,
            head: head_file,
            segment,
            layout: Arc::new(Mutex::new(Layout {
                head,
                head_length,
                segments,
                end_length,
                left_out,
                next_segment: numbers.last().map_or(0, |last| last + 1),
            })),
        };
        Ok((output, reopened))
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
                output: [0; 2],
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
            segment.output[index(stream)] += bytes.len() as u64;
            layout.end_length += record.len() as u64;
            let mut dropped = Vec::new();
            while layout.end_length > JOB_KEPT_PART_BYTES as u64 && layout.segments.len() > 1 {
                let oldest = layout.segments.pop_front().expect("more than one segment");
                layout.end_length -= oldest.length;
                layout.left_out[0] += oldest.output[0];
                layout.left_out[1] += oldest.output[1];
                dropped.push(oldest.path);
            }
            let first = layout.segments.front().map_or(layout.next_segment, |segment| {
                segment_number(&segment.path).unwrap_or(layout.next_segment)
            });
            (!dropped.is_empty()).then(|| (dropped, layout.left_out, first))
        };
        // What goes is counted before it goes, so a runner that opens the
        // kept output again counts each stream whole (`KeptOutput::open`).
        // A snapshot that opened a dropped segment reads it to its end all
        // the same.
        if let Some((dropped, left_out, first)) = dropped {
            let count = self.directory.join(LEFT_OUT);
            let writing = self.directory.join(format!("{LEFT_OUT}.new"));
            tokio::fs::write(&writing, format!("{} {} {first}\n", left_out[0], left_out[1])).await?;
            tokio::fs::rename(&writing, &count).await?;
            for path in dropped {
                tokio::fs::remove_file(path).await?;
            }
        }
        Ok(())
    }

    /// Reads the kept output while the job runs, and after it ended.
    pub fn reader(&self) -> KeptReader {
        KeptReader(self.layout.clone())
    }
}

/// What [`KeptOutput::open`] found of each stream, stdout first.
pub struct Reopened {
    pub lengths: [u64; 2],
    pub tails: [Vec<u8>; 2],
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
            left_out: layout.left_out[0] + layout.left_out[1],
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
    /// The snapshot's records as an upload: the head, where output was left
    /// out the record that says how much, then the end
    /// (`demi_runner_process::pipes`).
    pub fn into_upload(self) -> io::Result<UploadBody> {
        let (head, length) = self.head;
        let mut body = UploadBody::new().file(head, 0, length);
        if self.left_out > 0 {
            let record = wire::encode_record(&KeptRecord::LeftOut(self.left_out))
                .map_err(io::Error::other)?;
            body = body.bytes(Bytes::from(record));
        }
        for (segment, length) in self.segments {
            body = body.file(segment, 0, length);
        }
        Ok(body)
    }
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

/// The number of the end segment at `path`.
fn segment_number(path: &std::path::Path) -> Option<u64> {
    path.file_name()?.to_str()?.strip_prefix("end-")?.parse().ok()
}

/// A stream's place in a pair of counts, stdout first.
fn index(stream: OutputStream) -> usize {
    match stream {
        OutputStream::Stdout => 0,
        OutputStream::Stderr => 1,
    }
}
