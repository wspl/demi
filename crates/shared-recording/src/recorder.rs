//! The thread that keeps a recording's frames, and the handles that feed it.
//!
//! A source codes each frame its screen drew that differs from the one before
//! and hands the coded picture to a [`Sink`], with where it differs. The sink
//! never waits: the thread only writes, and a source that finds it failing is
//! told so, which ends the recording. The thread writes each picture to the
//! video and its line to the index.
//!
//! ```text
//! source --push_picture--> Sink --queue--> thread: video + index
//!                            ^                       ^
//!                            +-- end(reason) --------+-- the deadline, the size
//! ```
//!
//! The thread also ends the recording: at its duration, at the longest a
//! recording runs, and at the most its files may take. Whoever ends it, the
//! queued pictures are kept first, and [`Live::ended`] is cancelled once the
//! files are complete.
//!
//! A source also hands over the inputs its screen took, each with the instant
//! it took it, which the thread writes to `inputs.jsonl` in the frames' time:
//! an instant is measured from when the first picture arrived, the frames'
//! zero, so an input shows a little before the frame that shows its effect,
//! as the capture and its coding take a moment. One taken before the first
//! picture waits for it, and is at zero.
//!
//! A source with an encoder that works behind its calls, as every hardware
//! encoder does, holds pictures a stop must not lose, since a loss at the end
//! is the one a model looks for: [`Live::stop`] asks it, through
//! [`Handover`], to deliver what it holds before the recording ends.

