//! An emulator's screen as one stream of pictures, which the live view and a
//! recording both take.
//!
//! The emulator draws each picture once, and every reader of the screen shares
//! that picture; the stream is opened when the first reader comes and closed
//! when the last has left. It is asked for at the largest size a reader needs:
//! a live view alone costs what its panel shows, and a recording raises it to
//! the screen's own pixels, the live view scaling what it is given.
//!
//! ```text
//! emulator --streamScreenshot, pixels in a file both map--> Screen
//!                                                            |  one picture,
//!                                  +-------------------------+  shared
//!                                  v                         v
//!                         live view: scaled to          recording: every
//!                         the panel, newest only        picture that differs
//! ```
//!
//! # Rules
//!
//! - The pixels of a picture reach this program through a file the emulator
//!   writes, where the Host's files can be mapped, and in the message
//!   otherwise, or where the emulator does not support the file.
//! - Without a hardware encoder on the Host the stream is asked for at the
//!   screen's logical size, which a software encoder keeps up with, and the
//!   recording is that size.
//! - A reader that needs a picture the stream drew before it came is given the
//!   latest one at once.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use tokio_util::task::AbortOnDropHandle;

use super::{
    emulator::Client,
    proto::{ImageFormat, ImageTransport, image_format::ImgFormat, image_transport::TransportChannel},
};

/// One picture of the screen: RGBA, rows from the top, no padding.
pub struct Picture {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// What the stream tells its readers.
pub enum Event {
    /// A picture, with its time in microseconds on the stream's clock.
    Picture(Arc<Picture>, u64),
    /// The emulator drew this many pictures it could not give.
    Missed(u32),
    /// The stream ended, and why.
    Ended(String),
}

type Reader = Arc<dyn Fn(Event) + Send + Sync>;

#[derive(Default)]
struct State {
    /// Each reader with the long side of the picture it needs.
    readers: Vec<(u64, u32, Reader)>,
    /// The long side the open stream is asked for.
    asked: u32,
    latest: Option<(Arc<Picture>, u64)>,
    stream: Option<AbortOnDropHandle<()>>,
}

/// The screen of one emulator.
pub struct Screen {
    client: Client,
    /// The long side of the screen's own pixels, which no stream is asked
    /// beyond.
    native: u32,
    /// The long side a recording needs.
    recording: u32,
    state: Mutex<State>,
    next: AtomicU64,
}

impl Screen {
    /// The screen of the emulator `client` reaches, `native` pixels large and
    /// `logical` points; a recording is at the first where the Host has a
    /// hardware encoder, at the second where it has none.
    pub fn new(client: Client, native: (u32, u32), logical: (u32, u32), hardware: bool) -> Arc<Self> {
        let long = |size: (u32, u32)| size.0.max(size.1).max(2);
        Arc::new(Self {
            client,
            native: long(native),
            recording: if hardware { long(native) } else { long(logical) },
            state: Mutex::default(),
            next: AtomicU64::new(1),
        })
    }

    /// The long side a recording needs, to pass to [`Self::subscribe`].
    pub fn recording(&self) -> u32 {
        self.recording
    }

    /// Hands `reader` every picture from now on, the latest first, until the
    /// subscription is dropped; the pictures are at least `long` on their long
    /// side where the screen has that many pixels. It runs on the stream's
    /// task, so it only hands the picture on.
    pub fn subscribe(self: &Arc<Self>, long: u32, reader: impl Fn(Event) + Send + Sync + 'static) -> Subscription {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let reader: Reader = Arc::new(reader);
        let latest = {
            let mut state = self.state.lock().expect("the screen's state is intact");
            state.readers.push((id, long.clamp(2, self.native), reader.clone()));
            self.fit(&mut state);
            state.latest.clone()
        };
        if let Some((picture, at)) = latest {
            reader(Event::Picture(picture, at));
        }
        Subscription {
            screen: self.clone(),
            id,
        }
    }

    /// Opens, or opens again at another size, the stream the readers need.
    fn fit(self: &Arc<Self>, state: &mut State) {
        let needed = state.readers.iter().map(|(_, long, _)| *long).max();
        match needed {
            None => state.stream = None,
            Some(long) if state.stream.is_none() || state.asked != long => {
                state.asked = long;
                state.stream = Some(self.open(long));
            }
            Some(_) => {}
        }
    }

