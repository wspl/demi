//! What an upgrade prints while the new backend starts: how far its update
//! of the data is, as `GET /api/update` says, which the backend answers
//! before it serves (`crates/backend-http/src/update.rs`).

use std::io::{Read as _, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use demi_web_api_protocol::state::{UpdateState, UpdateStep};

/// How often the progress is asked.
const EVERY: Duration = Duration::from_secs(5);

/// Asks how far the update is until it is stopped.
pub struct Watch {
    stop: Arc<AtomicBool>,
    asking: JoinHandle<()>,
}

impl Watch {
    /// Prints to standard output, each time it changes, how far the backend
    /// listening at `address` is, until [`Watch::stop`].
    pub fn start(address: SocketAddr) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let asking = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut last = String::new();
                while !stop.load(Ordering::Relaxed) {
                    if let Some(line) = ask(reachable(address)).and_then(|state| said(&state)) {
                        if line != last {
                            println!("{line}");
                            let _ = std::io::stdout().flush();
                            last = line;
                        }
                    }
                    std::thread::park_timeout(EVERY);
                }
            })
        };
        Self { stop, asking }
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        self.asking.thread().unpark();
        let _ = self.asking.join();
    }
}

/// A listener on every address is reached on the loopback one.
fn reachable(address: SocketAddr) -> SocketAddr {
    let ip = match address.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, address.port())
}

/// The update's state, or none while the backend does not answer.
fn ask(address: SocketAddr) -> Option<UpdateState> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    stream
        .write_all(b"GET /api/update HTTP/1.0\r\nHost: localhost\r\nAccept: application/json\r\n\r\n")
        .ok()?;
    let mut answer = Vec::new();
    stream.take(64 * 1024).read_to_end(&mut answer).ok()?;
    let body = answer.windows(4).position(|window| window == b"\r\n\r\n")? + 4;
    if !answer.starts_with(b"HTTP/1.0 200") && !answer.starts_with(b"HTTP/1.1 200") {
        return None;
    }
    serde_json::from_slice(&answer[body..]).ok()
}

/// The line for `state`; none when the backend is not updating.
fn said(state: &UpdateState) -> Option<String> {
    let UpdateState::Updating {
        step,
        conversations_done,
        conversations,
        bytes_done,
        bytes,
        left_seconds,
    } = state
    else {
        return None;
    };
    let what = match step {
        UpdateStep::Control => "Updating accounts and settings".to_owned(),
        UpdateStep::Conversations => format!("Updating conversations: {conversations_done} of {conversations}"),
    };
    let size = format!("{} of {}", size(*bytes_done), size(*bytes));
    let left = match left_seconds {
        None => String::new(),
        Some(seconds) if *seconds < 60 => ", less than a minute left".to_owned(),
        Some(seconds) => format!(", about {} minutes left", (seconds + 30) / 60),
    };
    Some(format!("{what} ({size}){left}"))
}

/// `1.2 GB`, `840 MB`, as the update screen writes them.
fn size(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in ["KB", "MB", "GB", "TB"] {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next;
    }
    if unit == "B" || value >= 10.0 { format!("{value:.0} {unit}") } else { format!("{value:.1} {unit}") }
}
