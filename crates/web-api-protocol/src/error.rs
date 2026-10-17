//! The body of every JSON error, and the one list of the codes it carries.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_shared_types::RequestId;

use crate::ids::LoginId;
use crate::passkeys::PasskeyId;

/// A JSON error answer: `{ code, message, params?, requestId }`, and a
/// plugin's own `reason` when a plugin refused the call. The page words the
/// answer from `code` and `params`; `message` is English, for the server
/// log, API clients and a code the page has no words for
/// (`localization.md` § Text from the backend).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    pub code: ErrorCode,
    /// Says what happened in words; for a refused body it names the field and
    /// the reason.
    pub message: String,
    /// The values the page's text for `code` needs, such as `{ limit: 50 }`,
    /// each a number or a string, and the same for every answer of the code.
    /// A name says how the page writes its value: one ending in `Bytes` is a
    /// size, `limitBytes`, and `count` chooses a plural form.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, serde_json::Value>,
    /// With `plugin_refused`, the plugin's snake_case word for the refusal,
    /// such as `tab_not_found`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// With `unknown_passkey`, the passkey the browser signed with, which
    /// the page tells the browser no account has.
    #[serde(default, rename = "credentialId", skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<PasskeyId>,
    /// With `provider_busy` for a login start, the starter's own login that
    /// holds the entry, which the page opens rather than a second one.
    #[serde(default, rename = "loginId", skip_serializing_if = "Option::is_none")]
    pub login_id: Option<LoginId>,
    /// With `workspace_exists`, the user's workspace of the device and
    /// directory, which the page opens rather than a second one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<crate::workspaces::WorkspaceDto>,
    /// The request the answer is of, as its `X-Request-Id` names it.
    #[serde(rename = "requestId")]
    pub request_id: RequestId,
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
    /// The change does not fit the kind of API-key entry: a vendor entry's
    /// endpoint and protocol are its vendor's and are never set.
    EndpointKind,
    /// A model list names one model id twice: an entry's model ids are
    /// unique.
    DuplicateModel,
    /// The entry does not take accounts this way: an API-key entry has none.
    AccountsUnsupported,
    /// A rotation other than `manual` for an entry with fewer than two
    /// accounts.
    RotationNeedsAccounts,
    /// A media shape for a vendor entry, whose shapes are its vendor's.
    ShapeFixed,
    /// The family has no login.
    NoLoginFlow,
    /// No login of the caller's has that id, or its result was dropped ten
    /// minutes after it finished.
    LoginNotFound,
    /// The login waits for no code: it is not one that takes a pasted code,
    /// it has not shown its link yet, or it ended.
    LoginNotWaiting,
    /// The entry holds no account of that id.
    AccountNotFound,
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
    /// The device is the caller's Cloud, which keeps its name.
    DeviceManaged,
    /// The device's runner is not connected.
    DeviceOffline,
    /// The device is the caller's Cloud, which no direct channel reaches
    /// (`web-api-hosts.md` § Direct channel).
    NotAPairedDevice,
    /// The Host's log could not be read.
    LogUnreadable,
    /// The caller owns no conversation of that id.
    ConversationNotFound,
    /// The transcript does not hold the block a page read names.
    BlockNotFound,
    /// The block a page read extends from is no longer at its index: the
    /// transcript was rewritten since the page read it (`web-api-conversations.md`
    /// § Pages).
    TranscriptChanged,
    /// The conversation's database could not be read on the server, as one
    /// a build that does not know its schema made: the page says so in its
    /// own words, and the message, which names server paths, goes to its
    /// console only.
    ConversationUnreadable,
    /// The conversation is archived: restore it first.
    ConversationArchived,
    /// An answer's share names a block that is not the text that ends the
    /// agent's reply to a message (`web-api-conversations.md` § Sharing).
    ShareTargetInvalid,
    /// An answer's share is never taken again: its answer does not change.
    ShareNotUpdatable,
    /// No share answers the token, or the snapshot holds no such agent,
    /// block or blob: one answer for each, so it tells nothing of what
    /// exists.
    ShareNotFound,
    /// The conversation has no undecided permission request of that id:
    /// another page decided it, an Allow of its category did, or a newer
    /// request replaced it.
    PermissionRequestNotFound,
    /// The answer does not fit the request: a credential request is
    /// answered with a value for each of its fields and how long to keep
    /// it, and only a credential request is.
    InvalidAnswer,
    /// The caller has no credential of that id.
    CredentialNotFound,
    /// Another of the caller's credentials has the name.
    CredentialNameTaken,
    /// The caller has no port forward of that id or name.
    PortForwardNotFound,
    /// Another of the caller's port forwards has the name.
    PortForwardNameTaken,
    /// The caller has no exposed port of that id.
    ExposedPortNotFound,
    /// Another row of the device exposes the port.
    ExposedPortTaken,
    /// A credential of the requested name is saved with other fields: its
    /// new values would not fit them.
    CredentialFieldsDiffer,
    /// The credential the request asks to use was removed.
    CredentialRemoved,
    /// The passkey challenge expired or was used: the page asks for another.
    PasskeyExpired,
    /// No account has the passkey the browser signed with; `credentialId`
    /// names it.
    UnknownPasskey,
    /// The caller has no passkey of that id.
    PasskeyNotFound,
    /// The backend offers no passkeys: its public URL's host is an IP
    /// address.
    PasskeysUnavailable,
    /// Making a handoff code needs the session verified in the last five
    /// minutes, by password or passkey.
    VerificationRequired,
    /// The handoff code is unknown, expired or spent.
    HandoffExpired,
    /// The session to sign out is the caller's own, which Sign Out ends.
    CurrentSession,
    /// The account has no session of that id.
    SessionNotFound,
    /// The work panel holds as many tabs, or as much of their data, as it
    /// can (`web-api-conversations.md` § Work panel state).
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
    /// The directory a request names is not on the Host (`web-api-conversations.md`
    /// § Workflows of a directory).
    FsNotFound,
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
    /// The file is binary (`demi_command_protocol::is_binary`).
    NotText,
    /// A directory has too many entries to list in one runner message.
    DirectoryTooLarge,
    /// The working tree could not be listed within the runner's time.
    ChangesTimeout,
    /// The directory is not inside a git repository.
    NotRepository,
    /// A commit's abbreviation names more than one commit.
    Ambiguous,
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
    /// The user has a workspace of that device and directory already, which
    /// the answer carries as `workspace`.
    WorkspaceExists,
    /// The directory is the device's home, which holds the conversations
    /// outside projects and is never a project (`conversations.md`
    /// § Conversations and projects).
    WorkspaceIsHome,
    /// A move names a directory on a device whose runner never reported its
    /// home, so whether the directory is a project's is unknown
    /// (`sessions-and-targets.md` § Resolve a target).
    DeviceHomeUnknown,
    /// A sidebar move leaves its row's project or pin partition.
    InvalidOrder,
    /// No account has that id.
    UserNotFound,
    /// The account is disabled, which refuses every sign-in, by a password,
    /// a passkey, a handoff code or a sign-in code.
    AccountDisabled,
    /// The account's owner must choose a password before anything else of
    /// the product opens.
    PasswordChoiceRequired,
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
    /// The caller has no upload of that id.
    UploadNotFound,
    /// Another live session's push subscription has that endpoint.
    PushEndpointTaken,
    /// The draft's replaced version is not the one the request names any
    /// more: a save or another page's action changed it.
    DraftChanged,
    /// The conversation keeps as many messages saved for later as it may.
    SavedFull,
    /// The conversation has no message saved for later of that id.
    SavedNotFound,
    /// A message saved for later has the id already, with other content or in
    /// another conversation.
    IdTaken,
    /// The caller has no automation of that id.
    AutomationNotFound,
    /// A schedule's cron expression is not five fields cron reads, which
    /// `message` names.
    InvalidCron,
    /// The time zone is not one the backend knows, such as `Asia/Shanghai`.
    InvalidTimeZone,
    /// An automation's existing conversation is not one of the caller's
    /// active conversations.
    AutomationConversationUnavailable,
    /// A limit on runs a day is for a change watch or a URL, not a time.
    LimitNotTaken,
    /// The automation's trigger is not a URL, which alone has a secret.
    NotUrlTrigger,
    /// The conversation is not filed under an automation's run.
    NotFiled,
}

serde_plain::derive_display_from_serialize!(ErrorCode);

/// The words of a value that breaks its rules, as a refused body says
/// them: each field's path and what is wrong with it, joined by `; `.
pub fn problems(report: &garde::Report) -> String {
    let problems: Vec<String> = report
        .iter()
        .map(|(path, error)| format!("{path}: {error}"))
        .collect();
    problems.join("; ")
}
