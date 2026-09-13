# Web application

`packages/web` is Demi's authenticated browser client. It owns navigation,
account state, and backend adapters. `packages/web-ui` owns reusable rendering
and interaction; `packages/web-gallery` demonstrates the same components with
fixtures. The browser never imports backend implementations or concrete model
providers. [Package boundaries](../package-boundaries.md) defines dependencies.

Vue 3 and TypeScript render the application, vue-router owns URLs, Pinia holds
application state, and Vite builds it. Tailwind compiles the shared theme. Run
`bun run web:dev` after starting the backend; the development UI is at
`http://127.0.0.1:18934`. `bun run web:build` writes `packages/web/dist`.

## State and connections

The composition root, `web/main.ts`, assembles the router and account stores.
REST supplies durable metadata and initial history. The agent WebSocket supplies
live transcript, queue, child-agent, and shell-job events.

```text
Backend REST ----------> account snapshot and conversation metadata
Backend agent stream --> cached conversation runtime --> shared ChatSession
Browser IndexedDB -----> drafts and unconfirmed operations -----^
User actions ----------> product handlers -----------> backend APIs
```

`state/product.ts` polls conditional `/api/state` snapshots every three seconds
and refreshes when the page becomes visible. It reconciles conversations,
projects, devices, account information, and preferences. `api/contracts.ts`
validates REST data; `api/transcript.ts` validates history and agent frames.
Polling updates metadata without replacing live transcript state.

`conversation/store.ts` uses `web-ui`'s `ConversationCache`. Opening a persisted
conversation loads its hosts and history while model discovery runs independently.
History becomes readable as soon as it arrives. Model loading and the initial
agent handshake do not put that history behind a loading pane. An archived
conversation or one without a usable model can display history without a runtime.

Each opened conversation retains its state, pending initialization, and runtime
for the signed-in page lifetime. Concurrent opens share initialization. Switching
from A to B and back reuses A's history and healthy socket; inactive cached
conversations continue receiving events. Model preparation always uses that
runtime's conversation, not whichever conversation is visible.

Context and archive changes invalidate the affected cache entry. The visible
entry reloads immediately; other entries reload when opened. Removal, discarded
empty drafts, initialization failure, explicit Retry, and account cleanup release
requests and runtimes. Releasing a runtime closes the browser attachment without
aborting the backend task. Reloading the browser starts a new cache.

A dropped connection retries once automatically. Failure preserves available
history and offers Retry. A server takeover releases the attachment without two
browser windows repeatedly reconnecting over each other. Explicitly selecting a
cached conversation can reopen a taken-over socket and reconcile through the
agent protocol without repeating REST history loading. Shared loading and failure
presentation belongs to [Session interface](session-interface.md).

Model discovery uses one account-wide catalog, a one-minute browser TTL, and one
pending request. Device availability is applied separately for the selected
conversation. Background refresh retains the existing catalog on failure.
[Model catalog caching](../model-catalog-cache.md) defines backend persistence.

## Drafts, sending, and recovery

New conversation allocates a local UUID and opens the composer without HTTP.
Repeated New reuses an empty draft for that project. Leaving an empty draft
removes it; nonempty drafts are saved per user in IndexedDB.

The first Send retains the local conversation even if creation fails. The product
creates the backend record with that UUID, applies the target and model, uploads
staged workspace files, and submits the message. Repeating creation with the same
UUID returns the owner's record; it cannot claim another user's ID.

The product keeps an unconfirmed submission separately from the next composer
draft: message ID, content, and attachment IDs. `PendingSubmission` shows the
pending message, failure, and Retry. Retry reuses the ID. The agent client checks
history and queue state before resubmitting, and the backend deduplicates admitted
IDs. Confirmation clears only that submission and its attachments.

Admission is different from completion. A later model failure resumes the
accepted turn rather than submitting the user message again. Retry and Resume
both call the runtime's `resume()` and preserve completed tool effects.

Only the last explicit user message can be edited. `SessionComposer` displays its
original content and attachments; `MessageEditRegion` makes the affected suffix
translucent and inert. Cancel or Escape restores the unrelated draft. Enter saves,
Shift+Enter inserts a newline, and IME composition never submits. Saving locks the
edit. Uncertain confirmation retains the immutable operation ID for Retry, also
after reload. [Message editing](../message-editing.md) defines durability and
conflicts; `web` supplies authenticated media and API adapters.

The assistant footer offers copy, a timestamp, and Fork at eligible final text
blocks. The product's fork handler creates and opens a new conversation through
the backend. [Message forking](../conversation-fork.md) defines which history is
copied and how operation retries work.

## Persistence boundaries

| Data | Owner |
|---|---|
| Transcript, queue, tree, and model selection of a saved conversation | Backend conversation and agent stores |
| Pins, ordering, read revisions, and archive state | Backend sidebar APIs |
| Appearance, shortcuts, and last explicit model choice | Backend per-user preferences |
| Draft text, message edits, unconfirmed sends, attachment bytes, and local conversation state | Per-user IndexedDB in `conversation/drafts.ts` |
| Project folds, recent projects, and hidden providers or models | Per-user browser state in `state/local.ts` |
| Opened runtimes and their pending loads | Page-lifetime `ConversationCache` |

