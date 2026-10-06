//! The plugin contract (`plugins.md` § The contract) and nothing that
//! implements it. A plugin is a factory, shared by every shard thread, that
//! carries its manifest and makes one instance per user shard; an instance
//! receives requests as data and acts only through its port, whose every
//! operation is one message and one answer. A plugin linked into the backend
//! passes the messages as Rust values; a plugin process would carry the same
//! messages over a wire, and every plugin's tests run through the JSON
//! loopback in [`testing`] to keep the two the same.

mod manifest;
mod plugin;
mod port;
mod request;
#[cfg(feature = "testing")]
pub mod testing;

pub use manifest::{
    Commands, Constant, DEMI_ROOT, DEMI_SUMMARY, EXECUTION_SOURCE, Manifest, Method, Page,
    Placement, PluginId, PluginIdError, Scope, State, Stream, Topic,
};
pub use plugin::{CommandPlugin, NoRequests, Plugin, PluginFactory, PortHandled};
pub use port::{
    CallKind, DirectoryFile, DirectoryPath, EntryKind, HostDirectory, HostEntry, HostFile,
    HostRead, PluginPort, PluginTransport, PortAnswer, PortFailure, PortMessage, PortRefusal,
    StoredValue,
};
pub use request::{PanelTabChange, PluginError, Reply, Request};
