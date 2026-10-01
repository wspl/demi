//! Inherited pipe transport for a parent-owned native service executable.

use crate::{Handler, ServiceError};
use std::{io, sync::Arc};
use tokio::io::{AsyncRead, AsyncWrite};

/// The executable owns the process lifetime. On Windows it must exit after this
/// returns to release a synchronous input worker still waiting on the parent.
pub async fn serve_stdio<H: Handler + ?Sized>(handler: Arc<H>) -> Result<(), ServiceError> {
    crate::serve(transport()?, handler).await
}

#[cfg(unix)]
fn transport() -> io::Result<impl AsyncRead + AsyncWrite + Unpin> {
    use std::os::fd::AsFd;
    use tokio::net::unix::pipe::{Receiver, Sender};
    Ok(tokio::io::join(
        Receiver::from_owned_fd(std::io::stdin().as_fd().try_clone_to_owned()?)?,
        Sender::from_owned_fd(std::io::stdout().as_fd().try_clone_to_owned()?)?,
    ))
}

#[cfg(windows)]
fn transport() -> io::Result<impl AsyncRead + AsyncWrite + Unpin> {
    use std::os::windows::io::AsHandle;
    let input = std::fs::File::from(std::io::stdin().as_handle().try_clone_to_owned()?);
    let output = std::fs::File::from(std::io::stdout().as_handle().try_clone_to_owned()?);
    Ok(tokio::io::join(
        tokio::fs::File::from_std(input),
        tokio::fs::File::from_std(output),
    ))
}
