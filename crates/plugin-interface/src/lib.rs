//! The plugin contract and nothing that implements it.
//!
//! A plugin is a factory, shared by every shard thread, that carries its
//! manifest and makes one instance per user shard. An instance receives
//! requests as data and acts only through its port, whose every operation is
//! one message and one answer.
//!
//! A plugin linked into the backend passes the messages as Rust values. A
//! plugin process would carry the same messages over a wire, and every plugin's
//! tests run through the JSON loopback in [`testing`] to keep the two the same.
//!
//! ```text
//!                  Request, Reply, port messages (this crate's types)
//! plugin host  <------------------------------------------------>  plugin
//!              in process: a call of the trait, messages as Rust values
//!              as a process (later): the same messages, encoded, over its stdio
//! ```
//!
//! # Messages only
//!
//! Everything a plugin receives is a message, so no plugin ever receives a live
//! object: no Host handle, no callback into the backend, no reference to a
//! session or a shard. That is:
//!
//! - what keeps a plugin written with a TypeScript SDK possible without
//!   embedding the Rust runtime (no such SDK is built);
//! - why a plugin's values change by versioned compare-and-set rather than by a
//!   callback inside a transaction;
//! - why a plugin names the directories a Host must hold instead of writing to
//!   a Host.
//!
//! # Rules
//!
//! Three rules keep the two transports the same:
//!
//! - Every message type is serializable and has a JSON Schema (serde and
//!   schemars), so a type that could not cross a wire does not compile into the
//!   contract.
//! - A plugin crate depends on this crate and on contract crates, never on a
//!   backend crate, so it cannot reach past the port.
//! - A service joins the port when a plugin needs it, as Hosts did, and never
//!   as an operation shaped for one plugin among the others.
//!
//! A blob a value or a directory names is never removed while it names it.
//!
//! This crate must not implement a plugin or a transport to a process, depend
//! on the agent's crates, or carry a live object in any message.

mod manifest;
mod plugin;
mod port;
pub mod project;
mod providers;
mod request;
#[cfg(feature = "testing")]
pub mod testing;
mod web;

pub use manifest::{
    Commands, Constant, DEMI_ROOT, DEMI_SUMMARY, EXECUTION_SOURCE, Manifest, Method, OAuth, Page,
    Placement, PluginId, PluginIdError, Scope, State, Stream,
};
pub use web::{
    SecretName, SecretNameError, StoredSecret, WebFailure, WebHeader, WebMethod, WebRequest,
    WebResponse,
};
pub use plugin::{CommandPlugin, NoRequests, Plugin, PluginFactory, PortHandled};
pub use port::{
    CallKind, DirectoryFile, DirectoryPath, EntryKind, HostDirectory, HostEntry, HostFile,
    HostRead, PluginPort, PluginTransport, PortAnswer, PortFailure, PortMessage, PortRefusal,
    StoredValue,
};
pub use providers::{
    AskedVideo, AudioAsk, AudioFacts, AudioFileFormat, AudioOption, AudioVoices, MadeAudio, PcmShape, VoiceInfo,
    CustomSizeRule, EntryStatus, ImageAsk, ImageAspect, ImageFacts, ImageInput, ImageKind, MadeImage,
    MadeVideo, MediaListing, MediaModel, MediaPrice, MediaRefusal, MediaUnlisted, ModelEntry, PriceRate, Unable,
    VideoAsk, VideoFacts, VideoOption, VideoResolution, VideoStep,
};
pub use request::{PluginError, Reply, Request};
/// What a project search names to watch (`Reply::Project`).
pub use demi_web_api_protocol::files::SubscriptionKind;
pub use demi_web_api_protocol::workflows::WatchSubject;
