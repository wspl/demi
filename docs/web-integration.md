# Web product integration

`packages/web` is the authenticated backend client. `packages/web-ui` owns the
reusable interface and interaction; `packages/web-gallery` supplies fixtures for
those same components. The product no longer has a prototype store or reply clock.
Shared input sizing and the production CSS inspection matrix are documented in
[control layout](control-layout.md).

## State and transport

`web/main.ts` composes the router and account-scoped stores. `state/product.ts`
reads conditional `GET /api/state` snapshots every three seconds and refreshes
when the page becomes visible. It reconciles account, conversation, project,
device and preference metadata. `api/contracts.ts` validates REST responses;
`api/transcript.ts` validates persisted blocks and incoming agent frames.

`conversation/store.ts` retains local drafts and loaded transcripts while
reconciling server metadata. New conversation allocates a UUID locally and opens
the composer without an HTTP request. Repeated New reuses an empty draft for the
same project. Leaving an empty draft discards it; drafts containing text or files
are restored from per-user IndexedDB storage.

The first Send makes the conversation permanent locally, including when the
network request fails. It creates the server record with
`POST /api/conversations { id: UUID }`, applies the chosen target and model, and
submits the message. Repeating creation with the same UUID returns the owner's
existing record without resetting it. Another account cannot claim that UUID.
Only the visible conversation owns a live
`ConversationRuntime`. Opening it reads its transcript, descendant histories and
attached hosts, then attaches to `/api/conversations/:id/stream`. Leaving the
page closes the client transport without sending a task-close or abort command.
The backend task continues. A dropped connection retries once automatically;
a failed reconnect exposes the shared Retry action. Server takeover closes the
attachment without an automatic reconnect fight between browser windows.

`AgentClient.submit` confirms a send when its message ID appears in the transcript
or queue. The product stores the submitted text, attachment IDs and message ID
until confirmation, separately from the next composer draft. An unconfirmed send
remains visible in `web-ui/agent/PendingSubmission.vue` with its error and Retry.
Retry reuses the message ID. `ConversationRuntime` checks restored history and
the queue before resubmitting; `AgentSession` ignores IDs already active, queued
or present in its transcript. Confirmation clears only that submitted message
and its attachments. This is admission confirmation, not turn completion:
a later model failure exposes Retry without duplicating the accepted user message.
The visible Retry/Resume recovery action calls `resume`, preserving completed
tool effects rather than regenerating the latest turn.

`web-ui/agent/SessionComposer.vue` switches the existing composer into message
editing. It displays every original text part and attachment while
`MessageEditRegion.vue` makes the target message and its suffix translucent and
inert to mouse and keyboard interaction. The × beside the send button or Escape exits editing
and restores the unrelated composer draft. Enter saves and resends; Shift+Enter
inserts a newline and IME composition never submits. `message-editing.ts` owns the
editing, sending and uncertain-confirmation phases. The product stores the edit
separately in per-user IndexedDB and hydrates media references through
authenticated blob requests before submission. Saving locks the edit; uncertain
confirmation keeps it immutable until Retry resolves the original operation ID.
A reload restores an interrupted submission as uncertain.
`AgentClient.editAndSend` waits for the correlated acceptance frame. The backend
commits the retained prefix and replacement before publishing it, then starts
inference with an independent provider runtime. See
[message editing](message-editing.md) for conflict, recovery and durability rules.

`web-ui/agent/ChatSession.vue` owns the title, transcript, dock and inspection
panels. `useSessionPanels` selects one child agent or terminal at a time. Child
phases and timestamps come from agent lifecycle frames and persisted tree nodes.
Terminal output comes from shell frames and stored tool views. Inspection is
read-only; the child panel's Stop action stops all children supported by the
agent protocol.

## Persistence and operations

| Data | Owner and behavior |
| --- | --- |
| Transcript, queue and pending steers | Backend agent session; reconnect restores the current session snapshot. Pending-steer recovery after process restart is outside this change. |
| Pins, ordering, read revisions and archive state | Backend conversation/sidebar APIs. A batch keeps successful changes and reports only failures. |
| Appearance and keyboard shortcuts | Backend per-field preference API. Local pending changes are overlaid until the write and refresh finish. |
| Local conversation metadata, draft text, message edits, unconfirmed sends, file bytes, scroll state and last model | Per-user IndexedDB records in `conversation/drafts.ts`. Binary previews are recreated, and unfinished uploads restart. Empty unsent drafts are not stored. |
| Project folds, recent projects, hidden providers/models | Per-user browser storage in `state/local.ts`. Hiding affects selection menus, not a running task. |

New conversations select the first available model. An explicit previous choice
that becomes unavailable stays selected with a warning; the product does not
silently switch it. With no usable model, the composer offers configuration only
to users with permission. Catalog selections carry full model metadata, thinking
configuration and service tier to the agent.

## Providers and accounts