    fn open(self: &Arc<Self>, long: u32) -> AbortOnDropHandle<()> {
        let screen = self.clone();
        AbortOnDropHandle::new(tokio::spawn(async move {
            let ended = screen.stream(long).await;
            let note = match ended {
                Ok(()) => "the emulator stopped sending its screen".to_owned(),
                Err(reason) => format!("the emulator's screen: {reason}"),
            };
            // A reader that comes after this opens a stream of its own.
            let readers: Vec<Reader> = {
                let mut state = screen.state.lock().expect("the screen's state is intact");
                state.latest = None;
                if let Some(stream) = state.stream.take() {
                    // This task is the stream, and has nothing left to wait
                    // for.
                    std::mem::forget(stream);
                }
                state.readers.iter().map(|(_, _, reader)| reader.clone()).collect()
            };
            for reader in readers {
                reader(Event::Ended(note.clone()));
            }
        }))
    }

    fn tell(&self, event: impl Fn() -> Event) {
        let readers: Vec<Reader> =
            self.state.lock().expect("the screen's state is intact").readers.iter().map(|(_, _, reader)| reader.clone()).collect();
        for reader in readers {
            reader(event());
        }
    }

    async fn stream(&self, long: u32) -> Result<(), String> {
        let mut client = self.client.clone();
        let mapped = Mapped::create(long);
        let request = ImageFormat {
            format: ImgFormat::Rgba8888.into(),
            width: long,
            height: long,
            transport: mapped.as_ref().map(|mapped| ImageTransport {
                channel: TransportChannel::Mmap.into(),
                handle: mapped.handle(),
            }),
            ..ImageFormat::default()
        };
        let mut stream = client
            .stream_screenshot(request)
            .await
            .map_err(|status| status.message().to_owned())?
            .into_inner();
        let started = Instant::now();
        let mut last = None::<u32>;
        while let Some(image) = stream.message().await.map_err(|status| status.message().to_owned())? {
            let (width, height) = image.format.as_ref().map_or((0, 0), |format| (format.width as usize, format.height as usize));
            let bytes = width * height * 4;
            // An empty picture says the display is off.
            if width < 2 || height < 2 {
                continue;
            }
            let rgba = if image.image.len() >= bytes {
                let mut rgba = image.image.to_vec();
                rgba.truncate(bytes);
                rgba
            } else if let Some(rgba) = mapped.as_ref().and_then(|mapped| mapped.read(bytes)) {
                rgba
            } else {
                continue;
            };
            if let Some(last) = last
                && image.seq > last + 1
            {
                let missed = image.seq - last - 1;
                self.tell(|| Event::Missed(missed));
            }
            last = Some(image.seq);
            let at = if image.timestamp_us > 0 {
                image.timestamp_us
            } else {
                u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
            };
            let picture = Arc::new(Picture {
                rgba,
                width: width as u32,
                height: height as u32,
            });
            self.state.lock().expect("the screen's state is intact").latest = Some((picture.clone(), at));
            self.tell(|| Event::Picture(picture.clone(), at));
        }
        Ok(())
    }
}

/// A reader's place at the stream; dropping it leaves, and the stream closes
/// with the last.
pub struct Subscription {
    screen: Arc<Screen>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut state = self.screen.state.lock().expect("the screen's state is intact");
        state.readers.retain(|(id, _, _)| *id != self.id);
        if state.readers.is_empty() {
            state.latest = None;
        }
        self.screen.fit(&mut state);
    }
}

/// The file the emulator writes a picture's pixels into, on a Host where the
/// emulator and this program map the same files; removed as the stream ends.
#[cfg(unix)]
struct Mapped {
    path: std::path::PathBuf,
    file: std::fs::File,
}

#[cfg(unix)]
impl Mapped {
    fn create(long: u32) -> Option<Self> {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "demi-screen-{}-{}.rgba",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|file| file.set_len(u64::from(long) * u64::from(long) * 4).map(|()| file));
        match file {
            Ok(file) => Some(Self { path, file }),
            Err(error) => {
                tracing::warn!("the emulator's pictures come in its messages, for {}: {error}", path.display());
                None
            }
        }
    }

    fn handle(&self) -> String {
        format!("file://{}", self.path.display())
    }

    /// The first `bytes` of the file; none when it is shorter.
    fn read(&self, bytes: usize) -> Option<Vec<u8>> {
        use std::os::unix::fs::FileExt as _;
        let mut rgba = vec![0; bytes];
        self.file.read_exact_at(&mut rgba, 0).ok().map(|()| rgba)
    }
}

#[cfg(unix)]
impl Drop for Mapped {
    fn drop(&mut self) {
        let _gone = std::fs::remove_file(&self.path);
    }
}

/// Without files both map, the pictures come in the messages.
#[cfg(not(unix))]
struct Mapped;

#[cfg(not(unix))]
impl Mapped {
    fn create(_long: u32) -> Option<Self> {
        None
    }

    fn handle(&self) -> String {
        String::new()
    }

    fn read(&self, _bytes: usize) -> Option<Vec<u8>> {
        None
    }
}