use std::{
    collections::VecDeque,
    fs,
    io::Write as _,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use demi_shared_recording_protocol::{
    Area, EndReason, Input, InputRow, MAX_BYTES, MAX_DURATION, RecordingId, RecordingInfo,
};
use tokio_util::sync::CancellationToken;

use crate::{
    Error,
    stored::{Contents, Ended, Entry, INDEX, INPUTS, Kept, META, Meta, VIDEO, WRITING},
};

/// A picture a source coded: H.264 Annex B, one access unit.
pub struct Picture {
    pub data: Vec<u8>,
    pub key: bool,
    /// When the source drew it, in microseconds on a clock of its own.
    pub at_us: f64,
    pub width: u32,
    pub height: u32,
    /// Where it differs from the picture before; the whole of it for the
    /// first.
    pub changed: Area,
}

/// The most coded bytes the queue holds before the thread, which only writes
/// them, is taken to be failing.
const PICTURE_BYTES: usize = 64 * 1024 * 1024;

/// What the sink handed over, in the order it did.
enum Item {
    Picture(Picture, Duration),
    /// An input and when it was taken, from the recording's start.
    Input(Input, Duration),
    /// Frames the screen drew and the source could not give.
    Missed(u32),
}

#[derive(Default)]
struct Queue {
    items: VecDeque<Item>,
    finish: Option<(EndReason, Option<String>)>,
    /// The coded bytes waiting.
    picture_bytes: usize,
}

impl Queue {
    /// Counts `count` missed frames after those queued, in the run that
    /// ends the queue when it is one.
    fn miss(&mut self, count: u32) {
        match self.items.back_mut() {
            Some(Item::Missed(run)) => *run = run.saturating_add(count),
            _ => self.items.push_back(Item::Missed(count)),
        }
    }
}

/// How a source that holds pictures behind its encoder hands them over
/// before its recording ends.
#[derive(Clone, Default)]
pub struct Handover {
    requested: CancellationToken,
    done: CancellationToken,
}

impl Handover {
    /// Cancelled when [`Live::stop`] wants the source to deliver what it
    /// holds.
    pub fn requested(&self) -> &CancellationToken {
        &self.requested
    }

    /// The source has delivered everything it had.
    pub fn done(&self) {
        self.done.cancel();
    }
}

/// How long a stop waits for the source's handover.
const HANDOVER: Duration = Duration::from_secs(5);

struct Shared {
    /// The size of the last picture handed over, packed as width and
    /// height, which an input's place is scaled by; 0 before the first.
    size: std::sync::atomic::AtomicU64,
    handover: Handover,
    queue: Mutex<Queue>,
    wake: Condvar,
    began: Instant,
    ended: CancellationToken,
}

/// Where a source puts the pictures it coded; cloning shares it.
#[derive(Clone)]
pub struct Sink(Arc<Shared>);

impl Sink {
    /// Hands over a coded picture. Whether it was taken: not once the
    /// recording ended, or when the pictures wait in more bytes than a thread
    /// that only writes them would let pile up, which ends the recording
    /// failed.
    pub fn push_picture(&self, picture: Picture) -> bool {
        let arrived = self.0.began.elapsed();
        self.0.size.store(
            (u64::from(picture.width) << 32) | u64::from(picture.height),
            std::sync::atomic::Ordering::Relaxed,
        );
        let mut queue = self.0.queue.lock().expect("the recording's queue is intact");
        if queue.finish.is_some() {
            return false;
        }
        if queue.picture_bytes + picture.data.len() > PICTURE_BYTES {
            queue.finish = Some((EndReason::Failed, Some("the recording's disk could not keep up".into())));
            self.0.wake.notify_one();
            return false;
        }
        queue.picture_bytes += picture.data.len();
        queue.items.push_back(Item::Picture(picture, arrived));
        self.0.wake.notify_one();
        true
    }

    /// The width and height of the last picture handed over: the pixels an
    /// input's place is in. None before the first.
    pub fn frame_size(&self) -> Option<(u32, u32)> {
        let size = self.0.size.load(std::sync::atomic::Ordering::Relaxed);
        (size != 0).then(|| ((size >> 32) as u32, size as u32))
    }

    /// Hands over an input the source took at `at`. A recording that ended
    /// takes none.
    pub fn input(&self, at: Instant, input: Input) {
        let taken = at.saturating_duration_since(self.0.began);
        let mut queue = self.0.queue.lock().expect("the recording's queue is intact");
        if queue.finish.is_some() {
            return;
        }
        queue.items.push_back(Item::Input(input, taken));
        self.0.wake.notify_one();
    }

    /// Counts `count` frames the screen drew and the source could not give.
    pub fn miss(&self, count: u32) {
        let mut queue = self.0.queue.lock().expect("the recording's queue is intact");
        queue.miss(count);
        self.0.wake.notify_one();
    }

    /// Ends the recording for `reason`, once the pictures already handed over
    /// are kept; the first reason stands.
    pub fn end(&self, reason: EndReason, note: Option<String>) {
        let mut queue = self.0.queue.lock().expect("the recording's queue is intact");
        queue.finish.get_or_insert((reason, note));
        self.0.wake.notify_one();
    }

    /// Cancelled once the recording's files are complete.
    pub fn ended(&self) -> &CancellationToken {
        &self.0.ended
    }

    /// How the source hands over what it holds at the end.
    pub fn handover(&self) -> &Handover {
        &self.0.handover
    }
}

/// A recording this program writes.
#[derive(Clone)]
pub struct Live {
    id: RecordingId,
    directory: PathBuf,
    sink: Sink,
    limit: (Duration, EndReason),
}

impl Live {
    pub fn id(&self) -> &RecordingId {
        &self.id
    }

    pub fn sink(&self) -> Sink {
        self.sink.clone()
    }

    /// How its source hands over what it holds at the end.
    pub fn handover(&self) -> &Handover {
        &self.sink.0.handover
    }

    /// Waits until the recording has to end, and says why: `closed` fired,
    /// the source's device or tab, or its limit came. None when it ended on
    /// its own first. The source then ends it by [`Self::stop`], so that what
    /// it holds is kept.
    pub async fn over(&self, closed: &CancellationToken) -> Option<EndReason> {
        let (limit, limited) = self.limit;
        tokio::select! {
            () = self.ended().cancelled() => None,
            () = closed.cancelled() => Some(EndReason::Closed),
            () = tokio::time::sleep(limit) => Some(limited),
        }
    }

    /// Whether it was told to end, and is keeping what it held before.
    pub fn finishing(&self) -> bool {
        self.sink.0.queue.lock().expect("the recording's queue is intact").finish.is_some()
    }

    /// Cancelled once the recording ended, whatever ended it.
    pub fn ended(&self) -> &CancellationToken {
        self.sink.ended()
    }

    /// What the recording is now.
    pub fn info(&self) -> Result<RecordingInfo, Error> {
        let ran = u64::try_from(self.sink.0.began.elapsed().as_micros()).unwrap_or(u64::MAX);
        Ok(Contents::read(&self.directory)?.info(&self.id, !self.ended().is_cancelled(), ran))
    }

    /// Ends it for `reason` and answers once its files are complete.
    pub async fn stop(&self, reason: EndReason) -> Result<RecordingInfo, Error> {
        if !self.ended().is_cancelled() {
            let handover = &self.sink.0.handover;
            handover.requested.cancel();
            // A source that is gone hands nothing over; the recording ends
            // with what it has.
            let _late = tokio::time::timeout(HANDOVER, handover.done.cancelled()).await;
        }
        self.sink.end(reason, None);
        self.ended().cancelled().await;
        self.info()
    }
}

/// Starts the thread that writes the recording in `directory`, a new one.
pub(crate) fn start(
    id: RecordingId,
    directory: PathBuf,
    target: &str,
    duration: Option<Duration>,
) -> Result<Live, Error> {
    let meta = Meta {
        target: target.to_owned(),
        ended: None,
    };
    write_meta(&directory, &meta)?;
    let video = fs::File::create(directory.join(VIDEO)).map_err(|error| Error::io(&directory, &error))?;
    let index = fs::File::create(directory.join(INDEX)).map_err(|error| Error::io(&directory, &error))?;
    let inputs = fs::File::create(directory.join(INPUTS)).map_err(|error| Error::io(&directory, &error))?;
    // Held until the writer ends, so that no program prunes the recording
    // while it is written.
    let writing = fs::File::create(directory.join(WRITING)).map_err(|error| Error::io(&directory, &error))?;
    writing.lock().map_err(|error| Error::io(&directory, &error))?;
    let limit = match duration {
        Some(duration) if duration < MAX_DURATION => (duration, EndReason::Duration),
        _ => (MAX_DURATION, EndReason::TimeLimit),
    };
    let shared = Arc::new(Shared {
        size: std::sync::atomic::AtomicU64::new(0),
        handover: Handover::default(),
        queue: Mutex::default(),
        wake: Condvar::new(),
        began: Instant::now(),
        ended: CancellationToken::new(),
    });
    let writer = Writer {
        shared: shared.clone(),
        directory: directory.clone(),
        meta,
        video,
        index,
        inputs,
        waiting: Vec::new(),
        _writing: writing,
        first_picture: None,
        origin: None,
        kept: 0,
        last_time: 0,
        video_bytes: 0,
        bytes: 0,
        // The source ends its recording at the limit itself, handing over
        // what it holds first; this deadline is the backstop.
        deadline: shared.began + limit.0 + HANDOVER * 2,
        deadline_reason: limit.1,
    };
    std::thread::Builder::new()
        .name("demi-recording".into())
        .spawn(move || writer.run())
        .map_err(|error| Error::Failed(format!("the recording's thread did not start: {error}")))?;
    Ok(Live {
        id,
        directory,
        sink: Sink(shared),
        limit,
    })
}

pub(crate) fn write_meta(directory: &std::path::Path, meta: &Meta) -> Result<(), Error> {
    let partial = directory.join(format!("{META}.partial"));
    fs::write(&partial, serde_json::to_vec(meta).expect("a recording's meta serializes"))
        .and_then(|()| fs::rename(&partial, directory.join(META)))
        .map_err(|error| Error::io(directory, &error))
}

struct Writer {
    shared: Arc<Shared>,
    directory: PathBuf,
    meta: Meta,
    video: fs::File,
    index: fs::File,
    inputs: fs::File,
    /// The inputs taken before the first picture arrived, which wait for it.
    waiting: Vec<(Input, Duration)>,
    /// The lock that says the recording is being written.
    _writing: fs::File,
    /// The first picture's time on its source's clock.
    first_picture: Option<f64>,
    /// When the first picture arrived, from the recording's start.
    origin: Option<Duration>,
    kept: u32,
    /// The time of the frame kept last, in microseconds.
    last_time: u64,
    /// How many bytes the video holds.
    video_bytes: u64,
    bytes: u64,
    deadline: Instant,
    deadline_reason: EndReason,
}

enum Work {
    Picture(Picture, Duration),
    Input(Input, Duration),
    Missed(u32),
    Finish(EndReason, Option<String>),
}

impl Writer {
    fn run(mut self) {
        let (reason, note) = loop {
            match self.next() {
                Work::Picture(picture, arrived) => {
                    if let Err(error) = self.keep(picture, arrived) {
                        break (EndReason::Failed, Some(error.to_string()));
                    }
                    if self.bytes >= MAX_BYTES {
                        break (EndReason::SizeLimit, None);
                    }
                }
                Work::Missed(missed) => {
                    if let Err(error) = self.note_missed(missed) {
                        break (EndReason::Failed, Some(error.to_string()));
                    }
                }
                Work::Input(input, taken) => {
                    if let Err(error) = self.note_input(input, taken) {
                        break (EndReason::Failed, Some(error.to_string()));
                    }
                }
                Work::Finish(reason, note) => break (reason, note),
            }
        };
        // What the queue still holds was drawn before the end.
        let late = {
            let mut queue = self.shared.queue.lock().expect("the recording's queue is intact");
            queue.finish.get_or_insert((reason, note.clone()));
            std::mem::take(&mut queue.items)
        };
        let mut note = note;
        let mut reason = reason;
        if reason != EndReason::Failed {
            let mut failure = None;
            for item in late {
                if failure.is_none() {
                    failure = match item {
                        Item::Picture(picture, arrived) => self.keep(picture, arrived).err(),
                        Item::Missed(count) => self.note_missed(count).err(),
                        Item::Input(input, taken) => self.note_input(input, taken).err(),
                    };
                }
            }
            if let Some(error) = failure {
                reason = EndReason::Failed;
                note = Some(error.to_string());
            }
        }
        // Inputs of a recording that kept no picture are at its start.
        self.origin.get_or_insert(Duration::ZERO);
        if let Err(error) = self.flush_waiting()
            && reason != EndReason::Failed
        {
            reason = EndReason::Failed;
            note = Some(error.to_string());
        }
        let ran = self.ran().max(Duration::from_micros(self.last_time));
        self.meta.ended = Some(Ended {
            reason,
            note,
            duration_us: u64::try_from(ran.as_micros()).unwrap_or(u64::MAX),
        });
        if let Err(error) = write_meta(&self.directory, &self.meta) {
            tracing::warn!(event = "recording.end_write_failed", %error, "a recording's end was not written");
        }
        self.shared.ended.cancel();
    }

    /// How long the recording ran: from its first frame, or from its start
    /// while it has none.
    fn ran(&self) -> Duration {
        let now = self.shared.began.elapsed();
        match self.origin {
            Some(arrived) => now.saturating_sub(arrived),
            None => now,
        }
    }

    fn next(&mut self) -> Work {
        let mut queue = self.shared.queue.lock().expect("the recording's queue is intact");
        loop {
            match queue.items.pop_front() {
                Some(Item::Picture(picture, arrived)) => {
                    queue.picture_bytes -= picture.data.len();
                    return Work::Picture(picture, arrived);
                }
                Some(Item::Missed(count)) => return Work::Missed(count),
                Some(Item::Input(input, taken)) => return Work::Input(input, taken),
                None => {}
            }
            if let Some((reason, note)) = queue.finish.take() {
                return Work::Finish(reason, note);
            }
            let now = Instant::now();
            if now >= self.deadline {
                return Work::Finish(self.deadline_reason, None);
            }
            queue = self
                .shared
                .wake
                .wait_timeout(queue, self.deadline - now)
                .expect("the recording's queue is intact")
                .0;
        }
    }

    fn note_missed(&mut self, count: u32) -> Result<(), Error> {
        if count == 0 {
            return Ok(());
        }
        self.write_entry(&Entry::Missed {
            after: self.kept,
            count,
        })
    }

    fn write_entry(&mut self, entry: &Entry) -> Result<(), Error> {
        let mut line = serde_json::to_vec(entry).expect("an index entry serializes");
        line.push(b'\n');
        self.bytes += line.len() as u64;
        self.index
            .write_all(&line)
            .map_err(|error| Error::io(&self.directory, &error))
    }

    /// Writes `input`, taken `taken` after the start, in the frames' time,
    /// or keeps it until the first picture gives that time its zero.
    fn note_input(&mut self, input: Input, taken: Duration) -> Result<(), Error> {
        let Some(origin) = self.origin else {
            self.waiting.push((input, taken));
            return Ok(());
        };
        let row = InputRow {
            time_us: u64::try_from(taken.saturating_sub(origin).as_micros()).unwrap_or(u64::MAX),
            input,
        };
        let mut line = serde_json::to_vec(&row).expect("an input serializes");
        line.push(b'\n');
        self.bytes += line.len() as u64;
        self.inputs
            .write_all(&line)
            .map_err(|error| Error::io(&self.directory, &error))
    }

    /// Writes the inputs that waited for the first picture.
    fn flush_waiting(&mut self) -> Result<(), Error> {
        for (input, taken) in std::mem::take(&mut self.waiting) {
            self.note_input(input, taken)?;
        }
        Ok(())
    }

    /// Keeps a coded picture as it came, and says where it is in the video.
    fn keep(&mut self, picture: Picture, arrived: Duration) -> Result<(), Error> {
        if self.origin.is_none() {
            self.origin = Some(arrived);
            self.flush_waiting()?;
        }
        let origin = *self.first_picture.get_or_insert(picture.at_us);
        let time = (picture.at_us - origin).max(0.0) as u64;
        // Time never runs back, whatever the source's clock does.
        self.last_time = self.last_time.max(time);
        let offset = self.video_bytes;
        self.video_bytes += picture.data.len() as u64;
        self.video
            .write_all(&picture.data)
            .map_err(|error| Error::io(&self.directory, &error))?;
        self.bytes += picture.data.len() as u64;
        self.write_entry(&Entry::Frame(Kept {
            t: self.last_time,
            x: picture.changed.x,
            y: picture.changed.y,
            w: picture.changed.width,
            h: picture.changed.height,
            fw: picture.width,
            fh: picture.height,
            o: offset,
            l: picture.data.len() as u32,
            key: picture.key,
            a: Some(u64::try_from(arrived.saturating_sub(self.origin.unwrap_or_default()).as_micros()).unwrap_or(u64::MAX)),
        }))?;
        self.kept += 1;
        Ok(())
    }
}
