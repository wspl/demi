//! Operations a test supplies in place of the runner's: files in memory,
//! a watch the test speaks for, and a stream that echoes its input.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use demi_runner_direct::{
    Answer, ByteStream, FileRange, FileText, Listing, Operations, Scope, StreamActivity,
    StreamRequest, WatchStream,
};
use demi_runner_protocol::direct::{ChannelError, ChannelErrorCode};
use demi_runner_protocol::files::{DirectoryEntry, FileWatchMessage};
use futures_util::{StreamExt, future::BoxFuture};
use tokio::sync::mpsc;

/// What the operations hold and saw, which the test reads.
#[derive(Default)]
pub struct Held {
    /// Files by path, each with its version.
    pub files: BTreeMap<String, (Vec<u8>, u32)>,
    /// The scopes the operations acted in.
    pub scopes: Vec<Scope>,
    /// The paths of each watch's `paths` messages.
    pub watched: Vec<Vec<String>>,
    /// The streams opened: the operation each one's name binds, and the
    /// id its request carries.
    pub streams: Vec<(String, String)>,
    /// How many streams the runner gave an id, which is `s` and the count.
    pub given: usize,
}

#[derive(Clone)]
pub struct Fake {
    pub held: Arc<Mutex<Held>>,
    /// How many writes started, and how many ended before their bytes were
    /// in place, as one cut short does: dropped or failed.
    pub started_writes: tokio::sync::watch::Sender<usize>,
    pub cut_writes: tokio::sync::watch::Sender<usize>,
    /// What each watch says, from the test.
    pub watch_says: Arc<Mutex<Option<mpsc::UnboundedReceiver<FileWatchMessage>>>>,
    /// What the runner told the backend of its streams, in order: each
    /// one's conversation, and whether it opened or closed.
    pub activity: tokio::sync::watch::Sender<Vec<(String, bool)>>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            held: Arc::default(),
            started_writes: tokio::sync::watch::Sender::new(0),
            cut_writes: tokio::sync::watch::Sender::new(0),
            watch_says: Arc::default(),
            activity: tokio::sync::watch::Sender::new(Vec::new()),
        }
    }
}

fn version(number: u32) -> String {
    format!("W/\"{number}\"")
}

fn missing() -> ChannelError {
    ChannelError::new(ChannelErrorCode::FsError, 404, "No such file")
}

impl Fake {
    pub fn put(&self, path: &str, bytes: &[u8]) {
        let mut held = self.held.lock().unwrap();
        let number = held.files.get(path).map_or(1, |(_, number)| number + 1);
        held.files.insert(path.into(), (bytes.to_vec(), number));
    }

    pub fn file(&self, path: &str) -> Option<Vec<u8>> {
        self.held.lock().unwrap().files.get(path).map(|(bytes, _)| bytes.clone())
    }

    fn saw(&self, scope: Scope) {
        self.held.lock().unwrap().scopes.push(scope);
    }
}

impl Operations for Fake {
    fn read(&self, scope: Scope, path: String, offset: u64, length: Option<u64>) -> Answer<FileRange> {
        self.saw(scope);
        let file = self.held.lock().unwrap().files.get(&path).cloned();
        Box::pin(async move {
            let (bytes, number) = file.ok_or_else(missing)?;
            let start = (offset as usize).min(bytes.len());
            let end = length.map_or(bytes.len(), |length| (start + length as usize).min(bytes.len()));
            let part = Bytes::copy_from_slice(&bytes[start..end]);
            // In chunks, as a file is read.
            let chunks: Vec<io::Result<Bytes>> = part
                .chunks(100 * 1024)
                .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
                .collect();
            Ok(FileRange {
                size: bytes.len() as u64,
                version: version(number),
                modified: demi_runner_protocol::wire::Timestamp(1_790_000_000_123),
                body: Box::pin(futures_util::stream::iter(chunks)),
            })
        })
    }

