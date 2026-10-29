//! A conversation's recordings on a Host: starting one, keeping the newest
//! few, and reading any of them.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use demi_shared_openh264::Codec;
use demi_shared_recording_protocol::{EndReason, FrameRow, FrameSet, Gap, Input, InputRow, KEPT, RecordingId, RecordingInfo};

/// How long a pause between keys may be for the keys to be one word typed.
const TYPING_US: u64 = 1_500_000;

use crate::{
    Error,
    recorder::{self, Live},
    stored::{self, Contents},
};

/// The recordings of one conversation on this Host, in one directory that
/// every program recording for the conversation shares: the ids are given
/// once across them, and the newest [`KEPT`] stay, whoever made them.
pub struct Library {
    directory: PathBuf,
    pub(crate) codec: Arc<dyn Codec>,
    live: Mutex<HashMap<RecordingId, Live>>,
}

impl Library {
    pub fn new(directory: PathBuf, codec: Arc<dyn Codec>) -> Self {
        Self {
            directory,
            codec,
            live: Mutex::default(),
        }
    }

    /// The directory the conversation's recordings are in.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Starts recording `target`, such as a tab or a device. It stops itself
    /// after `duration`, and at the limits of every recording.
    pub async fn begin(&self, target: &str, duration: Option<Duration>) -> Result<Live, Error> {
        fs::create_dir_all(&self.directory).map_err(|error| Error::io(&self.directory, &error))?;
        let (id, directory) = self.allocate()?;
        let live = match recorder::start(id.clone(), directory.clone(), target, duration) {
            Ok(live) => live,
            Err(error) => {
                let _ = fs::remove_dir_all(&directory);
                return Err(error);
            }
        };
        self.live.lock().expect("the live recordings are intact").insert(id, live.clone());
        if let Err(error) = self.prune(live.id()) {
            let _ = fs::remove_dir_all(&directory);
            return Err(error);
        }
        Ok(live)
    }

    /// A new recording's id and its directory, made.
    fn allocate(&self) -> Result<(RecordingId, PathBuf), Error> {
        let mut number = stored::listed(&self.directory).last().map_or(1, |(id, _)| id.number() + 1);
        loop {
            let id = RecordingId::numbered(number);
            let directory = self.directory.join(id.as_str());
            match fs::create_dir(&directory) {
                Ok(()) => return Ok((id, directory)),
                // Another program took it a moment ago.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => number += 1,
                Err(error) => return Err(Error::io(&directory, &error)),
            }
        }
    }

    /// Removes the oldest recordings beyond the newest [`KEPT`], none that
    /// any program still writes: another program records for the conversation
    /// too, in the same directory.
    fn prune(&self, newest: &RecordingId) -> Result<(), Error> {
        let mut listed = stored::listed(&self.directory);
        while listed.len() > KEPT {
            let Some(position) = listed.iter().position(|(id, directory)| id != newest && !stored::written(directory)) else {
                return Err(Error::TooManyRunning);
            };
            let (_, directory) = listed.remove(position);
            let _ = fs::remove_dir_all(directory);
        }
        Ok(())
    }

    /// Ends `live` and removes it, as a recording that never began.
    pub async fn discard(&self, live: &Live) {
        let _ = live.stop(EndReason::Failed).await;
        self.live.lock().expect("the live recordings are intact").remove(live.id());
        let _ = fs::remove_dir_all(self.directory.join(live.id().as_str()));
    }

    /// The recording this program writes under `id`, while it writes it.
    pub fn live(&self, id: &RecordingId) -> Option<Live> {
        let live = self.live.lock().expect("the live recordings are intact");
        live.get(id).filter(|live| !live.ended().is_cancelled()).cloned()
    }

    /// Every recording this program writes still.
    pub fn running(&self) -> Vec<Live> {
        let live = self.live.lock().expect("the live recordings are intact");
        live.values().filter(|live| !live.ended().is_cancelled()).cloned().collect()
    }

