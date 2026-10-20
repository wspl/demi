//! The plugin's side of a request: every
//! operation is one message and one answer, carried by a transport the
//! plugin host supplies. A refusal is an answer too, so it crosses a wire as
//! data.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use demi_command_declarations::NativeOperation;
use demi_host_interface::{PortError, PortRequest, PortResponse, PortTransport, RpcPort};
use demi_shared_types::{B64Bytes, BlobRef, MediaKind};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::panel::{CreatePanelTab, WorkPanel};

use crate::{
    AskedVideo, AudioAsk, AudioVoices, ImageAsk, MadeAudio, MadeImage, MediaListing, MediaRefusal, PluginId, Scope, SecretName, StoredSecret,
    VideoAsk, VideoStep, WebFailure, WebRequest, WebResponse,
};
use futures_util::future::LocalBoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};

/// One operation a plugin asks of Demi.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PortMessage {
    /// An operation of a command request's rpc port: its IO.
    Rpc {
        request: PortRequest,
    },
    /// The plugin's value `key` for the user.
    ReadValue {
        key: String,
    },
    /// Every value of the plugin's for the user.
    ListValues,
    /// Writes `key` if its revision is still `revision`; none for a value
    /// that does not exist yet. `blobs` are the blobs the value names, which
    /// stay while it names them.
    WriteValue {
        key: String,
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<u64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blobs: Vec<BlobRef>,
    },
    /// Removes `key` if its revision is still `revision`.
    RemoveValue {
        key: String,
        revision: u64,
    },
    /// Stores `bytes` in the user's blob namespace.
    PutBlob {
        bytes: B64Bytes,
    },
    /// The bytes of the user's blob `blob`.
    GetBlob {
        blob: BlobRef,
    },
    /// Replaces the plugin's set of Host directories for the user.
    SetDirectories {
        directories: Vec<HostDirectory>,
    },
    /// Reads paths on the request's conversation's primary Host, if it is
    /// running, without waking it.
    ReadHostFiles {
        reads: Vec<HostRead>,
    },
    /// The plugin's page state of `scope` changed: the user's, or the
    /// request's conversation's.
    Changed {
        scope: Scope,
    },
    /// Runs one operation of a package the plugin's commands bind, on the
    /// request's conversation's primary Host.
    PackageCall {
        operation: NativeOperation,
        args: Map<String, Value>,
        kind: CallKind,
    },
    /// The request's conversation's work panel, with only the tabs of the
    /// plugin's own panel kinds.
    PanelTabs,
    /// Creates a tab of one of the plugin's panel kinds.
    CreatePanelTab {
        tab: CreatePanelTab,
    },
    /// Sets the fields of `data` in the tab's data, removing the null ones.
    UpdatePanelTab {
        id: String,
        data: Map<String, Value>,
    },
    RemovePanelTab {
        id: String,
    },
    /// The plugin's secret `name` for the user.
    ReadSecret {
        name: SecretName,
    },
    /// Writes the secret `name` if its revision is still `revision`; none
    /// for a secret that does not exist yet.
    WriteSecret {
        name: SecretName,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<u64>,
    },
    /// Removes the secret `name` if its revision is still `revision`.
    RemoveSecret {
        name: SecretName,
        revision: u64,
    },
    /// Sends one outbound HTTPS request.
    SendRequest {
        request: WebRequest,
    },
    /// Starts the user's sign-in to the vendor the manifest names, and
    /// answers the authorization URL the page opens.
    OauthStart,
    /// A live access token of the user's sign-in, renewed first when it
    /// expires within a minute, or when it is the `refused` one the
    /// vendor's resource server refused; a `refused` token already
    /// replaced answers the current one without renewing.
    OauthToken {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refused: Option<String>,
    },
    /// Revokes the user's sign-in at the vendor when it offers revocation,
    /// and forgets it either way.
    OauthSignOut,
    /// The models of `kind` of the user's provider entries, each with the
    /// entries that offer it.
    MediaModels {
        kind: MediaKind,
    },
    /// Makes one image through an entry's account, for the request's
    /// conversation and the agent that called the command.
    MakeImage {
        ask: ImageAsk,
    },
    /// Asks for one video through an entry's account, for the request's
    /// conversation and the agent that called the command.
    ///
    /// The port answers once the vendor took the job, which the request then
    /// follows with [`PortMessage::VideoProgress`]. A request has one job at
    /// most, and it ends with the request: stopped before its video, it is
    /// cancelled at the vendor where the vendor can cancel one.
    MakeVideo {
        ask: VideoAsk,
    },
    /// Waits for the request's job's next state, or its video.
    VideoProgress,
    /// Makes one audio file through an entry's key, for the request's
    /// conversation and the agent that called the command.
    MakeAudio {
        ask: AudioAsk,
    },
    /// The voices speech model `model` speaks in, as the entry a call of it
    /// takes now lists them, `search` narrowing them where given.
    AudioVoices {
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        search: Option<String>,
    },
}

