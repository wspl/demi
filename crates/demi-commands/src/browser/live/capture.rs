//! The loopback socket the capture extension connects to
//! (`browser-live-view.md` § Capture): the module starts and steers tab
//! captures over it, and receives their encoded frames. It binds before Chrome
//! starts, since the extension learns its address from its own files, and
//! accepts only this environment's token.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use bytes::{Buf, Bytes};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::mpsc};
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{ErrorResponse, Request, Response},
    http::StatusCode,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::super::{BrowserError, Result};

/// A frame's header from the extension: capture (u32), sequence (u32), flags
/// (u8, 1 = key frame), three reserved bytes, timestamp in microseconds
/// (f64), width and height (u16 each), eight reserved bytes.
const FRAME_HEADER: usize = 32;
/// Frames a capture's consumer has not taken yet; beyond them the consumer
/// is behind and the newest frames are dropped until it asks for a key frame.
const FRAME_QUEUE: usize = 8;

/// Why this Host cannot capture, or none (`browser-live-view.md` §
/// Capture). On Linux arm64, Chrome's SME code faults on a CPU that reports
/// SME without SVE, until the GPU process gives up and Chrome exits.
pub(crate) fn unavailable() -> Option<&'static str> {
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        // SAFETY: getauxval only reads the process's auxiliary vector.
        let (hwcap, hwcap2) = unsafe {
            (
                libc::getauxval(libc::AT_HWCAP),
                libc::getauxval(libc::AT_HWCAP2),
            )
        };
        if sme_without_sve(hwcap, hwcap2) {
            return Some(
                "this Host's CPU reports SME without SVE, which Chrome's tab capture cannot run on; boot its kernel with arm64.nosme to show the browser",
            );
        }
    }
    None
}

/// The kernel's `HWCAP_SVE` and `HWCAP2_SME` bits (`asm/hwcap.h`).
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "aarch64")),
    allow(dead_code)
)]
fn sme_without_sve(hwcap: u64, hwcap2: u64) -> bool {
    const HWCAP_SVE: u64 = 1 << 22;
    const HWCAP2_SME: u64 = 1 << 23;
    hwcap2 & HWCAP2_SME != 0 && hwcap & HWCAP_SVE == 0
}

/// One encoded picture of a capture.
#[derive(Debug, Clone)]
pub(crate) struct Frame {
    pub sequence: u32,
    pub key: bool,
    pub timestamp: f64,
    pub width: u16,
    pub height: u16,
    pub data: Bytes,
}

#[derive(Debug)]
pub(crate) enum CaptureEvent {
    Started,
    Frame(Frame),
    /// No picture arrived: the page stopped painting before capture began.
    Stalled,
    Failed(String),
}

/// The socket, bound before Chrome starts.
pub(crate) struct CaptureServer {
    listener: TcpListener,
    token: String,
}

impl CaptureServer {
    pub async fn bind() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let mut token = [0_u8; 24];
        getrandom::fill(&mut token)
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        Ok(Self {
            listener,
            token: token.iter().map(|byte| format!("{byte:02x}")).collect(),
        })
    }

    /// The address the extension dials.
    pub fn address(&self) -> Result<String> {
        Ok(format!(
            "ws://127.0.0.1:{}/?token={}",
            self.listener.local_addr()?.port(),
            self.token
        ))
    }

    /// Accepts the extension's connections until `ended`, each replacing the
    /// one before, and routes them into `captures`.
    pub fn serve(self, captures: Arc<Captures>, tasks: &TaskTracker, ended: CancellationToken) {
        let connections = tasks.clone();
        tasks.spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = ended.cancelled() => break,
                    accepted = self.listener.accept() => accepted,
                };
                let Ok((stream, _)) = accepted else { continue };
                let token = self.token.clone();
                let captures = captures.clone();
                let ended = ended.clone();
                connections.spawn(async move {
                    let expected = format!("token={token}");
                    // Tungstenite's callback answers a refusal with a whole response.
                    #[allow(clippy::result_large_err)]
                    let authorized = |request: &Request, response: Response| {
                        if request.uri().query() == Some(expected.as_str()) {
                            Ok(response)
                        } else {
                            let mut refusal = ErrorResponse::new(None);
                            *refusal.status_mut() = StatusCode::FORBIDDEN;
                            Err(refusal)
                        }
                    };
                    let Ok(socket) = tokio_tungstenite::accept_hdr_async(stream, authorized).await
                    else {
                        return;
                    };
                    captures.connect(socket, ended).await;
                });
            }
        });
    }
}

struct Connection {
    commands: mpsc::UnboundedSender<Value>,
    generation: u64,
}