    /// The recording `id`, running or not.
    pub async fn stored(&self, id: &RecordingId) -> Result<Stored, Error> {
        let directory = self.directory.join(id.as_str());
        if !directory.join(stored::META).is_file() {
            // One older than the oldest kept was pushed out; another never was.
            let pushed_out = stored::listed(&self.directory).first().is_some_and(|(oldest, _)| id.number() < oldest.number());
            return Err(if pushed_out { Error::Removed(id.clone()) } else { Error::NoRecording(id.clone()) });
        }
        let live = self.live(id);
        // A recording that is ending has its files complete in a moment.
        if let Some(live) = live.as_ref().filter(|live| live.finishing()) {
            live.ended().cancelled().await;
        }
        let live = live.filter(|live| !live.ended().is_cancelled());
        Ok(Stored {
            id: id.clone(),
            directory,
            live,
        })
    }

    /// The recordings of `target`, newest first, running or not.
    pub async fn list(&self, target: &str) -> Result<Vec<RecordingInfo>, Error> {
        let mut found = Vec::new();
        for (id, _) in stored::listed(&self.directory).into_iter().rev() {
            let stored = self.stored(&id).await?;
            let info = stored.info()?;
            if info.target == target {
                found.push(info);
            }
        }
        Ok(found)
    }

    /// Ends this program's recordings, each with every frame it holds kept,
    /// and answers once their files are complete. Keeping the conversation's
    /// recordings is no program's to decide: they go when the conversation is
    /// archived or deleted.
    pub async fn finish(&self) {
        for live in self.running() {
            let _ = live.stop(EndReason::Closed).await;
        }
    }
}

/// The frames of a recording a listing shows.
pub struct Listing {
    pub recording: RecordingInfo,
    pub frames: Vec<FrameRow>,
    /// Where frames were missed between them.
    pub gaps: Vec<Gap>,
    /// The inputs `frames` lists from the first frame listed to the next
    /// frame after the last, or to the end.
    pub inputs: Vec<InputRow>,
}

/// A recording to read.
pub struct Stored {
    pub(crate) id: RecordingId,
    pub(crate) directory: PathBuf,
    pub(crate) live: Option<Live>,
}

impl Stored {
    pub fn id(&self) -> &RecordingId {
        &self.id
    }

    pub(crate) fn contents(&self) -> Result<Contents, Error> {
        Contents::read(&self.directory)
    }

    /// What the recording is now.
    pub fn info(&self) -> Result<RecordingInfo, Error> {
        match &self.live {
            Some(live) => live.info(),
            None => Ok(self.contents()?.info(&self.id, false, 0)),
        }
    }

    /// The frames `which` names, or every one, with what was missed between
    /// them.
    pub fn frames(&self, which: Option<&FrameSet>) -> Result<Listing, Error> {
        let contents = self.contents()?;
        let ran = match &self.live {
            Some(live) => live.info()?.duration_ms * 1000,
            None => 0,
        };
        let recording = contents.info(&self.id, self.live.is_some(), ran);
        let frames: Vec<FrameRow> = contents.rows().filter(|row| which.is_none_or(|which| which.contains(row.frame))).collect();
        // A listing of some frames has the inputs from its first frame to
        // the frame after its last; one of every frame has them all.
        let from = match which {
            Some(_) => frames.first().map_or(u64::MAX, |row| row.time_us),
            None => 0,
        };
        let until = match (which, frames.last()) {
            (Some(_), Some(last)) => contents.frames.get(last.frame as usize).map_or(u64::MAX, |next| next.t),
            _ => u64::MAX,
        };
        let mut inputs: Vec<InputRow> = Vec::new();
        // When the last key of the text typed last went down.
        let mut typed_at = 0;
        for row in contents.inputs.into_iter().filter(|row| row.input.listed() && row.time_us >= from && row.time_us < until) {
            // Text the user typed key by key is one line, as a word is.
            if let (Some(InputRow { input: Input::Typed { characters: count, by: last_by, .. }, .. }), Input::Typed { characters, by, .. }) =
                (inputs.last_mut(), &row.input)
                && last_by == by
                && row.time_us.saturating_sub(typed_at) < TYPING_US
            {
                *count += characters;
                typed_at = row.time_us;
                continue;
            }
            typed_at = row.time_us;
            inputs.push(row);
        }
        Ok(Listing {
            recording,
            frames,
            gaps: contents.gaps,
            inputs,
        })
    }

    /// The directory the recording's files are in.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}