/// The answer to one [`PortMessage`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PortAnswer {
    Rpc {
        response: PortResponse,
    },
    Value {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<StoredValue>,
    },
    Values {
        values: BTreeMap<String, StoredValue>,
    },
    /// The value's new revision.
    Written {
        revision: u64,
    },
    /// The name of a blob the plugin put.
    Blob {
        blob: BlobRef,
    },
    /// A blob's bytes; none for a blob the user's namespace lacks.
    Bytes {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bytes: Option<B64Bytes>,
    },
    /// Each directory's path on every Host, in the order of the set.
    Directories {
        paths: Vec<DirectoryPath>,
    },
    /// What each read found, in the order of the reads.
    HostFiles {
        files: Vec<HostFile>,
    },
    /// The operation is done and answers nothing.
    Done,
    /// A package call's JSON result.
    Called {
        result: Value,
    },
    Panel {
        panel: WorkPanel,
    },
    /// The panel's revision once a change is in it.
    PanelRevision {
        revision: u64,
    },
    /// A secret; none for one the plugin does not have.
    Secret {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret: Option<StoredSecret>,
    },
    Response {
        response: WebResponse,
    },
    /// Where the page sends the browser to sign in.
    AuthorizationUrl {
        url: String,
    },
    AccessToken {
        token: String,
    },
    MediaModels {
        listing: MediaListing,
    },
    Image {
        image: MadeImage,
    },
    VideoAsked {
        asked: AskedVideo,
    },
    VideoProgress {
        step: VideoStep,
    },
    Audio {
        audio: MadeAudio,
    },
    AudioVoices {
        voices: AudioVoices,
    },
    Refused {
        refusal: PortRefusal,
    },
}

impl PortAnswer {
    fn name(&self) -> &'static str {
        match self {
            Self::Rpc { .. } => "rpc",
            Self::Value { .. } => "value",
            Self::Values { .. } => "values",
            Self::Written { .. } => "written",
            Self::Blob { .. } => "blob",
            Self::Bytes { .. } => "bytes",
            Self::Directories { .. } => "directories",
            Self::HostFiles { .. } => "host_files",
            Self::Done => "done",
            Self::Called { .. } => "called",
            Self::Panel { .. } => "panel",
            Self::PanelRevision { .. } => "panel_revision",
            Self::Secret { .. } => "secret",
            Self::Response { .. } => "response",
            Self::AuthorizationUrl { .. } => "authorization_url",
            Self::AccessToken { .. } => "access_token",
            Self::MediaModels { .. } => "media_models",
            Self::Image { .. } => "image",
            Self::VideoAsked { .. } => "video_asked",
            Self::VideoProgress { .. } => "video_progress",
            Self::Audio { .. } => "audio",
            Self::AudioVoices { .. } => "audio_voices",
            Self::Refused { .. } => "refused",
        }
    }
}

/// A stored value and the revision a write of it names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredValue {
    pub value: Value,
    pub revision: u64,
}

/// A directory the plugin keeps on every Host its user's jobs run on
/// (`crates/backend-host-access/src/plugin_files.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostDirectory {
    /// 1 to 64 lowercase letters, digits and hyphens, unique in the set.
    pub name: String,
    pub files: Vec<DirectoryFile>,
}

