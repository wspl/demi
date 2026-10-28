//! The bytes of file transfers (`crates/backend-host-access/src/access.rs`).
//!
//! The edge moves them while the user's shard holds the transfer's admission
//! through its lease.
//!
//! - A download's body is read as the user's browser takes it. It is cut short,
//!   never ended, when the Host's read fails or the shard ends the transfer.
//!   A connection on which nothing moves for a minute is closed.
//! - An upload's body is read as the Host writes it. The upload stalls when the
//!   user's browser sends nothing for a minute, and a Host that takes nothing
//!   for a minute closes its connection.

use std::io;
use std::time::Duration;

use axum::body::Body;
use axum::http::StatusCode;
use bytes::Bytes;
use demi_runner_client::PipeReader;
use demi_host_interface::HostError;
use demi_web_api_protocol::error::ErrorCode;
use futures_util::StreamExt as _;

use super::error::ApiError;
use super::listener::ConnectionWatch;
use demi_backend_host_access::lease::Lease;
use demi_backend_host_access::transfer::OpenUpload;

/// How long a transfer waits for the user's browser, and a connection for
/// any byte, before it ends: the stalled-client timeout web servers use,
/// nginx's `send_timeout` among them.
pub(super) const TRANSFER_IDLE: Duration = Duration::from_secs(60);

/// A download's body: the Host's bytes as the user's browser takes them.
///
/// It ends complete only when the Host's read did. A failing read, or the shard
/// ending the transfer, cuts it short, and the user's browser receives an
/// incomplete response. The lease and the connection's watch go with the body,
/// whether it ends or the user's browser leaves.
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
    /// The Host's write failed, such as for what the path holds.
    WriteFailed(HostError),
    Refused(ApiError),
}

/// Streams the body the user's browser sends into an open upload.
///
/// A chunk is read only once the Host took the one before, so a slow Host slows
/// the upload, and time the Host takes does not count against the user's
/// browser. However the copy ends short of its end, the file stays as it was.
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
                writer.fail("the upload stalled: nothing arrived for a minute");
                return UploadEnd::Refused(ApiError::new(
                    StatusCode::REQUEST_TIMEOUT,
                    ErrorCode::TransferStalled,
                    "The upload stalled",
                ));
            }
            Ok(None) => break,
            Ok(Some(Err(error))) => {
                writer.fail("the upload was cut short");
                return UploadEnd::Refused(ApiError::invalid_body(format!(
                    "The upload was cut short: {error}"
                )));
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
        Ok(Err(error)) => UploadEnd::WriteFailed(error),
        // The shard's owner ended without an outcome only when the transfer
        // was ended.
        Err(_) => ended(),
    }
}