`settings/providers.ts` owns account-scoped provider drafts and API operations.
`settings/ProvidersPanel.vue` passes this state to `SettingsProvidersPage`.
The provider page selects the first subscription or API entry when no valid selection
exists. OpenAI API and Anthropic API appear as unconfigured local drafts when
absent; entering a key creates the backend provider. Their protocol and endpoint
come from the vendor catalog. The Add provider dialog fixes its title and search
field above a separately scrolling results list. API providers support creation, credential replacement, metadata changes,
explicit connection tests, catalog refresh and deletion. A manual endpoint stays
a local draft until it has credentials and at least one complete model.
Manual-model validation requires a positive context window and an optional
positive output limit no greater than the context window. Supported media are
images, video formats and PDF. Save errors retain the model dialog's input.
The final persisted manual model cannot be removed individually; the provider
can be deleted or switched back to its catalog.

The backend vendor catalog fills omitted OpenAI and Anthropic endpoints from the
corresponding provider runtime constants. The product and Gallery prefill these
addresses for default API entries, vendor additions and bare protocol additions;
users can edit the address before configuring the entry.

Claude Code imports a setup-token. Codex and Grok use device login, including
polling and cancellation. Account lists support adding, switching and removing
credentials under backend permissions. Shared mode exposes configuration only
to administrators. Queries never return credential material; configured keys
are replaced through an empty secret field. Quota rows show actual reported
windows only. Explicit refresh probes quota only when supported and free.

`settings/SettingsDialog.vue` connects nickname, verified email change and
password change to the account APIs. New accounts have no nickname; shared account
headers display the email until a nickname is set, and avatars use its first character. Email delivery remains the backend's
injected `accountMail` adapter. Closing an email or password dialog leaves a
submitted request running. Reopening shows its current phase and retained input;
success clears temporary credentials. A failure after closing also produces a
toast. Provider device-login polling is cancelled when its dialog closes.
Authentication expiry aborts
account-scoped activity and returns to sign-in. See [authentication](web-authentication.md).

## Loading and submitted operations

`state/product.ts` distinguishes an initial catalog read from an empty catalog.
Concurrent vendor reads share a request; subsequent opens reuse the account's
cached catalog. Background model and state refreshes retain existing content,
including after a failed refresh. `web-ui/ui/AsyncRegion.vue` supplies initial
loading, failure and retry presentation for provider, device, archive and project
lists. `ModelSelector` displays loading or retry before reporting model availability.
The app mounts before its startup session check finishes and shows a shared
loading region until routing is ready.

Provider draft IDs and subscription order survive closing and reopening settings.
Provider and device writes live in account-scoped stores, so closing a panel does
not cancel a submitted write. Account changes abort those operations. Provider
configuration failures retain unsaved fields with Retry save. The product retains
`SettingsModelEditor` data across panel unmounts; Review model reopens a failed
submission with its inputs. Display-name autosave reports Saving, Saved or Not
saved and allows retry.

`Button` and `IconButton` expose `loading`, keep their dimensions and accessible
names, and suppress repeated activation. Form submissions use these controls;
related inputs are locked while the request runs. Conversation metadata and host
changes are guarded per conversation. Archive, restore, provider-account and
device-revoke actions expose pending feedback on the affected row or control.
Folder creation uses one idle/naming/saving phase and releases it on completion,
failure or cancellation. No minimum loading duration is imposed.

The Gallery's Control layout page includes `GalleryLoadingStates` examples for
initial reads, cached content, failure/retry and submitted controls.

## Devices, Cloud and files

`devices/pairing.ts` claims actual pairing codes. `devices/files.ts` adapts
`GET/POST /api/devices/:id/fs` to the shared file browser, including directory
creation, home, size and modification time. `WorkspaceDirectoryMenu` and
`RemoteFilePicker` own browsing interaction. Product handlers supply device
sources and submit target changes or file references. Choosing an existing
project directory moves into that project; another directory sets a direct
device target. Main-target changes require an idle, unarchived conversation.

`api/uploads.ts` sends file bytes with XHR transfer progress. The selected model's
accepted extensions distinguish model attachments from workspace files. Upload
failures retain a tile with Retry; Send remains blocked until uploads are ready.
Workspace files in a local conversation are staged without a request and upload
after its first Send creates the server record. A failed first-send upload stays
with the unconfirmed message and retries through that message's Retry action.
Removing a file cancels its transfer. A target change invalidates and repeats
workspace uploads against the new context. Remote references retain device ID
and path in a file reference; the backend includes the existing host read command
in that reference so the agent reads the device at execution time.
`ContentMedia.vue` renders images, video and document links and releases object
URLs when a source changes or the component unmounts.

Cloud status and limits come from the user's state snapshot. Reset submits the
operation ID through `/api/cloud/reset` and follows the reported operation state;
acceptance alone does not imply completion. Failed reset retries use the same
operation ID. The backend owns Cloud wake, isolation and home preservation.

## Deliberately deferred

The installation instructions remain a mock. Cloud directory browsing, MCP,
Skills, notifications, language switching, data/privacy actions, account deletion,
public registration/recovery, administration pages, Git branch display and manual
terminal input are not enabled. Their existing deferred entry points remain
hidden or disabled. No deployment or production email service was provisioned.

## Validation

Tests use scripted providers, fake email delivery, mocked HTTP and local runner
fixtures; they never call real models. Coverage includes message admission,
disconnect/cancellation, subagent history, metadata reconciliation, partial batch
failure, read acknowledgement, file and provider contracts, and shared UI helpers.
Browser verification uses a disposable backend account and device. Exact check
results are recorded in [progress](demi-next/progress.md).
