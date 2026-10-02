package webapi

// A JSON error answer: `{ code, message }`, and a plugin's own `reason`
// when a plugin refused the call.
// +demi:root direction=receive output=web
// +demi:tolerant
type ErrorBody struct {
	Code ErrorCode `json:"code"`
	// Says what happened in words; for a refused body it names the field and
	// the reason.
	Message string `json:"message"`
	// With `plugin_refused`, the plugin's snake_case word for the refusal,
	// such as `tab_not_found`.
	Reason *string `json:"reason,omitempty"`
}

// Every error code the web app can see. A situation has one code on every
// route (`web-api.md` § Resource index); the HTTP status belongs to the
// route that answers.
// +demi:enum unauthenticated not_found invalid_body too_large internal_error backend_closing already_set_up invalid_credentials too_many_attempts email_taken invalid_code mail_unavailable mail_failed invalid_query forbidden provider_not_found model_not_found setting_unavailable model_not_selected provider_busy provider_exists unknown_provider_type unknown_vendor subscription_only accounts_unsupported no_login_flow login_not_found account_not_found active_account token_import_failed quota_requires_inference quota_unavailable catalog_unavailable provider_status_failed rate_limited device_not_found device_in_use device_offline log_unreadable conversation_not_found conversation_archived conversation_busy host_not_attached host_stopped cloud_unavailable cloud_resetting cloud_capacity cloud_crash_loop fs_error file_changed is_directory file_exists protected_path transfer_stalled file_too_large not_text directory_too_large changes_timeout not_repository changes_failed host_operation_failed forbidden_origin unknown_stream upgrade_required stream_failed id_unavailable invalid_revision invalid_frame frame_delivery_failed turn_in_flight workspace_not_found workspace_in_use invalid_order user_not_found unknown_plugin unknown_plugin_method plugin_disabled plugin_refused plugin_failed host_is_main name_taken target_conflict operation_failed fork_conflict invalid_fork_target no_messages upload_not_found draft_changed
type ErrorCode string

