//! The bytes of file transfers (`sessions-and-targets.md` § Host
//! operations): the edge moves them while the user's shard holds the
//! transfer's admission through its lease. A download's body is taken as
//! the browser takes it and is cut short, never ended, when the Host's read
//! fails or the shard ends the transfer; a connection on which nothing moves
//! for a minute is closed. An upload's body is read as the Host writes it:
//! a browser that sends nothing for a minute stalls it, and a Host that
//! takes nothing for a minute closes its connection.

use std::io;
use std::time::Duration;

use axum::body::Body;
use axum::http::StatusCode;
use bytes::Bytes;
use demi_host_remote::PipeReader;
use demi_web_api::error::ErrorCode;
use futures_util::StreamExt as _;

use super::error::ApiError;
use super::listener::ConnectionWatch;
use crate::conversation::transfer::OpenUpload;
use crate::shard::lease::Lease;

/// How long a transfer waits for the browser, and a connection for any
/// byte, before it ends: the stalled-client timeout web servers use, nginx's
/// `send_timeout` among them.
pub(super) const TRANSFER_IDLE: Duration = Duration::from_secs(60);

/// A download's body: the Host's bytes as the browser takes them. It ends
/// complete only when the Host's read did; a failing read, or the shard
/// ending the transfer, cuts it short, which the browser sees as an
/// incomplete response. The lease and the connection's watch go with the
/// body, whether it ends or the browser leaves.
pub(super) fn paced_body(reader: PipeReader, lease: Lease, watch: ConnectionWatch) -> Body {
    struct Pacing {
        reader: PipeReader,
        lease: Lease,
        _watch: ConnectionWatch,
    }
    enum Next {
        Chunk(Bytes),
        End,
        Cut(String),
    }
    let pacing = Pacing {
        reader,
        lease,
        _watch: watch,
    };
    let chunks = futures_util::stream::unfold(Some(pacing), |pacing| async move {
        let mut pacing = pacing?;
        let next = tokio::select! {
            biased;
            () = pacing.lease.ended() => Next::Cut("the transfer was ended".to_owned()),
            chunk = pacing.reader.next() => match chunk {
                Some(Ok(bytes)) => Next::Chunk(bytes),
                Some(Err(failure)) => Next::Cut(failure.to_string()),
                None => Next::End,
            },
        };
        match next {
            Next::Chunk(bytes) => Some((Ok(bytes), Some(pacing))),
            // An error ends the response without its last chunk.
            Next::Cut(reason) => Some((Err(io::Error::other(reason)), None)),
            Next::End => None,
        }
    });
    Body::from_stream(chunks)
}

/// How an upload's copy ended short of its write.
#[derive(Debug)]
pub(super) enum UploadEnd {
    /// The file is in place.
    Written,
    /// The Host took nothing for the limit: nothing moved on the connection,
    /// which is closed.
    HostStalled,
    Refused(ApiError),
}

