//! The body of every JSON error, and the one list of the codes it carries.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A JSON error answer: `{ code, message }`, and a plugin's own `reason`
/// when a plugin refused the call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    pub code: ErrorCode,
    /// Says what happened in words; for a refused body it names the field and
    /// the reason.
    pub message: String,
    /// With `plugin_refused`, the plugin's snake_case word for the refusal,
    /// such as `tab_not_found`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Every error code the web app can see. A situation has one code on every
/// route (`web-api.md` § Resource index); the HTTP status belongs to the
/// route that answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request carries no live session.
    Unauthenticated,
    /// No route answers the method and path, or the path names nothing the
    /// caller holds, such as a blob outside the caller's namespace.
    NotFound,
    /// A JSON body does not match its request type.
    InvalidBody,
    /// A body the backend reads whole is over its limit.
    TooLarge,
    /// An upload that sent no bytes named a SHA-256 the caller's blobs do
    /// not hold: the page sends the bytes next.
    BlobMissing,
    /// The backend failed in a way the request could not cause.
    InternalError,
    /// The backend is shutting down and starts no new work.
    BackendClosing,
    /// Setup has created the master account already.
    AlreadySetUp,
    /// The email address and password, or the current password, do not match.
    InvalidCredentials,
    /// Too many attempts for now: five failed logins for one address within
    /// a minute of each other, or a new verification code within a minute of
    /// the last one.
    TooManyAttempts,
    /// Another account, or the caller's own, has the email address.
    EmailTaken,
    /// The verification code is wrong, expired or used up, or its challenge
    /// no longer holds; or no runner waits with the pairing code.
    InvalidCode,
    /// The backend has no account mail sender to deliver a verification code.
    MailUnavailable,
    /// The verification mail could not be delivered.
    MailFailed,
    /// A query parameter is not one the route reads, such as `refresh=1`.
    InvalidQuery,
    /// The caller's role does not allow the operation, such as a user who
    /// only infers configuring a shared instance's providers.
    Forbidden,
    /// The provider entry does not exist in the caller's scope.
    ProviderNotFound,
    /// The provider entry's catalog does not list the model.
    ModelNotFound,
    /// The conversation's model does not offer the thinking effort or the
    /// service tier.
    SettingUnavailable,
    /// The conversation has no model yet: it cannot be opened, titled, or
    /// given an effort or a tier.
    ModelNotSelected,
    /// Another change of the entry is still running, such as a device login
    /// into it.
    ProviderBusy,
    /// The scope already holds its one subscription entry of the family.
    ProviderExists,
    /// No family has that name.
    UnknownProviderType,
    /// models.dev lists no vendor of that id that a family speaks to.
    UnknownVendor,
    /// A subscription family is created by its login, and its entry takes
    /// only a new label.
    SubscriptionOnly,
    /// The entry does not take accounts this way: an API-key entry has none,
    /// and a family adds accounts by device login or by a token, not both.
    AccountsUnsupported,
    /// The family has no device login.
    NoLoginFlow,
    /// No login of the caller's has that id, or its result was dropped ten
    /// minutes after it finished.
    LoginNotFound,
    /// The entry holds no account of that id.
    AccountNotFound,
    /// The active account cannot be removed: select another first, or delete
    /// the provider.
    ActiveAccount,
    /// The supplied setup token is not an account of the family; the message
    /// never repeats it.
    TokenImportFailed,
    /// Reading the provider's usage would spend an inference request.
    QuotaRequiresInference,
    /// The vendor's usage endpoint did not answer, refused, or answered what
    /// cannot be read.
    QuotaUnavailable,
    /// The vendor list could not be read from models.dev.
    CatalogUnavailable,
    /// The entry's provider could not be built or read, which the sync
    /// state shows as the entry's `failed` details.
    ProviderStatusFailed,
    /// Too many pairing codes tried: ten claims per user within a minute.
    RateLimited,
    /// The caller owns no device of that id, or not one the route takes,
    /// such as the Cloud for revocation.
    DeviceNotFound,
    /// The device's runner is not connected.
    DeviceOffline,
    /// The device is the caller's Cloud, which no direct channel reaches
    /// (`web-api.md` § Direct channel).
    NotAPairedDevice,
    /// The Host's log could not be read.
    LogUnreadable,
    /// The caller owns no conversation of that id.
    ConversationNotFound,
    /// The conversation is archived: restore it first.
    ConversationArchived,
    /// The conversation has no undecided permission request of that id:
    /// another page decided it, an Allow of its category did, or a newer
    /// request replaced it.
    PermissionRequestNotFound,
    /// The work panel holds as many tabs, or as much of their data, as it
    /// can (`web-api.md` § Work panel state).
    PanelFull,
    /// No plugin the user has on declares the work panel kind.
    UnknownPanelKind,
    /// An archive, a target change or a detach is ending the conversation's
    /// file transfers and user streams; a new one waits for nothing and is
    /// refused.
    ConversationBusy,
    /// The device is neither the conversation's primary Host nor attached to
    /// it.
    HostNotAttached,
    /// The Cloud is stopped, and a user stream or a call like one never
    /// wakes it.
    HostStopped,
    /// The Cloud cannot start, or it is changing state and admits no
    /// operation now.
    CloudUnavailable,
    /// Another reset holds the Cloud. An operation waits for a reset; a
    /// second reset is refused.
    CloudResetting,
    /// Every Cloud capacity permit of the backend is taken, so the Cloud
    /// cannot start now; nothing queues for a permit.
    CloudCapacity,
    /// The Cloud's runtime was lost too often in a short time, so it no
    /// longer starts by itself; a reset recovers it.
    CloudCrashLoop,
    /// The Host's filesystem refused the operation: nothing at the path, or
    /// no permission for it.
    FsError,
    /// The file is no longer the version the request named.
    FileChanged,
    /// A directory is at the path a file upload names.
    IsDirectory,
    /// A file is at the path, and the upload did not ask to replace it.
    FileExists,
    /// The path is, or holds, a directory the Host needs: its root, its home
    /// or the conversation's execution directory.
    ProtectedPath,
    /// The user's browser sent nothing of an upload for a minute.
    TransferStalled,
    /// A file is over the 8 MiB the product shows as text or keeps as an
    /// edit.
    FileTooLarge,
    /// The file is not UTF-8 text, or holds a NUL byte.
    NotText,
    /// A directory has too many entries to list in one runner message.
    DirectoryTooLarge,
    /// The working tree could not be listed within the runner's time.
    ChangesTimeout,
    /// The directory is not inside a git repository.
    NotRepository,
    /// The working tree could not be read.
    ChangesFailed,
    /// The Host failed the operation in a way no other code names.
    HostOperationFailed,
    /// The request could act with the user's session and came from a page
    /// that is not the product's (`backend.md` § Authentication and
    /// ownership).
    ForbiddenOrigin,
    /// No user stream has that name.
    UnknownStream,
    /// A user stream is a WebSocket, and the request is not an upgrade.
    UpgradeRequired,
    /// The Host could not open the stream: its service failed to start or
    /// refused it.
    StreamFailed,
    /// Another user's conversation holds the id, or a Fork reserved it.
    IdUnavailable,
    /// A read acknowledgement names output the conversation does not have
    /// yet.
    InvalidRevision,
    /// A conversation socket's frame does not match its schema; the socket
    /// stays open.
    InvalidFrame,
    /// The backend could not prepare a conversation socket's frame for its
    /// session, such as a storage failure; the frames behind it still
    /// arrive.
    FrameDeliveryFailed,
    /// Work of the conversation's tree is running, or another transition
    /// holds the conversation: an archive or a target change waits for
    /// nothing and is refused.
    TurnInFlight,
    /// The caller owns no workspace of that id.
    WorkspaceNotFound,
    /// Conversations still target the workspace, which therefore stays.
    WorkspaceInUse,
    /// A sidebar move leaves its row's project or pin partition.
    InvalidOrder,
    /// No account has that id.
    UserNotFound,
    /// The caller has no subagent profile of that id.
    ProfileNotFound,
    /// Another of the caller's subagent profiles has the name.
    ProfileExists,
    /// The backend has no plugin of that id.
    UnknownPlugin,
    /// The plugin has no page method of that name for the route's scope.
    UnknownPluginMethod,
    /// The user has the plugin off.
    PluginDisabled,
    /// The plugin refused the call; `reason` is its own word for why.
    PluginRefused,
    /// The plugin failed, such as a value that does not read or a package
    /// call whose operation failed.
    PluginFailed,
    /// The device is the conversation's primary Host, which is never attached
    /// as well.
    HostIsPrimary,
    /// Another attached host of the conversation has that name.
    NameTaken,
    /// Another target change of the conversation came first.
    TargetConflict,
    /// One field of a conversation patch failed in a way the request could
    /// not cause; the fields applied stay applied.
    OperationFailed,
    /// The Fork's id belongs to another creation attempt: another source or
    /// another text.
    ForkConflict,
    /// The Fork's text is not a completed assistant text of the source's
    /// history.
    InvalidForkTarget,
    /// The conversation has no message with text to title.
    NoMessages,
    /// The caller has no upload of that id.
    UploadNotFound,
    /// The draft's replaced version is not the one the request names any
    /// more: a save or another page's action changed it.
    DraftChanged,
}

serde_plain::derive_display_from_serialize!(ErrorCode);