impl HostDirectory {
    /// The SHA-256 of its listing, each file's mode, SHA-256 and path in
    /// path order, in hexadecimal: two directories of the same files have
    /// the same digest.
    pub fn digest(&self) -> String {
        let mut files: Vec<&DirectoryFile> = self.files.iter().collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut listing = Sha256::new();
        for file in files {
            let mode = if file.executable { "755" } else { "644" };
            listing.update(format!("{mode} {} {}\n", file.blob.as_str(), file.path));
        }
        format!("{:x}", listing.finalize())
    }

    /// Its name on a Host: its name and the first 12 digits of its digest,
    /// which tell two contents of one name apart.
    pub fn host_name(&self) -> String {
        format!("{}-{}", self.name, &self.digest()[..12])
    }

    /// Its path on every Host, below the Host's home, in the directory of
    /// `plugin`.
    pub fn path(&self, plugin: &PluginId) -> String {
        format!("~/.demi/plugins/{plugin}/{}", self.host_name())
    }

    /// Refuses a set that names a directory twice, or a name or a file path
    /// that breaks its rule.
    pub fn check_set(directories: &[HostDirectory]) -> Result<(), String> {
        let mut names = BTreeSet::new();
        for directory in directories {
            let name = &directory.name;
            let valid = (1..=64).contains(&name.len())
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
            if !valid {
                return Err(format!(
                    "\"{name}\" is not a directory name: 1 to 64 lowercase letters, digits and hyphens"
                ));
            }
            if !names.insert(name) {
                return Err(format!("the directory \"{name}\" is named twice"));
            }
            let mut paths = BTreeSet::new();
            for file in &directory.files {
                let path = &file.path;
                let relative = !path.is_empty()
                    && path
                        .split('/')
                        .all(|part| !part.is_empty() && part != "." && part != "..");
                if !relative {
                    return Err(format!("\"{path}\" in \"{name}\" is not a relative path"));
                }
                if !paths.insert(path) {
                    return Err(format!("\"{path}\" is in \"{name}\" twice"));
                }
            }
        }
        Ok(())
    }
}

/// A file of a [`HostDirectory`], whose bytes are a blob the plugin put.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DirectoryFile {
    /// Relative, with `/` between its parts.
    pub path: String,
    pub executable: bool,
    pub blob: BlobRef,
}

/// Where a directory of the set is on every Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DirectoryPath {
    pub name: String,
    /// Below the Host's home, as `~/.demi/plugins/skills/tdd-9f2c1a7b3e40`.
    pub path: String,
}

/// One path a plugin reads on a Host, as the Host names it, and the most
/// bytes of a file to answer: absolute, or, for a command, relative to its
/// job's directory, which the Host's own rules resolve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostRead {
    pub path: String,
    pub limit: u64,
    /// A directory `path` is under, below which no link is followed: one
    /// there answers [`HostFile::Link`]; absent, links are followed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
}

impl HostRead {
    /// A read of `path` that follows links, as most reads do.
    pub fn new(path: String, limit: u64) -> Self {
        Self { path, limit, within: None }
    }
}

/// What a path on a Host is. A symbolic link is followed, unless the read
/// names a directory it is `within`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HostFile {
    Missing,
    /// A directory's entries, in the Host's order.
    Directory {
        entries: Vec<HostEntry>,
    },
    /// A file's first bytes, at most the read's limit, and its size.
    File {
        bytes: B64Bytes,
        size: u64,
    },
    /// Neither a file nor a directory.
    Other,
    /// A link below the directory the read named it `within`, not followed.
    Link,
    /// The Host could not read it, such as for want of permission.
    Unreadable {
        message: String,
    },
}

/// An entry of a directory on a Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostEntry {
    pub name: String,
    pub kind: EntryKind,
}

/// An entry's kind, without following a symbolic link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// What a package call does on the Host, which decides whether it wakes a
/// stopped Cloud and whether it is activity (`backend-user-shard`'s
/// `crates/backend-user-shard/src/lifecycle/mod.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    /// Work the user starts, such as opening a tab: it wakes a stopped
    /// Cloud.
    Starts,
    /// An operation on what runs there, such as closing a tab: activity,
    /// but a stopped Cloud is refused rather than woken.
    Operates,
    /// A look, such as listing the tabs: a stopped Cloud is refused, and
    /// the look is no activity.
    Looks,
}