/// Streams the browser's body into an open upload. A chunk is read only
/// once the Host took the one before, so a slow Host slows the browser, and
/// time the Host takes does not count against the browser. However the copy
/// ends short of its end, the file stays as it was.
pub(super) async fn copy_upload(body: Body, upload: OpenUpload) -> UploadEnd {
    let OpenUpload {
        mut writer,
        written,
        lease,
    } = upload;
    let ended = || {
        UploadEnd::Refused(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::ConversationBusy,
            "The conversation changed under the upload",
        ))
    };
    let mut chunks = body.into_data_stream();
    loop {
        let next = tokio::select! {
            biased;
            () = lease.ended() => return ended(),
            next = tokio::time::timeout(TRANSFER_IDLE, chunks.next()) => next,
        };
        let chunk = match next {
            Err(_) => {
                writer.fail("the browser sent nothing for too long");
                return UploadEnd::Refused(ApiError::new(
                    StatusCode::REQUEST_TIMEOUT,
                    ErrorCode::TransferStalled,
                    "The browser sent nothing for too long",
                ));
            }
            Ok(None) => break,
            Ok(Some(Err(error))) => {
                writer.fail("the upload was cut short");
                return UploadEnd::Refused(ApiError::invalid_body(format!("The upload was cut short: {error}")));
            }
            Ok(Some(Ok(chunk))) => chunk,
        };
        let wrote = tokio::select! {
            biased;
            () = lease.ended() => return ended(),
            wrote = tokio::time::timeout(TRANSFER_IDLE, writer.write(chunk)) => wrote,
        };
        match wrote {
            Err(_) => {
                writer.fail("the Host took nothing for too long");
                return UploadEnd::HostStalled;
            }
            // The write failed on the Host; its outcome says why.
            Ok(Err(_)) => break,
            Ok(Ok(())) => {}
        }
    }
    writer.end();
    let outcome = tokio::select! {
        biased;
        () = lease.ended() => return ended(),
        outcome = written => outcome,
    };
    match outcome {
        Ok(Ok(())) => UploadEnd::Written,
        Ok(Err(error)) => UploadEnd::Refused(ApiError::host_operation(error)),
        // The shard's owner ended without an outcome only when the transfer
        // was ended.
        Err(_) => ended(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use demi_host_remote::Pipes;
    use demi_shell::HostError;
    use tokio::sync::oneshot;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::edge::listener::ConnectionControl;

    /// A pipe this process both fills and drains.
    fn pipe() -> (demi_host_remote::PipeWriter, PipeReader, Pipes) {
        let pipes = Pipes::new(Duration::from_secs(120));
        let pipe = pipes.mint(None, None);
        (pipe.writer().unwrap(), pipe.reader().unwrap(), pipes)
    }

    async fn body_bytes(body: Body) -> Result<Vec<u8>, String> {
        let mut stream = body.into_data_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk.map_err(|error| error.to_string())?);
        }
        Ok(bytes)
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_download_ends_complete_only_when_the_host_read_did() {
        // The Host's bytes pass through, and the lease goes with the body.
        let (mut writer, reader, _pipes) = pipe();
        let (lease, released) = Lease::new(CancellationToken::new());
        let control = ConnectionControl::detached();
        let body = paced_body(reader, lease, control.watch(TRANSFER_IDLE, CancellationToken::new()));
        let filling = tokio::task::spawn_local(async move {
            writer.write(Bytes::from_static(b"one ")).await.unwrap();
            writer.write(Bytes::from_static(b"two")).await.unwrap();
            writer.end();
        });
        assert_eq!(body_bytes(body).await.unwrap(), b"one two");
        filling.await.unwrap();
        released.await;

        // A read that fails cuts the body short.
        let (mut writer, reader, _pipes) = pipe();
        let (lease, _released) = Lease::new(CancellationToken::new());
        let body = paced_body(reader, lease, control.watch(TRANSFER_IDLE, CancellationToken::new()));
        let failing = tokio::task::spawn_local(async move {
            writer.write(Bytes::from_static(b"part")).await.unwrap();
            writer.fail("the device went away");
        });
        assert!(body_bytes(body).await.is_err());
        failing.await.unwrap();

        // The shard ending the transfer closes the connection at once, even
        // while nothing reads the body, as when the browser stopped taking
        // bytes; the body, read, is cut short though the Host sent nothing.
        let (_writer, reader, _pipes) = pipe();
        let ending = CancellationToken::new();
        let (lease, _released) = Lease::new(ending.clone());
        let control = ConnectionControl::detached();
        let body = paced_body(reader, lease, control.watch(TRANSFER_IDLE, ending.clone()));
        ending.cancel();
        tokio::task::yield_now().await;
        assert!(control.is_closed());
        assert!(body_bytes(body).await.is_err());
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_connection_nothing_moves_on_is_closed_and_a_browser_that_leaves_releases_the_transfer() {
        let control = ConnectionControl::detached();
        let watch = control.watch(TRANSFER_IDLE, CancellationToken::new());
        tokio::time::sleep(TRANSFER_IDLE / 2).await;
        control.touched();
        tokio::time::sleep(TRANSFER_IDLE / 2 + Duration::from_secs(1)).await;
        // Bytes moved half way: the deadline moved with them.
        assert!(!control.is_closed());
        tokio::time::sleep(TRANSFER_IDLE).await;
        assert!(control.is_closed());
        drop(watch);

        // The browser leaving drops the body, which releases the lease and
        // stops the read.
        let (mut writer, reader, _pipes) = pipe();
        let (lease, released) = Lease::new(CancellationToken::new());
        let body = paced_body(reader, lease, ConnectionControl::detached().watch(TRANSFER_IDLE, CancellationToken::new()));
        drop(body);
        released.await;
        assert!(writer.write(Bytes::from_static(b"late")).await.is_err());
    }

    fn upload() -> (OpenUpload, PipeReader, oneshot::Sender<Result<(), HostError>>, CancellationToken, Pipes) {
        let (writer, reader, pipes) = pipe();
        let (done, written) = oneshot::channel();
        let ending = CancellationToken::new();
        let (lease, _released) = Lease::new(ending.clone());
        (OpenUpload { writer, written, lease }, reader, done, ending, pipes)
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn an_upload_is_read_as_the_host_writes_and_the_host_time_does_not_count_against_the_browser() {
        let (open, mut reader, done, _ending, _pipes) = upload();
        let body = Body::from_stream(futures_util::stream::iter([
            Ok::<_, io::Error>(Bytes::from_static(b"one ")),
            Ok(Bytes::from_static(b"two ")),
            Ok(Bytes::from_static(b"three")),
        ]));
        let host = tokio::task::spawn_local(async move {
            let mut bytes = Vec::new();
            // The Host takes each chunk well after the browser sent it, and
            // the whole upload takes longer than the limit.
            loop {
                tokio::time::sleep(TRANSFER_IDLE / 2 + Duration::from_secs(5)).await;
                match reader.next().await {
                    Some(chunk) => bytes.extend_from_slice(&chunk.unwrap()),
                    None => break,
                }
            }
            done.send(Ok(())).unwrap();
            bytes
        });
        assert!(matches!(copy_upload(body, open).await, UploadEnd::Written));
        assert_eq!(host.await.unwrap(), b"one two three");
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_quiet_browser_stalls_the_upload_and_ending_the_transfer_refuses_it() {
        let (open, mut reader, _done, _ending, _pipes) = upload();
        let quiet = Body::from_stream(futures_util::stream::pending::<Result<Bytes, io::Error>>());
        let copied = copy_upload(quiet, open).await;
        assert!(matches!(&copied, UploadEnd::Refused(error) if error.code() == ErrorCode::TransferStalled), "{copied:?}");
        assert!(reader.next().await.unwrap().is_err());

        let (open, _reader, _done, ending, _pipes) = upload();
        let quiet = Body::from_stream(futures_util::stream::pending::<Result<Bytes, io::Error>>());
        let copying = tokio::task::spawn_local(copy_upload(quiet, open));
        tokio::time::sleep(Duration::from_secs(1)).await;
        ending.cancel();
        let copied = copying.await.unwrap();
        assert!(matches!(&copied, UploadEnd::Refused(error) if error.code() == ErrorCode::ConversationBusy), "{copied:?}");
    }
}
