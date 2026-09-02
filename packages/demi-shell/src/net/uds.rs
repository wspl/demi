//! Unix domain sockets: connect, listen and accept. Streams enter the
//! handle table and are read and written through `demishell:fs`.

use std::os::unix::fs::PermissionsExt;
use std::rc::Rc;

use rquickjs::{Ctx, Object, Result};
use tokio::net::{UnixListener, UnixStream};

use crate::error::{bad_handle, throw_errno, throw_io};
use crate::handles::{Resource, Stream};
use crate::state::{state, Activity};

fn insert_stream(ctx: &Ctx<'_>, stream: UnixStream) -> i32 {
    let (r, w) = stream.into_split();
    state(ctx).handles.borrow_mut().insert(Resource::Stream(Stream::new(Some(Box::new(r)), Some(Box::new(w)))))
}

pub async fn uds_connect<'js>(ctx: Ctx<'js>, path: String) -> Result<i32> {
    let _activity = Activity::begin(&ctx);
    let stream = UnixStream::connect(&path).await.map_err(|e| throw_io(&ctx, e, "connect", Some(&path)))?;
    Ok(insert_stream(&ctx, stream))
}

/// Binds `path`; the mode is applied before the listener is returned, so
/// no client ever sees the socket with the default permissions.
pub async fn uds_listen<'js>(ctx: Ctx<'js>, path: String, options: Object<'js>) -> Result<i32> {
    let mode: u32 = options.get("mode")?;
    let listener = UnixListener::bind(&path).map_err(|e| throw_io(&ctx, e, "listen", Some(&path)))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).map_err(|e| throw_io(&ctx, e, "chmod", Some(&path)))?;
    Ok(state(&ctx).handles.borrow_mut().insert(Resource::Listener(Rc::new(listener), Rc::new(tokio::sync::Notify::new()))))
}

pub async fn accept<'js>(ctx: Ctx<'js>, id: i32) -> Result<i32> {
    let _activity = Activity::begin(&ctx);
    let (listener, cancel) = {
        let st = state(&ctx);
        let mut handles = st.handles.borrow_mut();
        match handles.get_mut(id) {
            Some(Resource::Listener(l, c)) => (l.clone(), c.clone()),
            Some(_) => return Err(throw_errno(&ctx, libc::ENOTSOCK, "accept", None)),
            None => return Err(bad_handle(&ctx, "accept")),
        }
    };
    let accepted = tokio::select! {
        r = listener.accept() => r,
        _ = cancel.notified() => Err(std::io::Error::from_raw_os_error(libc::ECANCELED)),
    };
    let (stream, _) = accepted.map_err(|e| throw_io(&ctx, e, "accept", None))?;
    Ok(insert_stream(&ctx, stream))
}