/// Why Demi refused a port operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PortRefusal {
    /// The conversation's host access refused, as its routes would: a page
    /// call that passes it on answers with its code and status.
    #[error("{message}")]
    Host {
        code: ErrorCode,
        status: u16,
        message: String,
    },
    /// The package operation exited nonzero, with what it wrote to its
    /// standard error.
    #[error("the operation failed: {stderr}")]
    Operation { stderr: String },
    /// The Host could not start the operation: its package's service did
    /// not start, or the runner refused to start it, in the runner's words
    /// (`crates/runner-jobs/src/commands/streams.rs`).
    #[error("{message}")]
    Service { message: String },
    /// Another write of the value came first.
    #[error("the value changed since it was read")]
    Conflict,
    /// The operation needs a conversation, and the request has none.
    #[error("the request has no conversation")]
    NoConversation,
    /// The conversation's primary Host is not running: a stopped Cloud, or a
    /// device whose runner is not connected. A read never wakes it.
    #[error("the conversation's Host is not running")]
    NotRunning,
    /// A change of the work panel was refused, as its routes would refuse
    /// it, such as `panel_full`.
    #[error("{message}")]
    Panel { code: ErrorCode, message: String },
    /// An outbound request, or a step of an OAuth sign-in that reaches the
    /// vendor, did not answer.
    #[error("{failure}")]
    Web { failure: WebFailure },
    /// The user is not signed in to the plugin's vendor: never, signed out,
    /// or the vendor refused to renew the sign-in, which the host then
    /// forgot.
    #[error("the user is not signed in")]
    SignedOut,
    /// No image, video or audio was made.
    #[error("{refusal}")]
    Media { refusal: MediaRefusal },
}

/// Why a typed port operation did not answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortFailure {
    #[error(transparent)]
    Port(#[from] PortError),
    #[error(transparent)]
    Refused(PortRefusal),
}

/// What carries a port's messages: the plugin host's answers in process, a
/// wire to a plugin process, or a test's.
pub trait PluginTransport {
    fn request(&self, message: PortMessage) -> LocalBoxFuture<'_, Result<PortAnswer, PortError>>;
}

/// The plugin's side of one request.
#[derive(Clone)]
pub struct PluginPort {
    transport: Rc<dyn PluginTransport>,
    cancel: CancellationToken,
}

impl PluginPort {
    pub fn new(transport: Rc<dyn PluginTransport>, cancel: CancellationToken) -> Self {
        Self { transport, cancel }
    }

    /// A command request's rpc port, whose operations travel as
    /// [`PortMessage::Rpc`].
    pub fn rpc(&self) -> RpcPort {
        RpcPort::new(
            Rc::new(RpcMessages(self.transport.clone())),
            self.cancel.clone(),
        )
    }

    /// The loopback's JSON round trip wraps the port's own transport.
    #[cfg(feature = "testing")]
    pub(crate) fn transport(&self) -> &Rc<dyn PluginTransport> {
        &self.transport
    }

    #[cfg(feature = "testing")]
    pub(crate) fn cancellation(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Completes once the request is stopped.
    pub fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.cancel.cancelled()
    }

    /// Sends `message`; a refusal is the operation's failure.
    async fn ask(&self, message: PortMessage) -> Result<PortAnswer, PortFailure> {
        match self.transport.request(message).await? {
            PortAnswer::Refused { refusal } => Err(PortFailure::Refused(refusal)),
            answer => Ok(answer),
        }
    }

    pub async fn value(&self, key: impl Into<String>) -> Result<Option<StoredValue>, PortFailure> {
        match self.ask(PortMessage::ReadValue { key: key.into() }).await? {
            PortAnswer::Value { value } => Ok(value),
            answer => Err(unexpected("read_value", &answer)),
        }
    }

    pub async fn values(&self) -> Result<BTreeMap<String, StoredValue>, PortFailure> {
        match self.ask(PortMessage::ListValues).await? {
            PortAnswer::Values { values } => Ok(values),
            answer => Err(unexpected("list_values", &answer)),
        }
    }

