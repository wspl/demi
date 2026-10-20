//! Downloading ahead (`crates/runner/src/update.rs`): a backend that names the
//! runner release coming next has an installed runner download it while it
//! serves, and the download resumes from the bytes it has when its connection
//! breaks off or stalls.
//!
//! The backend here is scripted: it serves the executable's compressed copy
//! over raw HTTP on the runner's own backend address, cutting or stalling the
//! first answer halfway.

use std::collections::BTreeMap;

use demi_backend_remote_host::testing::RunnerProcessOptions;
use demi_command_protocol::host_target;
use demi_host_interface::SpawnEnv;
use demi_runner_protocol::wire::{Inbound, Outbound};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpStream;

use crate::{CONNECT, Host, TOKEN};

/// The release the runner was installed from, and the one coming next.
const INSTALLED: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const NEXT: &str = "2222222222222222222222222222222222222222222222222222222222222222";

/// The next release's executable, bytes zstd cannot shrink, so the copy
/// it is served as is large enough to break off halfway, and that copy.
struct Executable {
    bytes: Vec<u8>,
    compressed: Vec<u8>,
}

impl Executable {
    fn new() -> Self {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let bytes: Vec<u8> = (0..1 << 20)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        let compressed =
            demi_shared_artifacts::encode_blocking(&bytes, demi_shared_artifacts::Effort::Fast)
                .unwrap();
        Self { bytes, compressed }
    }

    /// What the backend asks the runner to download.
    fn prefetch(&self) -> Inbound {
        Inbound::Prefetch {
            release: NEXT.into(),
            sha256: format!("{:x}", Sha256::digest(&self.bytes)),
            size: self.bytes.len() as u64,
        }
    }
}

/// An installed runner of [`INSTALLED`], online, whose downloads give a
/// connection up after `stall_ms` without a byte.
async fn installed(stall_ms: u64) -> Host {
    let env = BTreeMap::from([("DEMI_DOWNLOAD_STALL_MS".to_owned(), Some(stall_ms.to_string()))]);
    Host::start_with(RunnerProcessOptions {
        env: SpawnEnv::Overlay(env),
        release: Some(INSTALLED.into()),
        ..RunnerProcessOptions::default()
    })
    .await
    .online()
    .await
}

/// The runner's next request to the backend, which must be a download of
/// the next release's executable carrying the device's token: its
/// connection, and the bytes it asks to start from.
async fn download(host: &Host) -> (TcpStream, u64) {
    let accepted = tokio::time::timeout(CONNECT, host.listener.accept()).await;
    let Ok(Ok((stream, _))) = accepted else {
        panic!("the runner asked for no download:\n{}", host.process.output());
    };
    let mut stream = BufReader::new(stream);
    let mut head = Vec::new();
    loop {
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
        head.push(line.trim_end().to_owned());
    }
    let path = format!(
        "GET /runner-artifacts/{NEXT}/{}/demi-runner.zst HTTP/1.1",
        host_target()
    );
    assert_eq!(head[0], path, "{head:?}");
    let header = |name: &str| {
        head.iter().find_map(|line| {
            let (header, value) = line.split_once(':')?;
            header.eq_ignore_ascii_case(name).then(|| value.trim().to_owned())
        })
    };
    assert_eq!(header("demi-runner-token").as_deref(), Some(TOKEN), "{head:?}");
    let from = header("range").map_or(0, |range| {
        let from = range.strip_prefix("bytes=").and_then(|range| range.strip_suffix('-'));
        from.expect("a range from a byte on").parse().unwrap()
    });
    (stream.into_inner(), from)
}

/// Answers a download from byte `from` with the copy's bytes from there to
/// `to`, as a server whose answer is the copy's from `from` to its end.
async fn answer(stream: &mut TcpStream, compressed: &[u8], from: u64, to: usize) {
    let size = compressed.len();
    let head = if from == 0 {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nContent-Encoding: zstd\r\n\r\n")
    } else {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{size}\r\nContent-Encoding: zstd\r\n\r\n",
            size as u64 - from,
            size - 1
        )
    };
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(&compressed[from as usize..to]).await.unwrap();
    stream.flush().await.unwrap();
}

/// The runner's answer to the prefetch, once its download ended.
async fn answered(host: &mut Host) -> Outbound {
    loop {
        match host.frame().await {
            frame @ (Outbound::Prefetched { .. } | Outbound::PrefetchFailed { .. }) => return frame,
            _ => {}
        }
    }
}

/// Asserts that the next release is in place in the installation, and
/// its download's bytes are gone.
fn in_place(host: &Host, executable: &Executable) {
    let releases = host.state().join("releases");
    assert_eq!(std::fs::read(releases.join(NEXT).join("demi-runner")).unwrap(), executable.bytes);
    assert!(!releases.join(format!(".download-{NEXT}")).exists());
}

/// A download that breaks off halfway continues with a Range request from
/// the bytes the runner has, instead of starting again, and the runner
/// stays connected throughout.
#[tokio::test]
async fn a_broken_download_resumes_from_its_bytes() {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let mut host = installed(30_000).await;
        let executable = Executable::new();
        let half = executable.compressed.len() / 2;
        host.send(executable.prefetch()).await;
        let (mut first, from) = download(&host).await;
        assert_eq!(from, 0);
        answer(&mut first, &executable.compressed, 0, half).await;
        // The connection breaks off.
        drop(first);
        let (mut second, from) = download(&host).await;
        assert_eq!(from, half as u64);
        answer(&mut second, &executable.compressed, from, executable.compressed.len()).await;
        assert_eq!(answered(&mut host).await, Outbound::Prefetched { release: NEXT.into() });
        in_place(&host, &executable);
        host.close().await;
    })
    .await
    .expect("the test finishes");
}

/// A download that receives nothing for the read timeout gives its
/// connection up and resumes on a new one at once.
#[tokio::test]
async fn a_stalled_download_resumes_on_a_new_connection() {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        // A test's stall, in place of 30 seconds.
        let mut host = installed(300).await;
        let executable = Executable::new();
        let half = executable.compressed.len() / 2;
        host.send(executable.prefetch()).await;
        let (mut stalled, from) = download(&host).await;
        assert_eq!(from, 0);
        // Half the bytes, then nothing, the connection held open.
        answer(&mut stalled, &executable.compressed, 0, half).await;
        let (mut second, from) = download(&host).await;
        assert_eq!(from, half as u64);
        answer(&mut second, &executable.compressed, from, executable.compressed.len()).await;
        assert_eq!(answered(&mut host).await, Outbound::Prefetched { release: NEXT.into() });
        in_place(&host, &executable);
        drop(stalled);
        host.close().await;
    })
    .await
    .expect("the test finishes");
}
