//! A recording's files and how they are read.
//!
//! ```text
//! <directory>/r<number>/
//!   recording.json   what it records, and how it ended once it has
//!   frames.jsonl     one line for each picture or run of frames missed
//!   inputs.jsonl     one line for each input the screen took, in time order
//!   video.h264       the pictures' H.264, Annex B, one coded picture each
//! ```
//!
//! A source codes only the pictures that differ from the one before, and says
//! where each differs, so each picture of the video is a frame.
//!
//! Everything a listing says of a recording, how many frames it kept, where
//! they changed, how many it missed, is read from `frames.jsonl`: nothing
//! derivable is stored a second time. A line is written, and the video's bytes
//! it names flushed, before the next frame is taken, so a recording is readable
//! while it runs and after its program ended.

use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};

use demi_shared_recording_protocol::{Area, EndReason, FrameRow, FrameSize, Gap, Input, InputRow, RecordingId, RecordingInfo};
use serde::{Deserialize, Serialize};

use crate::Error;

pub(crate) const META: &str = "recording.json";
pub(crate) const INDEX: &str = "frames.jsonl";
pub(crate) const VIDEO: &str = "video.h264";
pub(crate) const INPUTS: &str = "inputs.jsonl";
/// The file a program holds an exclusive lock on while it writes a
/// recording, which the system releases when the program ends, however it
/// does: whether another program still writes a recording is whether the lock
/// is held.
pub(crate) const WRITING: &str = "writing";

/// Whether a program, this one or another, writes the recording in
/// `directory` now.
pub(crate) fn written(directory: &Path) -> bool {
    let Ok(file) = fs::File::open(directory.join(WRITING)) else {
        return false;
    };
    // A lock taken is let go at once: nothing writes here, and nothing starts
    // writing in a recording that began.
    matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Meta {
    pub target: String,
    pub ended: Option<Ended>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Ended {
    pub reason: EndReason,
    pub note: Option<String>,
    /// How long it ran, from its first frame.
    pub duration_us: u64,
}

/// One frame of the video: a picture that differs from the one before.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Kept {
    /// Microseconds from the first frame.
    pub t: u64,
    /// Where it differs from the frame before.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Its size.
    pub fw: u32,
    pub fh: u32,
    /// Its bytes in the video, and whether it is a key frame.
    pub o: u64,
    pub l: u32,
    pub key: bool,
    /// When it arrived, in microseconds from the first frame's arrival;
    /// none in a recording an earlier release made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a: Option<u64>,
}

impl Kept {
    pub fn changed(&self) -> Area {
        Area {
            x: self.x,
            y: self.y,
            width: self.w,
            height: self.h,
        }
    }

    pub fn size(&self) -> FrameSize {
        FrameSize {
            width: self.fw,
            height: self.fh,
        }
    }

    /// Whether it differs everywhere, as the first frame and one of another
    /// size do.
    pub fn whole(&self) -> bool {
        (self.x, self.y, self.w, self.h) == (0, 0, self.fw, self.fh)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", deny_unknown_fields)]
pub(crate) enum Entry {
    #[serde(rename = "f")]
    Frame(Kept),
    #[serde(rename = "m")]
    Missed { after: u32, count: u32 },
}

/// What a recording's files hold now.
pub(crate) struct Contents {
    pub meta: Meta,
    /// Every frame of the video, numbered from 1 in this order.
    pub frames: Vec<Kept>,
    pub gaps: Vec<Gap>,
    /// Every input the screen took, in time order.
    pub inputs: Vec<InputRow>,
}

impl Contents {
    pub fn read(directory: &Path) -> Result<Self, Error> {
        let meta = fs::read(directory.join(META)).map_err(|error| Error::io(directory, &error))?;
        let meta: Meta = serde_json::from_slice(&meta)
            .map_err(|error| Error::Corrupt(format!("{}: {error}", directory.join(META).display())))?;
        let mut text = String::new();
        fs::File::open(directory.join(INDEX))
            .and_then(|mut file| file.read_to_string(&mut text))
            .map_err(|error| Error::io(directory, &error))?;
        // A line a program killed mid-write left unfinished is no frame.
        let finished = text.rfind('\n').map_or(0, |end| end + 1);
        let (mut frames, mut gaps) = (Vec::new(), Vec::new());
        for line in text[..finished].lines() {
            match serde_json::from_str::<Entry>(line)
                .map_err(|error| Error::Corrupt(format!("{}: {error}", directory.join(INDEX).display())))?
            {
                Entry::Frame(kept) => frames.push(kept),
                Entry::Missed { after, count } => gaps.push(Gap { after, count }),
            }
        }
        let mut inputs = inputs(directory)?;
        // An input is timed by when it was taken, from when the first frame
        // arrived; a frame by when it was drawn. A frame arrives the time its
        // capture and coding take after it was drawn, the first one maybe
        // later than most, so the input's time moves by how much sooner than
        // the first any frame arrived: the inputs then sit as far before the
        // frames as they were taken before the screen was drawn.
        let sooner = frames
            .iter()
            .filter_map(|kept| kept.a.map(|arrived| kept.t.saturating_sub(arrived)))
            .max()
            .unwrap_or(0);
        // One taken before the first frame arrived is at the start, which
        // stays its time, as a note's is the one it was set at; none is
        // after the recording's end.
        let end = meta.ended.as_ref().map(|ended| ended.duration_us);
        for row in &mut inputs {
            if row.time_us > 0 && !matches!(row.input, Input::Note { .. }) {
                row.time_us = row.time_us.saturating_add(sooner);
            }
            if let Some(end) = end {
                row.time_us = row.time_us.min(end);
            }
        }
        // A note set after the inputs about it is in order with them.
        inputs.sort_by_key(|row| row.time_us);
        Ok(Self { meta, frames, gaps, inputs })
    }

    /// The recording as `start` and `stop` tell it. `running` is whether this
    /// program still writes it; a recording with no end that none writes was
    /// interrupted.
    pub fn info(&self, id: &RecordingId, running: bool, ran_us: u64) -> RecordingInfo {
        let (ended, note, duration_us) = match (&self.meta.ended, running) {
            (Some(ended), _) => (Some(ended.reason), ended.note.clone(), ended.duration_us),
            (None, true) => (None, None, ran_us),
            (None, false) => (
                Some(EndReason::Interrupted),
                None,
                self.frames.last().map_or(0, |last| last.t),
            ),
        };
        RecordingInfo {
            id: id.clone(),
            target: self.meta.target.clone(),
            running: ended.is_none(),
            ended,
            note,
            duration_ms: duration_us / 1000,
            frames: self.frames.len() as u64,
            missed: self.gaps.iter().map(|gap| u64::from(gap.count)).sum(),
            area: self
                .frames
                .iter()
                .map(Kept::changed)
                .reduce(Area::union)
                .filter(|_| !self.frames.is_empty()),
            size: self.frames.first().map(Kept::size),
        }
    }

    pub fn rows(&self) -> impl Iterator<Item = FrameRow> + '_ {
        self.frames.iter().enumerate().map(|(index, kept)| FrameRow {
            frame: index as u32 + 1,
            time_us: kept.t,
            changed: kept.changed(),
            whole: kept.whole(),
        })
    }
}

/// The inputs of the recording in `directory`; none for one an earlier
/// release made, which kept no inputs.
fn inputs(directory: &Path) -> Result<Vec<InputRow>, Error> {
    let path = directory.join(INPUTS);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(directory, &error)),
    };
    // A line a program killed mid-write left unfinished is no input.
    let finished = text.rfind('\n').map_or(0, |end| end + 1);
    let mut rows = text[..finished]
        .lines()
        .map(|line| serde_json::from_str::<InputRow>(line).map_err(|error| Error::Corrupt(format!("{}: {error}", path.display()))))
        .collect::<Result<Vec<_>, _>>()?;
    // A command's line is written when it ends, at the time it began.
    rows.sort_by_key(|row| row.time_us);
    Ok(rows)
}