    /// Writes `key` if it is still at `revision`, none for a new value, and
    /// answers its new revision; [`PortRefusal::Conflict`] when another
    /// write came first.
    pub async fn write_value(
        &self,
        key: impl Into<String>,
        value: Value,
        revision: Option<u64>,
    ) -> Result<u64, PortFailure> {
        self.write_value_naming(key, value, revision, Vec::new())
            .await
    }

    /// Writes `key` as [`write_value`](Self::write_value) does, for a value
    /// that names `blobs`.
    pub async fn write_value_naming(
        &self,
        key: impl Into<String>,
        value: Value,
        revision: Option<u64>,
        blobs: Vec<BlobRef>,
    ) -> Result<u64, PortFailure> {
        let message = PortMessage::WriteValue {
            key: key.into(),
            value,
            revision,
            blobs,
        };
        match self.ask(message).await? {
            PortAnswer::Written { revision } => Ok(revision),
            answer => Err(unexpected("write_value", &answer)),
        }
    }

    pub async fn put_blob(&self, bytes: B64Bytes) -> Result<BlobRef, PortFailure> {
        match self.ask(PortMessage::PutBlob { bytes }).await? {
            PortAnswer::Blob { blob } => Ok(blob),
            answer => Err(unexpected("put_blob", &answer)),
        }
    }

    pub async fn blob(&self, blob: BlobRef) -> Result<Option<B64Bytes>, PortFailure> {
        match self.ask(PortMessage::GetBlob { blob }).await? {
            PortAnswer::Bytes { bytes } => Ok(bytes),
            answer => Err(unexpected("get_blob", &answer)),
        }
    }

    /// Replaces the plugin's Host directories for the user, and answers
    /// each one's path on every Host.
    pub async fn set_directories(
        &self,
        directories: Vec<HostDirectory>,
    ) -> Result<Vec<DirectoryPath>, PortFailure> {
        match self
            .ask(PortMessage::SetDirectories { directories })
            .await?
        {
            PortAnswer::Directories { paths } => Ok(paths),
            answer => Err(unexpected("set_directories", &answer)),
        }
    }

    /// Reads `reads` on the conversation's primary Host, one answer per read;
    /// [`PortRefusal::NotRunning`] when the Host is not running.
    pub async fn read_host_files(
        &self,
        reads: Vec<HostRead>,
    ) -> Result<Vec<HostFile>, PortFailure> {
        match self.ask(PortMessage::ReadHostFiles { reads }).await? {
            PortAnswer::HostFiles { files } => Ok(files),
            answer => Err(unexpected("read_host_files", &answer)),
        }
    }

    /// Removes `key` if it is still at `revision`;
    /// [`PortRefusal::Conflict`] when another write came first.
    pub async fn remove_value(
        &self,
        key: impl Into<String>,
        revision: u64,
    ) -> Result<(), PortFailure> {
        let message = PortMessage::RemoveValue {
            key: key.into(),
            revision,
        };
        self.done("remove_value", message).await
    }

    /// Marks the plugin's page state of `scope` as changed, so the pages
    /// that show it read it again; [`PortRefusal::NoConversation`] for the
    /// conversation scope of a request without one.
    pub async fn changed(&self, scope: Scope) -> Result<(), PortFailure> {
        self.done("changed", PortMessage::Changed { scope }).await
    }

    pub async fn package_call(
        &self,
        operation: NativeOperation,
        args: Map<String, Value>,
        kind: CallKind,
    ) -> Result<Value, PortFailure> {
        let message = PortMessage::PackageCall {
            operation,
            args,
            kind,
        };
        match self.ask(message).await? {
            PortAnswer::Called { result } => Ok(result),
            answer => Err(unexpected("package_call", &answer)),
        }
    }

    /// The request's conversation's work panel, with the tabs of the
    /// plugin's own panel kinds.
    pub async fn panel_tabs(&self) -> Result<WorkPanel, PortFailure> {
        match self.ask(PortMessage::PanelTabs).await? {
            PortAnswer::Panel { panel } => Ok(panel),
            answer => Err(unexpected("panel_tabs", &answer)),
        }
    }

