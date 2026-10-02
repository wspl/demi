//! The plugin's side of a request (`plugins.md` § The contract): every
//! operation is one message and one answer, carried by a transport the
//! plugin host supplies. A refusal is an answer too, so it crosses a wire as
//! data.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use demi_command_declarations::NativeOperation;
use demi_host_interface::{PortError, PortRequest, PortResponse, PortTransport, RpcPort};
use demi_shared_types::{B64Bytes, BlobRef, Timestamp};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::exposes::ExposeAddress;
use demi_web_api_protocol::ids::{DeviceId, ExposeId};
use demi_web_api_protocol::panel::{CreatePanelTab, WorkPanel};

use crate::{PluginId, Scope};
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
    /// An operation of a command request's rpc port: its IO and the
    /// invoking node's command storage.
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
    /// Reads paths on the request's conversation's main Host, if it is
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
    /// request's conversation's main Host.
    PackageCall {
        operation: NativeOperation,
        args: Map<String, Value>,
        kind: CallKind,
    },
    /// The request's conversation's main and attached Hosts.
    ConversationHosts,
    /// The user's live exposes, soonest expiry first.
    ListExposes,
    /// A new expose of `address` on the user's `device`, for `lifetime`
    /// seconds.
    CreateExpose {
        device: DeviceId,
        address: String,
        lifetime: u64,
    },
    /// Moves the expose's expiry to `lifetime` seconds from now.
    RenewExpose {
        expose: ExposeId,
        lifetime: u64,
    },
    /// Destroys the expose at once.
    RemoveExpose {
        expose: ExposeId,
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
    Hosts {
        hosts: Vec<ConversationHost>,
    },
    Exposes {
        list: ExposeList,
    },
    Expose {
        expose: ExposeRecord,
    },
    Panel {
        panel: WorkPanel,
    },
    /// The panel's revision once a change is in it.
    PanelRevision {
        revision: u64,
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
            Self::Hosts { .. } => "hosts",
            Self::Exposes { .. } => "exposes",
            Self::Expose { .. } => "expose",
            Self::Panel { .. } => "panel",
            Self::PanelRevision { .. } => "panel_revision",
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
/// (`plugins.md` § Host directories).
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

/// One path a plugin reads on a Host: absolute, as the Host names it, and
/// the most bytes of a file to answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostRead {
    pub path: String,
    pub limit: u64,
}

/// What a path on a Host is. A symbolic link is followed.
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
/// stopped Cloud and whether it is activity (`resource-lifecycle.md`
/// § Activity).
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

/// A Host of the request's conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationHost {
    /// Its name as `demi host list` shows it.
    pub name: String,
    pub device: DeviceId,
    pub role: HostRole,
    pub online: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostRole {
    Main,
    Attached,
}

/// The user's live exposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExposeList {
    /// Whether the instance has an expose domain; without one there are no
    /// exposes.
    pub available: bool,
    /// When Demi listed them, by the clock their expiries are read by.
    pub listed_at: Timestamp,
    /// Soonest expiry first.
    pub exposes: Vec<ExposeRecord>,
}

/// An expose as the port shows it (`expose.md` § The expose record).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExposeRecord {
    pub id: ExposeId,
    pub device: DeviceId,
    /// The device's name, the Cloud's as `Cloud`.
    pub device_name: String,
    pub address: ExposeAddress,
    pub url: String,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
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
    /// Another write of the value came first.
    #[error("the value changed since it was read")]
    Conflict,
    /// An expose operation was refused.
    #[error("{message}")]
    Expose {
        reason: ExposeRefusal,
        message: String,
    },
    /// The operation needs a conversation, and the request has none.
    #[error("the request has no conversation")]
    NoConversation,
    /// The conversation's main Host is not running: a stopped Cloud, or a
    /// device whose runner is not connected. A read never wakes it.
    #[error("the conversation's Host is not running")]
    NotRunning,
    /// A change of the work panel was refused, as its routes would refuse
    /// it, such as `panel_full` (`web-api.md` § Work panel state).
    #[error("{message}")]
    Panel { code: ErrorCode, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExposeRefusal {
    /// The instance has no expose domain.
    Unavailable,
    /// The address is not `host:port` or a port.
    InvalidAddress,
    /// The device is not the user's.
    DeviceNotFound,
    /// The device's runner is not connected, or its Cloud is not running.
    DeviceOffline,
    /// The user has no live expose of that id.
    NotFound,
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

    /// Reads `reads` on the conversation's main Host, one answer per read;
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

    pub async fn conversation_hosts(&self) -> Result<Vec<ConversationHost>, PortFailure> {
        match self.ask(PortMessage::ConversationHosts).await? {
            PortAnswer::Hosts { hosts } => Ok(hosts),
            answer => Err(unexpected("conversation_hosts", &answer)),
        }
    }

    pub async fn exposes(&self) -> Result<ExposeList, PortFailure> {
        match self.ask(PortMessage::ListExposes).await? {
            PortAnswer::Exposes { list } => Ok(list),
            answer => Err(unexpected("list_exposes", &answer)),
        }
    }

    pub async fn create_expose(
        &self,
        device: DeviceId,
        address: String,
        lifetime: u64,
    ) -> Result<ExposeRecord, PortFailure> {
        let message = PortMessage::CreateExpose {
            device,
            address,
            lifetime,
        };
        self.expose("create_expose", message).await
    }

    pub async fn renew_expose(
        &self,
        expose: ExposeId,
        lifetime: u64,
    ) -> Result<ExposeRecord, PortFailure> {
        let message = PortMessage::RenewExpose { expose, lifetime };
        self.expose("renew_expose", message).await
    }

    pub async fn remove_expose(&self, expose: ExposeId) -> Result<(), PortFailure> {
        self.done("remove_expose", PortMessage::RemoveExpose { expose })
            .await
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

    async fn expose(
        &self,
        asked: &'static str,
        message: PortMessage,
    ) -> Result<ExposeRecord, PortFailure> {
        match self.ask(message).await? {
            PortAnswer::Expose { expose } => Ok(expose),
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
