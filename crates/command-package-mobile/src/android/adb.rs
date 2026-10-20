//! The adb server's own protocol
//! (`crates/command-package-mobile/src/android/mod.rs`), which the `adb`
//! program speaks to it on its port: a request is its length in four hex
//! digits and its text, answered `OKAY` or `FAIL` with a reason. A device's
//! service, a shell command, a port inside it, or the file sync service,
//! is a request after `host:transport:<serial>` on the same connection,
//! which then carries that service's bytes.

use std::path::Path;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::driver::{DriverError, Result};

/// The largest piece of a file the sync service takes at once.
const SYNC_CHUNK: usize = 64 * 1024;

/// The adb server at its port on this computer.
#[derive(Debug, Clone, Copy)]
pub struct Adb {
    pub port: u16,
}

fn failed(what: &str, error: impl std::fmt::Display) -> DriverError {
    DriverError::Failed(format!("adb {what}: {error}"))
}

impl Adb {
    /// The server `ANDROID_ADB_SERVER_PORT` names, or the default one.
    pub fn from_environment() -> Self {
        let port = std::env::var("ANDROID_ADB_SERVER_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(5037);
        Self { port }
    }

    /// Whether the server answers.
    pub async fn running(&self) -> bool {
        TcpStream::connect(("127.0.0.1", self.port)).await.is_ok()
    }

    async fn connect(&self) -> Result<TcpStream> {
        TcpStream::connect(("127.0.0.1", self.port))
            .await
            .map_err(|error| failed("server", format!("not reachable on port {}: {error}", self.port)))
    }

    /// Sends `request` and reads its answer.
    async fn request(stream: &mut TcpStream, request: &str) -> Result<()> {
        let framed = format!("{:04x}{request}", request.len());
        stream
            .write_all(framed.as_bytes())
            .await
            .map_err(|error| failed(request, error))?;
        let mut status = [0u8; 4];
        stream
            .read_exact(&mut status)
            .await
            .map_err(|error| failed(request, error))?;
        match &status {
            b"OKAY" => Ok(()),
            b"FAIL" => {
                let mut length = [0u8; 4];
                stream.read_exact(&mut length).await.map_err(|error| failed(request, error))?;
                let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap_or("0"), 16).unwrap_or(0);
                let mut reason = vec![0u8; length];
                stream.read_exact(&mut reason).await.map_err(|error| failed(request, error))?;
                Err(failed(request, String::from_utf8_lossy(&reason)))
            }
            other => Err(failed(request, format!("answered {:?}", String::from_utf8_lossy(other)))),
        }
    }

    /// Waits until the emulator `serial` is online in adb.
    pub async fn wait_for(&self, serial: &str) -> Result<()> {
        let mut stream = self.connect().await?;
        // Answered OKAY once when the request is taken and again when the
        // device is there.
        Self::request(&mut stream, &format!("host-serial:{serial}:wait-for-any-device")).await?;
        let mut status = [0u8; 4];
        stream
            .read_exact(&mut status)
            .await
            .map_err(|error| failed("wait-for-device", error))?;
        Ok(())
    }

    /// A connection to `service` on device `serial`.
    pub async fn open(&self, serial: &str, service: &str) -> Result<TcpStream> {
        let mut stream = self.connect().await?;
        Self::request(&mut stream, &format!("host:transport:{serial}")).await?;
        Self::request(&mut stream, service).await?;
        Ok(stream)
    }

    /// Runs `command` in the device's shell and answers what it printed.
    pub async fn shell(&self, serial: &str, command: &str) -> Result<String> {
        let mut stream = self.open(serial, &format!("shell:{command}")).await?;
        let mut printed = Vec::new();
        stream
            .read_to_end(&mut printed)
            .await
            .map_err(|error| failed("shell", error))?;
        Ok(String::from_utf8_lossy(&printed).into_owned())
    }

    /// Writes `bytes` to the device's `remote` file with `mode`.
    pub async fn push(&self, serial: &str, bytes: &[u8], remote: &str, mode: u32) -> Result<()> {
        let mut stream = self.open(serial, "sync:").await?;
        let failure = |error: std::io::Error| failed("push", error);
        let target = format!("{remote},{mode}");
        stream.write_all(&sync_header(b"SEND", target.len())).await.map_err(failure)?;
        stream.write_all(target.as_bytes()).await.map_err(failure)?;
        for chunk in bytes.chunks(SYNC_CHUNK) {
            stream.write_all(&sync_header(b"DATA", chunk.len())).await.map_err(failure)?;
            stream.write_all(chunk).await.map_err(failure)?;
        }
        let modified = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        stream
            .write_all(&sync_header(b"DONE", modified as usize))
            .await
            .map_err(failure)?;
        let (id, length) = sync_read_header(&mut stream).await.map_err(failure)?;
        match &id {
            b"OKAY" => Ok(()),
            _ => Err(failed("push", sync_reason(&mut stream, length).await)),
        }
    }

    /// Copies the device's `remote` file to `local`.
    pub async fn pull(&self, serial: &str, remote: &str, local: &Path) -> Result<()> {
        self.pull_at_most(serial, remote, local, u64::MAX).await
    }

    /// Copies the device's `remote` file to `local`, failing once it runs
    /// past `limit` bytes.
    pub async fn pull_at_most(&self, serial: &str, remote: &str, local: &Path, limit: u64) -> Result<()> {
        let mut stream = self.open(serial, "sync:").await?;
        let failure = |error: std::io::Error| failed("pull", error);
        stream.write_all(&sync_header(b"RECV", remote.len())).await.map_err(failure)?;
        stream.write_all(remote.as_bytes()).await.map_err(failure)?;
        let mut file = tokio::fs::File::create(local).await.map_err(failure)?;
        let mut buffer = vec![0u8; SYNC_CHUNK];
        let mut pulled: u64 = 0;
        loop {
            let (id, length) = sync_read_header(&mut stream).await.map_err(failure)?;
            match &id {
                b"DATA" => {
                    let piece = buffer.get_mut(..length).ok_or_else(|| failed("pull", "a piece larger than 64 KiB"))?;
                    pulled += length as u64;
                    if pulled > limit {
                        // The rest of the transfer goes with the connection.
                        return Err(failed("pull", format!("{remote} is larger than {limit} bytes")));
                    }
                    stream.read_exact(piece).await.map_err(failure)?;
                    file.write_all(piece).await.map_err(failure)?;
                }
                b"DONE" => break,
                _ => return Err(failed("pull", sync_reason(&mut stream, length).await)),
            }
        }
        file.flush().await.map_err(failure)
    }
}

fn sync_header(id: &[u8; 4], length: usize) -> [u8; 8] {
    let mut header = [0u8; 8];
    header[..4].copy_from_slice(id);
    header[4..].copy_from_slice(&(length as u32).to_le_bytes());
    header
}

async fn sync_read_header(stream: &mut TcpStream) -> std::io::Result<([u8; 4], usize)> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header).await?;
    let mut id = [0u8; 4];
    id.copy_from_slice(&header[..4]);
    let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    Ok((id, length))
}

/// The reason a sync request failed, which follows its `FAIL`.
async fn sync_reason(stream: &mut TcpStream, length: usize) -> String {
    let mut reason = vec![0u8; length.min(4096)];
    match stream.read_exact(&mut reason).await {
        Ok(_) => String::from_utf8_lossy(&reason).into_owned(),
        Err(error) => error.to_string(),
    }
}