/// Adds `row` to the inputs of the recording in `directory`, which no
/// program writes: a note set after it ended.
pub(crate) fn append_input(directory: &Path, row: &InputRow) -> Result<(), Error> {
    use std::io::{Seek as _, Write as _};
    let path = directory.join(INPUTS);
    let mut line = serde_json::to_vec(row).expect("an input serializes");
    line.push(b'\n');
    let io = |error: std::io::Error| Error::io(&path, &error);
    let mut file = fs::OpenOptions::new().create(true).read(true).write(true).truncate(false).open(&path).map_err(io)?;
    // Held while the file is put right and added to, against a note another
    // program sets at the same moment; the system lets it go with the file.
    file.lock().map_err(io)?;
    let mut text = Vec::new();
    file.read_to_end(&mut text).map_err(io)?;
    // A line a program killed mid-write left unfinished, which a reading
    // skips, would join the note's into one that no reading can.
    let finished = text.iter().rposition(|byte| *byte == b'\n').map_or(0, |end| end + 1);
    file.set_len(finished as u64).map_err(io)?;
    file.seek(std::io::SeekFrom::Start(finished as u64)).map_err(io)?;
    file.write_all(&line).map_err(io)
}

/// The recordings in `directory`, oldest first.
pub(crate) fn listed(directory: &Path) -> Vec<(RecordingId, PathBuf)> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut found: Vec<(RecordingId, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let id = entry.file_name().to_str()?.parse::<RecordingId>().ok()?;
            Some((id, entry.path()))
        })
        .collect();
    found.sort_by_key(|(id, _)| id.number());
    found
}
