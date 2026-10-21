//! A TCP forwarder between a runner and a fixture's backend end, which a test
//! can stall: while it is stalled, no byte passes in either direction and both
//! sockets stay open, as on a network that stops passing data without telling
//! either end.

use std::{io, net::SocketAddr};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, tcp::OwnedReadHalf, tcp::OwnedWriteHalf},
    sync::watch,
    task::JoinHandle,
};

/// The bytes one read takes from a socket.
const CHUNK: usize = 16 * 1024;

/// A forwarder to `upstream` that a test stalls and resumes.
pub struct Network {
    address: SocketAddr,
    stalled: watch::Sender<bool>,
    accepting: JoinHandle<()>,
}

impl Network {
    /// Listens on a local port and forwards each connection to `upstream`.
    pub async fn start(upstream: SocketAddr) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (stalled, watching) = watch::channel(false);
        let accepting = tokio::spawn(async move {
            while let Ok((near, _)) = listener.accept().await {
                let Ok(far) = TcpStream::connect(upstream).await else {
                    continue;
                };
                // Small frames leave at once on both legs, as on the edge.
                let _ = near.set_nodelay(true);
                let _ = far.set_nodelay(true);
                let (near_read, near_write) = near.into_split();
                let (far_read, far_write) = far.into_split();
                tokio::spawn(forward(near_read, far_write, watching.clone()));
                tokio::spawn(forward(far_read, near_write, watching.clone()));
            }
        });
        Ok(Self {
            address,
            stalled,
            accepting,
        })
    }

    /// The address a runner connects to.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Stops passing bytes in both directions, keeping the sockets open.
    pub fn stall(&self) {
        self.stalled.send_replace(true);
    }
}

impl Drop for Network {
    fn drop(&mut self) {
        self.accepting.abort();
    }
}

/// Copies `from` to `to` while the network passes data; a stalled one reads
/// and writes nothing.
async fn forward(mut from: OwnedReadHalf, mut to: OwnedWriteHalf, stalled: watch::Receiver<bool>) {
    let mut stalled = stalled;
    let mut buffer = vec![0; CHUNK];
    loop {
        if stalled.wait_for(|stalled| !stalled).await.is_err() {
            return;
        }
        let Ok(read) = from.read(&mut buffer).await else {
            return;
        };
        if read == 0 {
            let _ = to.shutdown().await;
            return;
        }
        if stalled.wait_for(|stalled| !stalled).await.is_err() {
            return;
        }
        if to.write_all(&buffer[..read]).await.is_err() {
            return;
        }
    }
}