    fn write(&self, scope: Scope, path: String, replace: bool, mut body: ByteStream) -> Answer<()> {
        self.saw(scope);
        let fake = self.clone();
        Box::pin(async move {
            fake.started_writes.send_modify(|started| *started += 1);
            // Counts the write as cut unless it puts its file in place.
            let cut = fake.cut_writes.clone();
            let guard = scopeguard(move || cut.send_modify(|cut| *cut += 1));
            if !replace && fake.file(&path).is_some() {
                return Err(ChannelError::new(ChannelErrorCode::FileExists, 409, "taken"));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = body.next().await {
                let chunk = chunk.map_err(|error| {
                    ChannelError::new(ChannelErrorCode::HostOperationFailed, 500, error.to_string())
                })?;
                bytes.extend_from_slice(&chunk);
            }
            // In place whole, or not at all.
            fake.put(&path, &bytes);
            guard.defuse();
            Ok(())
        })
    }

    fn text(&self, scope: Scope, path: String, held: Option<String>) -> Answer<FileText> {
        self.saw(scope);
        let file = self.held.lock().unwrap().files.get(&path).cloned();
        Box::pin(async move {
            let (bytes, number) = file.ok_or_else(missing)?;
            let current = version(number);
            let text = (held.as_deref() != Some(current.as_str()))
                .then(|| String::from_utf8(bytes).unwrap());
            Ok(FileText {
                version: current,
                text,
            })
        })
    }

    fn list(&self, scope: Scope, path: Option<String>) -> Answer<Listing> {
        let listed = path.unwrap_or_else(|| scope.cwd.clone());
        self.saw(scope);
        let held = self.held.lock().unwrap();
        let prefix = format!("{listed}/");
        let entries = held
            .files
            .iter()
            .filter_map(|(path, (bytes, _))| {
                let name = path.strip_prefix(&prefix)?;
                Some(DirectoryEntry {
                    name: name.into(),
                    is_directory: false,
                    is_symbolic_link: false,
                    size: bytes.len() as u64,
                    modified_at: demi_shared_types::Timestamp::from_millisecond(0).unwrap(),
                })
            })
            .collect();
        Box::pin(async move {
            Ok(Listing {
                path: listed,
                home: Some("/home/ana".into()),
                entries,
            })
        })
    }

    fn mkdir(&self, scope: Scope, _path: String) -> Answer<()> {
        self.saw(scope);
        Box::pin(async { Ok(()) })
    }

    fn delete(&self, scope: Scope, path: String) -> Answer<()> {
        self.saw(scope);
        self.held.lock().unwrap().files.remove(&path);
        Box::pin(async { Ok(()) })
    }

    fn watch(&self, scope: Scope, paths: mpsc::UnboundedReceiver<Vec<String>>) -> WatchStream {
        self.saw(scope);
        let says = self.watch_says.lock().unwrap().take();
        let held = self.held.clone();
        // A `paths` message is recorded and answered with `changed` for
        // its paths.
        let named = futures_util::stream::unfold(paths, move |mut paths| {
            let held = held.clone();
            async move {
                let named = paths.recv().await?;
                held.lock().unwrap().watched.push(named.clone());
                Some((FileWatchMessage::Changed { paths: named }, paths))
            }
        });
        let said = futures_util::stream::unfold(says, |says| async move {
            let mut says = says?;
            let message = says.recv().await?;
            Some((message, Some(says)))
        });
        // A watch the test does not speak for says nothing more.
        let said = said.chain(futures_util::stream::pending());
        Box::pin(futures_util::stream::select(said, named))
    }

    fn stream(&self, scope: Scope, request: StreamRequest) -> Answer<ByteStream> {
        self.saw(scope);
        self.held
            .lock()
            .unwrap()
            .streams
            .push((request.binding.operation.clone(), request.stream.clone()));
        // Echoes the page's bytes, as a service stream's invocation answers
        // its input.
        let echo = request.input.map(|chunk| chunk.map(|bytes| {
            let mut echoed = b"echo:".to_vec();
            echoed.extend_from_slice(&bytes);
            Bytes::from(echoed)
        }));
        Box::pin(async move { Ok(Box::pin(echo) as ByteStream) })
    }

    fn stream_activity(&self, conversation: &str) -> BoxFuture<'static, StreamActivity> {
        let told = |open: bool| {
            let activity = self.activity.clone();
            let conversation = conversation.to_owned();
            move || activity.send_modify(|told| told.push((conversation, open)))
        };
        told(true)();
        let stream = {
            let mut held = self.held.lock().unwrap();
            held.given += 1;
            format!("s{}", held.given)
        };
        let activity = StreamActivity {
            stream,
            held: Box::new(scopeguard(told(false))),
        };
        Box::pin(std::future::ready(activity))
    }
}

/// Runs its action when dropped, unless defused.
struct Guard<F: FnOnce()>(Option<F>);

fn scopeguard<F: FnOnce()>(action: F) -> Guard<F> {
    Guard(Some(action))
}

impl<F: FnOnce()> Guard<F> {
    fn defuse(mut self) {
        self.0 = None;
    }
}

impl<F: FnOnce()> Drop for Guard<F> {
    fn drop(&mut self) {
        if let Some(action) = self.0.take() {
            action();
        }
    }
}