Preference writes overlay pending fields until persistence and refresh finish.
New conversations copy the saved model, thinking effort, and service tier. Without
a saved choice they choose the first available model. An explicit unavailable
choice remains selected with a warning and blocks Send until changed. With no
usable model, the composer offers configuration only to authorized users.

## Authentication and account operations

The application mounts a loading region while checking `/api/auth/me` before
routing to authenticated content. `/login` uses the shared email/password page.
Wrong credentials and rate limits come from the backend. A confirmed expired
session returns to sign-in. A temporary restore failure preserves an established
identity and reports the error; an initial check failure offers sign-in with an
error. Logout calls the backend and loads a fresh document to release account
state. [Authentication](../web-authentication.md) owns the session boundary.

`SettingsDialog` supplies real state and handlers to shared settings pages.
Nickname changes and verified email/password changes use account APIs. Closing
a submitted account dialog leaves its operation running; reopening shows its
phase and retained input. Success clears temporary credentials. Failure after
closing also produces a toast. Account changes abort account-scoped activity.

Provider writes likewise survive panel closure but end on account cleanup.
Failed saves retain fields; model edits can be reopened with their inputs.
Provider device-login polling is cancelled when its dialog closes. Queries never
return saved credentials. Claude Code imports a setup-token; Codex and Grok use
device login. Permissions, accounts, model sources, and quota semantics belong to
[Providers](providers-and-vault.md), while endpoints belong to [Web API](web-api.md).

An unconfigured provider can remain a local form draft. A manual endpoint is
submitted only after credentials and a complete model are present. Model limits
must be positive, and an output limit cannot exceed the context window. The last
persisted manual model cannot be removed individually; remove the provider or
switch its model source. Vendor defaults prefill editable endpoints.

Initial lists use `AsyncRegion` for loading, failure, and Retry. Background reads
retain content. Submitted controls preserve their size and accessible name,
prevent duplicate activation, and lock related fields. Conversation metadata and
target writes are guarded per conversation. A submitted row action shows pending
feedback on that row. No artificial minimum loading duration is imposed.

## Devices, targets, and attachments

Product adapters supply actual devices, paths, and requests to the shared host
menu, workspace dialog, and file browser. Choosing a project directory moves into
that project; another directory sets a direct device target. Main-target changes
require an idle, unarchived conversation. [Host menu](host-menu.md) defines the
selection interaction; [File browser](../file-browser.md) owns browsing behavior.

`devices/files.ts` adapts device filesystem endpoints, including directory creation,
home, sizes, and modification times. Remote references retain the device ID and
full path. Their record includes the host read command so the agent can read the
file at execution time; it does not embed a second copy of the file content.

`api/uploads.ts` reports byte-transfer progress. The selected model determines
which files are model media and which go to the workspace. Workspace files in an
unsent conversation remain staged until its first Send creates the backend record.
Failed uploads retain their tile and Retry; Send waits for readiness. A failed
first-send upload stays with its unconfirmed message and retries there.
Removing a file cancels its transfer. Changing target invalidates and repeats
workspace uploads for the new context. `ContentMedia` releases object URLs when
its source changes or it unmounts. [Product](product.md) defines attachment kinds.

Cloud status comes from account snapshots. Reset submits an operation ID and
follows backend phases; acceptance is not completion. Failed retries reuse that
ID. `CloudSettings` owns confirmation, progress, and retry interaction. The backend
owns waking, task cancellation, system replacement, and home preservation as
specified in [Managed hosts](managed-hosts.md).

## Implementation boundaries and verification

The application uses shared `ChatSession`, sidebar, menus, dialogs, settings, and
file controls. [Session interface](session-interface.md) defines transcript and
panel behavior; [Control layout](../control-layout.md) defines common appearance.
[Gallery synchronization](web-gallery-sync.md) defines acceptance in both consumers.

Cloud directory browsing is disabled in the product; workspace uploads and
execution can wake Cloud through the backend. The work panel has shared layout
and gallery examples, but file/diff viewers and product integration are deferred.
MCP, Skills, notifications, language switching, data/privacy actions, account
deletion, administration pages, public registration/recovery, Git branch display,
and interactive terminal input are not enabled. Deferred settings entries stay
disabled as specified in [Product](product.md).

Checks use mocked HTTP, scripted providers, disposable accounts, fake email, and
local runner fixtures, never real model calls. Verify draft/first-send recovery,
cached switching and takeover, loading failures, queue/stop/resume, editing/Fork,
partial batch failures, target selection, attachment cancellation/retry, and account
cleanup. Browser acceptance covers those flows plus narrow layouts and both theme
modes. Compilation uses `bun run typecheck:web` and `bun run web:build`.