/// The extension's connection and the captures it runs.
#[derive(Default)]
pub(crate) struct Captures {
    connection: Mutex<Option<Connection>>,
    connected: tokio::sync::Notify,
    captures: Mutex<HashMap<u32, mpsc::Sender<CaptureEvent>>>,
    next: AtomicU32,
    generations: std::sync::atomic::AtomicU64,
}

impl Captures {
    async fn connect(
        &self,
        socket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        ended: CancellationToken,
    ) {
        let (mut sink, mut stream) = socket.split();
        let (commands, mut outgoing) = mpsc::unbounded_channel::<Value>();
        let generation = self.generations.fetch_add(1, Ordering::Relaxed);
        *self.connection.lock().expect("capture lock poisoned") = Some(Connection {
            commands,
            generation,
        });
        self.connected.notify_waiters();
        let writer = async {
            while let Some(command) = outgoing.recv().await {
                if sink.send(Message::text(command.to_string())).await.is_err() {
                    break;
                }
            }
        };
        let reader = async {
            while let Some(Ok(message)) = stream.next().await {
                match message {
                    Message::Binary(bytes) => self.frame(bytes),
                    Message::Text(text) => {
                        if let Ok(event) = serde_json::from_str::<Value>(&text) {
                            self.event(&event);
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        };
        tokio::select! {
            _ = ended.cancelled() => {}
            _ = writer => {}
            _ = reader => {}
        }
        let mut connection = self.connection.lock().expect("capture lock poisoned");
        if connection
            .as_ref()
            .is_some_and(|current| current.generation == generation)
        {
            *connection = None;
        }
        drop(connection);
        // The extension lost its captures with its socket.
        let captures: Vec<_> = self
            .captures
            .lock()
            .expect("capture lock poisoned")
            .drain()
            .collect();
        for (_, events) in captures {
            let _gone = events.try_send(CaptureEvent::Failed(
                "capture extension disconnected".into(),
            ));
        }
    }

    fn frame(&self, mut bytes: Bytes) {
        if bytes.len() < FRAME_HEADER {
            return;
        }
        let mut header = bytes.split_to(FRAME_HEADER);
        let capture = header.get_u32();
        let sequence = header.get_u32();
        let key = header.get_u8() & 1 == 1;
        header.advance(3);
        let timestamp = header.get_f64();
        let width = header.get_u16();
        let height = header.get_u16();
        let events = self
            .captures
            .lock()
            .expect("capture lock poisoned")
            .get(&capture)
            .cloned();
        if let Some(events) = events {
            // A consumer that is behind loses the newest pictures, not the socket.
            let _behind = events.try_send(CaptureEvent::Frame(Frame {
                sequence,
                key,
                timestamp,
                width,
                height,
                data: bytes,
            }));
        }
    }

    fn event(&self, event: &Value) {
        let Some(capture) = event["capture"]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
        else {
            return;
        };
        let events = self
            .captures
            .lock()
            .expect("capture lock poisoned")
            .get(&capture)
            .cloned();
        let Some(events) = events else { return };
        let delivered = match event["type"].as_str() {
            Some("started") => CaptureEvent::Started,
            Some("stalled") => CaptureEvent::Stalled,
            Some("error") => CaptureEvent::Failed(
                event["message"]
                    .as_str()
                    .unwrap_or("capture failed")
                    .to_owned(),
            ),
            _ => return,
        };
        let _behind = events.try_send(delivered);
    }

    fn command(&self, command: Value) -> Result<()> {
        self.connection
            .lock()
            .expect("capture lock poisoned")
            .as_ref()
            .and_then(|connection| connection.commands.send(command).ok())
            .ok_or_else(|| {
                BrowserError::Unavailable("the capture extension is not connected".into())
            })
    }

    /// Waits until the extension connects, which it does as Chrome starts.
    async fn connection(&self, cancel: &CancellationToken) -> Result<()> {
        loop {
            let notified = self.connected.notified();
            if self
                .connection
                .lock()
                .expect("capture lock poisoned")
                .is_some()
            {
                return Ok(());
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err(BrowserError::Cancelled),
                _ = notified => {}
            }
        }
    }

    /// Captures the tab of CDP `target` at `width` × `height` pixels.
    pub async fn start(
        self: &Arc<Self>,
        target: &str,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        cancel: &CancellationToken,
    ) -> Result<Capture> {
        if let Some(reason) = unavailable() {
            return Err(BrowserError::UnsupportedCapability(reason.into()));
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), self.connection(cancel))
            .await
            .map_err(|_| {
                BrowserError::Unavailable("the capture extension did not connect".into())
            })??;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (sender, events) = mpsc::channel(FRAME_QUEUE);
        self.captures
            .lock()
            .expect("capture lock poisoned")
            .insert(id, sender);
        let capture = Capture {
            id,
            captures: self.clone(),
            events,
        };
        self.command(json!({
            "type": "start", "capture": id, "target": target,
            "width": width, "height": height, "fps": fps, "bitrate": bitrate,
        }))?;
        Ok(capture)
    }
}

/// One tab's capture; dropping it stops the capture.
pub(crate) struct Capture {
    id: u32,
    captures: Arc<Captures>,
    pub events: mpsc::Receiver<CaptureEvent>,
}

impl Capture {
    /// The consumer has these frames; up to `window` may be in flight.
    pub fn ack(&self, sequence: u32, window: u32) {
        let _gone = self.captures.command(
            json!({"type": "ack", "capture": self.id, "sequence": sequence, "window": window}),
        );
    }

    pub fn key_frame(&self) {
        let _gone = self
            .captures
            .command(json!({"type": "keyframe", "capture": self.id}));
    }

    pub fn encoding(&self, bitrate: u32, fps: u32) {
        let _gone = self.captures.command(
            json!({"type": "encoding", "capture": self.id, "bitrate": bitrate, "fps": fps}),
        );
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.captures
            .captures
            .lock()
            .expect("capture lock poisoned")
            .remove(&self.id);
        let _gone = self
            .captures
            .command(json!({"type": "stop", "capture": self.id}));
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use demi_command_service::protocol::CommandLocale;
    use tokio_util::sync::CancellationToken;

    use super::{super::super::environment::LaunchOptions, *};

    #[test]
    fn a_cpu_with_sme_but_no_sve_cannot_capture() {
        // AT_HWCAP2 of an Apple M4 guest, then of the same guest booted with
        // arm64.nosme; neither has SVE in AT_HWCAP.
        let hwcap = 0xefff_ffff & !(1 << 22);
        assert!(sme_without_sve(hwcap, 0x1a0_3fb2_6181));
        assert!(!sme_without_sve(hwcap, 0x32_6181));
        // A CPU with both, such as a server with SVE and SME.
        assert!(!sme_without_sve(hwcap | 1 << 22, 0x1a0_3fb2_6181));
    }

    #[tokio::test]
    #[ignore = "requires DEMI_TEST_CHROME pointing to an installed Chrome for Testing release"]
    async fn a_watched_tab_arrives_as_h264_starting_with_a_key_frame() {
        let executable =
            PathBuf::from(std::env::var_os("DEMI_TEST_CHROME").expect("DEMI_TEST_CHROME"));
        let locale = CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        };
        let timeout = Duration::from_secs(60);
        super::super::super::with_browser(
            LaunchOptions::pinned(executable, locale).unwrap(),
            CancellationToken::new(),
            |environment| async move {
                let page = "data:text/html,<body style='margin:0'><div id=b style='width:100px;height:100px;background:red'></div><script>let x=0;setInterval(()=>{x=x>600?0:x+7;b.style.marginLeft=x+'px'},16)</script>";
                let tab = environment
                    .open("about:blank", &CancellationToken::new(), timeout)
                    .await?;
                tab.page.goto(page).await?;
                let mut capture = environment
                    .live
                    .captures
                    .start(tab.target_id(), 1280, 720, 30, 4_000_000, &CancellationToken::new())
                    .await?;
                let mut started = false;
                let first = tokio::time::timeout(Duration::from_secs(20), async {
                    loop {
                        match capture.events.recv().await {
                            Some(CaptureEvent::Started) => started = true,
                            Some(CaptureEvent::Frame(frame)) => return frame,
                            Some(CaptureEvent::Stalled) => {}
                            Some(CaptureEvent::Failed(message)) => panic!("{message}"),
                            None => panic!("capture ended"),
                        }
                    }
                })
                .await
                .expect("no frame within 20 seconds");
                assert!(started);
                assert!(first.key, "the first frame is a key frame");
                assert_eq!((first.width, first.height), (1280, 720));
                assert_eq!(&first.data[..4], [0, 0, 0, 1], "Annex B start code");
                // A moving page keeps sending frames while the window allows.
                capture.ack(first.sequence, 8);
                let next = tokio::time::timeout(Duration::from_secs(5), capture.events.recv())
                    .await
                    .expect("no second frame");
                assert!(matches!(next, Some(CaptureEvent::Frame(frame)) if frame.sequence == first.sequence + 1));
                Ok(())
            },
        )
        .await
        .unwrap();
    }
}