    /// Creates a tab of one of the plugin's panel kinds, and answers the
    /// panel's revision; nothing changes for an id the panel has or had.
    pub async fn create_panel_tab(&self, tab: CreatePanelTab) -> Result<u64, PortFailure> {
        self.panel_change("create_panel_tab", PortMessage::CreatePanelTab { tab })
            .await
    }

    /// Sets the fields of `data` in the tab's data, removing the null ones,
    /// and answers the panel's revision; nothing changes for a tab the panel
    /// no longer has.
    pub async fn update_panel_tab(
        &self,
        id: impl Into<String>,
        data: Map<String, Value>,
    ) -> Result<u64, PortFailure> {
        let message = PortMessage::UpdatePanelTab {
            id: id.into(),
            data,
        };
        self.panel_change("update_panel_tab", message).await
    }

    pub async fn remove_panel_tab(&self, id: impl Into<String>) -> Result<u64, PortFailure> {
        let message = PortMessage::RemovePanelTab { id: id.into() };
        self.panel_change("remove_panel_tab", message).await
    }

    /// The plugin's secret `name` for the user, if it has one.
    pub async fn secret(&self, name: SecretName) -> Result<Option<StoredSecret>, PortFailure> {
        match self.ask(PortMessage::ReadSecret { name }).await? {
            PortAnswer::Secret { secret } => Ok(secret),
            answer => Err(unexpected("read_secret", &answer)),
        }
    }

    /// Writes the secret `name` if it is still at `revision`, none for a
    /// new secret, and answers its new revision; [`PortRefusal::Conflict`]
    /// when another write came first.
    pub async fn write_secret(
        &self,
        name: SecretName,
        value: String,
        revision: Option<u64>,
    ) -> Result<u64, PortFailure> {
        let message = PortMessage::WriteSecret {
            name,
            value,
            revision,
        };
        match self.ask(message).await? {
            PortAnswer::Written { revision } => Ok(revision),
            answer => Err(unexpected("write_secret", &answer)),
        }
    }

    /// Removes the secret `name` if it is still at `revision`;
    /// [`PortRefusal::Conflict`] when another write came first.
    pub async fn remove_secret(&self, name: SecretName, revision: u64) -> Result<(), PortFailure> {
        self.done("remove_secret", PortMessage::RemoveSecret { name, revision })
            .await
    }

    /// Sends `request`, and answers the vendor's response whatever its
    /// status; [`PortRefusal::Web`] when none came.
    pub async fn send(&self, request: WebRequest) -> Result<WebResponse, PortFailure> {
        match self.ask(PortMessage::SendRequest { request }).await? {
            PortAnswer::Response { response } => Ok(response),
            answer => Err(unexpected("send_request", &answer)),
        }
    }

    /// Starts the user's sign-in and answers the authorization URL, which
    /// the page opens in a new tab.
    pub async fn oauth_start(&self) -> Result<String, PortFailure> {
        match self.ask(PortMessage::OauthStart).await? {
            PortAnswer::AuthorizationUrl { url } => Ok(url),
            answer => Err(unexpected("oauth_start", &answer)),
        }
    }

    /// A live access token of the user's sign-in; [`PortRefusal::SignedOut`]
    /// when the user is not signed in.
    pub async fn oauth_token(&self) -> Result<String, PortFailure> {
        self.access_token(None).await
    }

    /// The access token to send in place of `refused`, which the vendor's
    /// resource server refused with 401: renewed now, or the current one
    /// when `refused` was already replaced. [`PortRefusal::SignedOut`] when
    /// the vendor refused the refresh token, which forgets the sign-in.
    pub async fn oauth_renew(&self, refused: &str) -> Result<String, PortFailure> {
        self.access_token(Some(refused.to_owned())).await
    }

    async fn access_token(&self, refused: Option<String>) -> Result<String, PortFailure> {
        match self.ask(PortMessage::OauthToken { refused }).await? {
            PortAnswer::AccessToken { token } => Ok(token),
            answer => Err(unexpected("oauth_token", &answer)),
        }
    }