// Values of the preceding enumeration.
const (
	// The request carries no live session.
	ErrorCodeUnauthenticated ErrorCode = "unauthenticated"
	// No route answers the method and path, or the path names nothing the
	// caller holds, such as a blob outside the caller's namespace.
	ErrorCodeNotFound ErrorCode = "not_found"
	// A JSON body does not match its request type.
	ErrorCodeInvalidBody ErrorCode = "invalid_body"
	// A body the backend reads whole is over its limit.
	ErrorCodeTooLarge ErrorCode = "too_large"
	// The backend failed in a way the request could not cause.
	ErrorCodeInternalError ErrorCode = "internal_error"
	// The backend is shutting down and starts no new work.
	ErrorCodeBackendClosing ErrorCode = "backend_closing"
	// Setup has created the master account already.
	ErrorCodeAlreadySetUp ErrorCode = "already_set_up"
	// The email address and password, or the current password, do not match.
	ErrorCodeInvalidCredentials ErrorCode = "invalid_credentials"
	// Too many attempts for now: five failed logins for one address within
	// a minute of each other, or a new verification code within a minute of
	// the last one.
	ErrorCodeTooManyAttempts ErrorCode = "too_many_attempts"
	// Another account, or the caller's own, has the email address.
	ErrorCodeEmailTaken ErrorCode = "email_taken"
	// The verification code is wrong, expired or used up, or its challenge
	// no longer holds; or no runner waits with the pairing code.
	ErrorCodeInvalidCode ErrorCode = "invalid_code"
	// The backend has no account mail sender to deliver a verification code.
	ErrorCodeMailUnavailable ErrorCode = "mail_unavailable"
	// The verification mail could not be delivered.
	ErrorCodeMailFailed ErrorCode = "mail_failed"
	// A query parameter is not one the route reads, such as `refresh=1`.
	ErrorCodeInvalidQuery ErrorCode = "invalid_query"
	// The caller's role does not allow the operation, such as a user who
	// only infers configuring a shared instance's providers.
	ErrorCodeForbidden ErrorCode = "forbidden"
	// The provider entry does not exist in the caller's scope.
	ErrorCodeProviderNotFound ErrorCode = "provider_not_found"
	// The provider entry's catalog does not list the model.
	ErrorCodeModelNotFound ErrorCode = "model_not_found"
	// The conversation's model does not offer the thinking effort or the
	// service tier.
	ErrorCodeSettingUnavailable ErrorCode = "setting_unavailable"
	// The conversation has no model yet: it cannot be opened, titled, or
	// given an effort or a tier.
	ErrorCodeModelNotSelected ErrorCode = "model_not_selected"
	// Another change of the entry is still running, such as a device login
	// into it.
	ErrorCodeProviderBusy ErrorCode = "provider_busy"
	// The scope already holds its one subscription entry of the family.
	ErrorCodeProviderExists ErrorCode = "provider_exists"
	// No family has that name.
	ErrorCodeUnknownProviderType ErrorCode = "unknown_provider_type"
	// models.dev lists no vendor of that id that a family speaks to.
	ErrorCodeUnknownVendor ErrorCode = "unknown_vendor"
	// A subscription family is created by its login, and its entry takes
	// only a new label.
	ErrorCodeSubscriptionOnly ErrorCode = "subscription_only"
	// The entry does not take accounts this way: an API-key entry has none,
	// and a family adds accounts by device login or by a token, not both.
	ErrorCodeAccountsUnsupported ErrorCode = "accounts_unsupported"
	// The family has no device login.
	ErrorCodeNoLoginFlow ErrorCode = "no_login_flow"
	// No login of the caller's has that id, or its result was dropped ten
	// minutes after it finished.
	ErrorCodeLoginNotFound ErrorCode = "login_not_found"
	// The entry holds no account of that id.
	ErrorCodeAccountNotFound ErrorCode = "account_not_found"
	// The active account cannot be removed: select another first, or delete
	// the provider.
	ErrorCodeActiveAccount ErrorCode = "active_account"
	// The supplied setup token is not an account of the family; the message
	// never repeats it.
	ErrorCodeTokenImportFailed ErrorCode = "token_import_failed"
	// Reading the provider's usage would spend an inference request.
	ErrorCodeQuotaRequiresInference ErrorCode = "quota_requires_inference"
	// The vendor's usage endpoint did not answer, refused, or answered what
	// cannot be read.
	ErrorCodeQuotaUnavailable ErrorCode = "quota_unavailable"
	// The vendor list could not be read from models.dev.
	ErrorCodeCatalogUnavailable ErrorCode = "catalog_unavailable"
	// The entry's provider could not be built or read, which the sync
	// state shows as the entry's `failed` details.
	ErrorCodeProviderStatusFailed ErrorCode = "provider_status_failed"
	// Too many pairing codes tried: ten claims per user within a minute.
	ErrorCodeRateLimited ErrorCode = "rate_limited"
	// The caller owns no device of that id, or not one the route takes,
	// such as the Cloud for revocation.
	ErrorCodeDeviceNotFound ErrorCode = "device_not_found"
	// Workspaces still point at the device, which therefore stays.
	ErrorCodeDeviceInUse ErrorCode = "device_in_use"
	// The device's runner is not connected.
	ErrorCodeDeviceOffline ErrorCode = "device_offline"
	// The Host's log could not be read.
	ErrorCodeLogUnreadable ErrorCode = "log_unreadable"
	// The caller owns no conversation of that id.
	ErrorCodeConversationNotFound ErrorCode = "conversation_not_found"
	// The conversation is archived: restore it first.
	ErrorCodeConversationArchived ErrorCode = "conversation_archived"
	// An archive, a target change or a detach is ending the conversation's
	// file transfers and user streams; a new one waits for nothing and is
	// refused.
	ErrorCodeConversationBusy ErrorCode = "conversation_busy"
	// The device is neither the conversation's main Host nor attached to
	// it.
	ErrorCodeHostNotAttached ErrorCode = "host_not_attached"
	// The Cloud is stopped, and a user stream or a call like one never
	// wakes it.
	ErrorCodeHostStopped ErrorCode = "host_stopped"
	// The Cloud cannot start, or it is changing state and admits no
	// operation now.
	ErrorCodeCloudUnavailable ErrorCode = "cloud_unavailable"
	// Another reset holds the Cloud. An operation waits for a reset; a
	// second reset is refused.
	ErrorCodeCloudResetting ErrorCode = "cloud_resetting"
	// Every Cloud capacity permit of the backend is taken, so the Cloud
	// cannot start now; nothing queues for a permit.
	ErrorCodeCloudCapacity ErrorCode = "cloud_capacity"
	// The Cloud's runtime was lost too often in a short time, so it no
	// longer starts by itself; a reset recovers it.
	ErrorCodeCloudCrashLoop ErrorCode = "cloud_crash_loop"
	// The Host's filesystem refused the operation: nothing at the path, or
	// no permission for it.
	ErrorCodeFsError ErrorCode = "fs_error"
	// The file is no longer the version the request named.
	ErrorCodeFileChanged ErrorCode = "file_changed"
	// A directory is at the path a file upload names.
	ErrorCodeIsDirectory ErrorCode = "is_directory"
	// A file is at the path, and the upload did not ask to replace it.
	ErrorCodeFileExists ErrorCode = "file_exists"
	// The path is, or holds, a directory the Host needs: its root, its home
	// or the conversation's execution directory.
	ErrorCodeProtectedPath ErrorCode = "protected_path"
	// The user's browser sent nothing of an upload for a minute.
	ErrorCodeTransferStalled ErrorCode = "transfer_stalled"
	// A file is over the 8 MiB the product shows as text or keeps as an
	// edit.
	ErrorCodeFileTooLarge ErrorCode = "file_too_large"
	// The file is not UTF-8 text, or holds a NUL byte.
	ErrorCodeNotText ErrorCode = "not_text"
	// A directory has too many entries to list in one runner message.
	ErrorCodeDirectoryTooLarge ErrorCode = "directory_too_large"
	// The working tree could not be listed within the runner's time.
	ErrorCodeChangesTimeout ErrorCode = "changes_timeout"
	// The directory is not inside a git repository.
	ErrorCodeNotRepository ErrorCode = "not_repository"
	// The working tree could not be read.
	ErrorCodeChangesFailed ErrorCode = "changes_failed"
	// The Host failed the operation in a way no other code names.
	ErrorCodeHostOperationFailed ErrorCode = "host_operation_failed"
	// The request could act with the user's session and came from a page
	// that is not the product's (`backend.md` § Authentication and
	// ownership).
	ErrorCodeForbiddenOrigin ErrorCode = "forbidden_origin"
	// No user stream has that name.
	ErrorCodeUnknownStream ErrorCode = "unknown_stream"
	// A user stream is a WebSocket, and the request is not an upgrade.
	ErrorCodeUpgradeRequired ErrorCode = "upgrade_required"
	// The Host could not open the stream: its service failed to start or
	// refused it.
	ErrorCodeStreamFailed ErrorCode = "stream_failed"
	// Another user's conversation holds the id, or a Fork reserved it.
	ErrorCodeIDUnavailable ErrorCode = "id_unavailable"
	// A read acknowledgement names output the conversation does not have
	// yet.
	ErrorCodeInvalidRevision ErrorCode = "invalid_revision"
	// A conversation socket's frame does not match its schema; the socket
	// stays open.
	ErrorCodeInvalidFrame ErrorCode = "invalid_frame"
	// The backend could not prepare a conversation socket's frame for its
	// session, such as a storage failure; the frames behind it still
	// arrive.
	ErrorCodeFrameDeliveryFailed ErrorCode = "frame_delivery_failed"
	// Work of the conversation's tree is running, or another transition
	// holds the conversation: an archive or a target change waits for
	// nothing and is refused.
	ErrorCodeTurnInFlight ErrorCode = "turn_in_flight"
	// The caller owns no workspace of that id.
	ErrorCodeWorkspaceNotFound ErrorCode = "workspace_not_found"
	// Conversations still target the workspace, which therefore stays.
	ErrorCodeWorkspaceInUse ErrorCode = "workspace_in_use"
	// A sidebar move leaves its row's project or pin partition.
	ErrorCodeInvalidOrder ErrorCode = "invalid_order"
	// No account has that id.
	ErrorCodeUserNotFound ErrorCode = "user_not_found"
	// The backend has no plugin of that id.
	ErrorCodeUnknownPlugin ErrorCode = "unknown_plugin"
	// The plugin has no page method of that name for the route's scope.
	ErrorCodeUnknownPluginMethod ErrorCode = "unknown_plugin_method"
	// The user has the plugin off.
	ErrorCodePluginDisabled ErrorCode = "plugin_disabled"
	// The plugin refused the call; `reason` is its own word for why.
	ErrorCodePluginRefused ErrorCode = "plugin_refused"
	// The plugin failed, such as a value that does not read or a package
	// call whose operation failed.
	ErrorCodePluginFailed ErrorCode = "plugin_failed"
	// The device is the conversation's main Host, which is never attached
	// as well.
	ErrorCodeHostIsMain ErrorCode = "host_is_main"
	// Another attached host of the conversation has that name.
	ErrorCodeNameTaken ErrorCode = "name_taken"
	// Another target change of the conversation came first.
	ErrorCodeTargetConflict ErrorCode = "target_conflict"
	// One field of a conversation patch failed in a way the request could
	// not cause; the fields applied stay applied.
	ErrorCodeOperationFailed ErrorCode = "operation_failed"
	// The Fork's id belongs to another creation attempt: another source or
	// another text.
	ErrorCodeForkConflict ErrorCode = "fork_conflict"
	// The Fork's text is not a completed assistant text of the source's
	// history.
	ErrorCodeInvalidForkTarget ErrorCode = "invalid_fork_target"
	// The conversation has no message with text to title.
	ErrorCodeNoMessages ErrorCode = "no_messages"
	// The caller has no upload of that id.
	ErrorCodeUploadNotFound ErrorCode = "upload_not_found"
	// The draft's replaced version is not the one the request names any
	// more: a save or another page's action changed it.
	ErrorCodeDraftChanged ErrorCode = "draft_changed"
)
