//! `demishell:net`: the backend socket, the relay, uploads and transfers.
//!
//! TLS lives inside these primitives; no TCP or TLS primitive is exposed.
//! The protocol work is done by `tokio-tungstenite` and `hyper`; this
//! module is the glue from URLs to connections (`connect`), from
//! connections to handles, and the proxy handling user hosts need.
//! Nothing here runs until the first call, so the startup path never
//! touches it.

mod connect;
mod http;
mod tls;
mod uds;
mod ws;

use rquickjs::function::{Async, Func};
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{Ctx, Result};

pub use ws::WsResource;

pub struct NetModule;

impl ModuleDef for NetModule {
    fn declare(decl: &Declarations<'_>) -> Result<()> {
        for name in ["wsConnect", "wsSend", "wsRecv", "wsClose", "udsConnect", "udsListen", "accept", "close", "httpRequest"] {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(_ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        exports.export("wsConnect", Func::from(Async(ws::ws_connect)))?;
        exports.export("wsSend", Func::from(Async(ws::ws_send)))?;
        exports.export("wsRecv", Func::from(Async(ws::ws_recv)))?;
        exports.export("wsClose", Func::from(Async(ws::ws_close)))?;
        exports.export("udsConnect", Func::from(Async(uds::uds_connect)))?;
        exports.export("udsListen", Func::from(Async(uds::uds_listen)))?;
        exports.export("accept", Func::from(Async(uds::accept)))?;
        exports.export("close", Func::from(crate::fs::close))?;
        exports.export("httpRequest", Func::from(Async(http::http_request)))?;
        Ok(())
    }
}