    /// Signs the user out: the vendor revokes the tokens when it offers
    /// revocation, and the host forgets them either way.
    pub async fn oauth_sign_out(&self) -> Result<(), PortFailure> {
        self.done("oauth_sign_out", PortMessage::OauthSignOut).await
    }

    /// The models of `kind` of the user's provider entries, each with the
    /// entries that offer it.
    pub async fn media_models(&self, kind: MediaKind) -> Result<MediaListing, PortFailure> {
        match self.ask(PortMessage::MediaModels { kind }).await? {
            PortAnswer::MediaModels { listing } => Ok(listing),
            answer => Err(unexpected("media_models", &answer)),
        }
    }

    /// Makes the image `ask` names; [`PortRefusal::Media`] when none was
    /// made.
    pub async fn make_image(&self, ask: ImageAsk) -> Result<MadeImage, PortFailure> {
        match self.ask(PortMessage::MakeImage { ask }).await? {
            PortAnswer::Image { image } => Ok(image),
            answer => Err(unexpected("make_image", &answer)),
        }
    }

    /// Asks for the video `ask` names and answers the job the vendor took;
    /// [`PortRefusal::Media`] when none was taken.
    pub async fn make_video(&self, ask: VideoAsk) -> Result<AskedVideo, PortFailure> {
        match self.ask(PortMessage::MakeVideo { ask }).await? {
            PortAnswer::VideoAsked { asked } => Ok(asked),
            answer => Err(unexpected("make_video", &answer)),
        }
    }

    /// The request's job's next state, or its video; [`PortRefusal::Media`]
    /// when the job failed, expired or outlasted the call's bound.
    pub async fn video_progress(&self) -> Result<VideoStep, PortFailure> {
        match self.ask(PortMessage::VideoProgress).await? {
            PortAnswer::VideoProgress { step } => Ok(step),
            answer => Err(unexpected("video_progress", &answer)),
        }
    }

    /// Makes the audio `ask` names; [`PortRefusal::Media`] when none was
    /// made.
    pub async fn make_audio(&self, ask: AudioAsk) -> Result<MadeAudio, PortFailure> {
        match self.ask(PortMessage::MakeAudio { ask }).await? {
            PortAnswer::Audio { audio } => Ok(audio),
            answer => Err(unexpected("make_audio", &answer)),
        }
    }

    /// The voices of speech model `model`, narrowed by `search`;
    /// [`PortRefusal::Media`] when no entry can list them.
    pub async fn audio_voices(&self, model: String, search: Option<String>) -> Result<AudioVoices, PortFailure> {
        match self.ask(PortMessage::AudioVoices { model, search }).await? {
            PortAnswer::AudioVoices { voices } => Ok(voices),
            answer => Err(unexpected("audio_voices", &answer)),
        }
    }

    async fn panel_change(
        &self,
        asked: &'static str,
        message: PortMessage,
    ) -> Result<u64, PortFailure> {
        match self.ask(message).await? {
            PortAnswer::PanelRevision { revision } => Ok(revision),
            answer => Err(unexpected(asked, &answer)),
        }
    }

    async fn done(&self, asked: &'static str, message: PortMessage) -> Result<(), PortFailure> {
        match self.ask(message).await? {
            PortAnswer::Done => Ok(()),
            answer => Err(unexpected(asked, &answer)),
        }
    }
}

fn unexpected(asked: &'static str, answer: &PortAnswer) -> PortFailure {
    PortFailure::Port(PortError::Unexpected {
        asked,
        answered: answer.name(),
    })
}

/// An rpc port's operations as port messages.
struct RpcMessages(Rc<dyn PluginTransport>);

impl PortTransport for RpcMessages {
    fn request(&self, request: PortRequest) -> LocalBoxFuture<'_, Result<PortResponse, PortError>> {
        Box::pin(async move {
            match self.0.request(PortMessage::Rpc { request }).await? {
                PortAnswer::Rpc { response } => Ok(response),
                answer => Err(PortError::Unexpected {
                    asked: "rpc",
                    answered: answer.name(),
                }),
            }
        })
    }
}
