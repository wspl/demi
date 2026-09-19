# Demi Next: Web API Reference

The browser and backend ship together, so routes have no version prefix.
Application data uses HTTP JSON; an open conversation uses the agent WebSocket
protocol. [Backend architecture](backend.md) defines authentication and transport
boundaries. This reference owns product endpoint contracts; domain rules remain
in the linked topic documents.

## Resource index

Paths in the index are relative to `/api`, except the public installation row.
Detailed sections use the same relative paths or spell out the `/api` prefix.
JSON errors carry `{ code, message }`; validation normally returns 400, missing
or inaccessible objects 404, and an operation refused by current state 409.
Partial conversation mutations use the explicit outcomes described below.

| Resource | Routes and request shape |
|---|---|
| Setup | `GET /setup` returns `{ needed }`; `POST /setup` creates the first master account and signs it in |
| Authentication | `POST /auth/login`, `POST /auth/logout`, `GET/PATCH /auth/me`, `PUT /auth/password`, `POST /auth/email`, `POST /auth/email/confirm` |
| Users | `GET/POST /users`, `PATCH /users/:id`; role hierarchy restricts administration |
| Application state | `GET /state`; conditional snapshot with private ETag |
| Settings | `GET /settings` returns fixed instance mode; `GET/PATCH /settings/preferences` |
| Conversations | `GET /conversations?archived=true\|false`, `POST /conversations { id }`, `PATCH /conversations/:id`, `POST /conversations/batch`, `POST /conversations/:id/fork { id, blockId }`, `POST /conversations/:id/read { revision }` |
| Conversation history | `GET /conversations/:id/transcript` returns root blocks and subagent histories, each with the [failure facts](backend.md#failure-facts) of its error blocks; `WS /conversations/:id/stream` carries agent frames |
| Conversation files | `GET/POST /conversations/:id/fs`, `GET /conversations/:id/fs/file?path=...`, `GET /conversations/:id/fs/raw?path=...&version=...&download=true\|false`, `GET/POST /conversations/:id/hosts/:deviceId/fs` |
| Working tree | `GET /conversations/:id/changes`, `GET /conversations/:id/changes/file?path=...`, `GET /conversations/:id/changes/raw?path=...&download=true\|false` |
| User streams | `WS /conversations/:id/streams/:name` opens a declared [user stream](#user-streams); `POST /conversations/:id/activity` reports a user operation |
| Sidebar | `POST /sidebar/reorder { kind, id, beforeId }` |
| Models | `GET /models?refresh=true\|false` returns the account-wide catalog |
| Providers | `GET /providers/catalog`, `GET/POST /providers`, `PATCH/DELETE /providers/:id`, `GET /providers/:id/status`, `POST /providers/:id/test`, `POST /providers/:id/quota`; account routes below |
| Usage | `GET /usage` for the caller; `GET /usage/instance` for admins in shared mode |
| Devices | `GET /devices`, `POST /devices/claim { code }`, `DELETE /devices/:id`, `GET /devices/:id/fs?path=<absolute>`, `POST /devices/:id/fs { path }` |
| Workspaces | `GET/POST /workspaces`, `PATCH /workspaces/:id { name }`, `DELETE /workspaces/:id` |
| Cloud | `GET /cloud`, `POST /cloud/reset { operationId }` |
| Exposes | `GET /exposes`, `POST /exposes { deviceId, address }`, `POST /exposes/:id/renew`, `DELETE /exposes/:id` |
| Attachments | `POST /attachments` with raw bytes; `GET /blobs/:sha256?type=...` |
| Attached hosts | `GET /conversations/:id/hosts`, `POST .../hosts { deviceId }`, `PATCH .../hosts/:deviceId { name }`, `DELETE .../hosts/:deviceId` |
| Runner transport | `WS /runner`; device-authenticated `PUT/GET /pipes/:id` for source/sink streams |
| Public installation | `GET /install.sh`, `GET /install.ps1`, `GET /runner-artifacts/:release/:target/:file` (root paths, outside `/api`) |

### Query parameters

A boolean query parameter (`archived`, `refresh`, `download`) is spelled exactly `true` or
`false`; omitted means false, and anything else — `1`, `TRUE`, an empty value —
returns 400 `invalid_query` rather than being read as one of them. A directory
in `GET /devices/:id/fs?path=` must be absolute on that device; omitted, the
listing starts at the device's home directory. `GET /blobs/:sha256?type=` is not
validated the same way: the parameter asks for a media type to render in place,
and a type the page does not show in place
([File previews](file-previews.md#keeping-file-content-inert)) simply leaves
the blob as a download.

## Conversation creation and Fork

`POST /conversations` requires a client-generated UUID. A new conversation uses
Cloud as its target without waking a machine. Retrying the same ID for the same
owner returns the existing record (200 instead of 201). An ID owned by another
user or reserved for a Fork returns 409 `id_unavailable`.

`POST /conversations/:id/fork` takes a destination UUID and an assistant block ID.
It copies a history boundary into a new conversation and returns `{ conversation,
model }`, with 201 on creation and 200 for a completed retry. It does not mutate
the source. The operation reserves the destination and records its metadata so
publication can recover after interruption. History boundary and command-state
semantics belong to [Conversation Fork](../conversation-fork.md).

## Workspaces, devices, and attached hosts

Workspace creation takes `{ deviceId, path, name }` or `{ cloud: true, name }`.
The first names a directory on a caller-owned device. The second creates a
project directory on the user's Cloud and can wake it. A workspace is a pointer;
renaming or deleting it does not rename or delete files. Deletion returns 409
`workspace_in_use` while conversations still target it.

Device revocation applies to user-paired devices, not the managed Cloud device.
It returns 409 `device_in_use` while workspaces point at that device. Successful
revocation closes its connection and removes its conversation attachments.
Pairing accepts a live code and returns the claimed device; expired/unknown codes
return 404 and excessive attempts 429. The installation routes contain no
credential and do not grant device access. The runner receives a pending code;
the signed-in browser claims it through
`POST /api/devices/claim`. Device tokens are delivered only to the runner.

Attached-host responses contain device identity, name, cwd, online state, and
attachment time. A conversation's main device cannot also be attached. Names are
unique within the conversation; a conflicting rename returns 409 `name_taken`.
Changes follow [Sessions and targets](sessions-and-targets.md).

## Uploads and media

`POST /attachments` accepts a nonempty raw request body with its media type in
`Content-Type`. The header must name one `type/subtype`, and `multipart/*` — a
form envelope rather than a file's bytes — is rejected. The maximum body is
25 MiB.
It returns `{ attachment }` with metadata and a reference ID. Uploading alone
stores a backend blob; sending `{ type: "upload", ref, fileName }` in a send/steer
frame resolves that upload and writes it to the selected Host's Demi attachment
directory. This can require an available execution target. Media handling and
failure behavior are defined in [Backend media handling](backend.md#media-by-reference).

## Cloud

`managed/` owns the unique managed device for each user, lazy allocation/wake,
shared-device admission and system reset. `conversation/` resolves target
selections and coordinates per-conversation switches. `storage/` enforces
managed-device uniqueness and records lifecycle intents; `managed/` publishes
consistent disk generations. Workspace creation ensures a directory on the user's device;
it does not allocate an independent VM.

The authenticated Web API exposes `GET /api/cloud` for logical device identity,
lifecycle state, disk usage/limits and pending operation, and
`POST /api/cloud/reset` with a validated `{ operationId }`. The backend selects
and records the shipped base version at admission. The response identifies the
operation; the status response reports stopping, saving, rebuilding, booting,
ready or failure. Retrying the same operation id returns the same operation.
A reset affects every job on the user's Cloud and preserves home. Requests
cannot name another user's device or arbitrary image paths. Settings use this
API even when the guest is offline or broken. Lifecycle behavior is defined in
[Managed hosts](managed-hosts.md).

## User streams

`WS /api/conversations/:id/streams/:name` opens the declared
[user stream](native-runtime.md#user-streams) `name` on the conversation's main
Host; `browser` is the [live browser view](browser-live-view.md). The upgrade
requires the session cookie, a conversation the user owns, and an `Origin` of
the product: the stream operates a browser signed in to the user's sites, so
an upgrade from any other origin answers 403 before the backend reaches the
Host. The route answers before the upgrade:

| Answer | When |
| --- | --- |
| 404 `unknown_stream` | No stream has that name |
| 409 `conversation_archived` | The conversation is archived |
| 409 `device_offline` | A paired device has no live runner |
| 409 `host_stopped` | The Cloud is stopped; a user stream never wakes it |
| 409 `conversation_busy` | An archive, a target change or a detach is ending the conversation's streams |
| 502 `stream_failed` | The Host could not open the stream: its service failed to start, or refused it |

Admission follows [Host operations](sessions-and-targets.md#host-operations).
After the upgrade, binary messages carry the stream's bytes both ways, in
order; message boundaries carry no meaning, since the stream frames its own
messages. The backend reads from the Host only as fast as the page takes the
bytes. It closes the socket when the stream ends, with a code and a reason:

| Code and reason | When |
| --- | --- |
| 1000 `completed` | The invocation completed |
| 1011 `host_unreachable` | The Host became unreachable, or a pipe to it failed |
| 4000 `conversation_changed` | An archive, a target or directory change, or a detach ended the stream |

`POST /api/conversations/:id/activity` records one user operation on the
conversation's Host as [activity](resource-lifecycle.md#activity) and answers
204. The page calls it at most every 30 seconds while the user operates a
user stream's view.

## Exposes

An expose record carries `id`, `deviceId`, `address`, `url`, `createdAt`
and `expiresAt`. `GET /api/exposes` returns `{ exposes }`, the caller's
exposes soonest expiry first. `POST /api/exposes` takes `{ deviceId, address }`
for a caller-owned, connected device and returns 201 with `{ expose }`; a
device that is offline or a stopped Cloud answers 409 `device_offline`, and
an instance without an expose domain answers 409 `expose_unavailable`.
`POST /api/exposes/:id/renew` sets the expiry to one hour from now and
returns `{ expose }`. `DELETE /api/exposes/:id` destroys it, ends its
connections, and returns 204. An expose the caller does not own, or one that has expired,
answers 404 `expose_not_found`. The `GET /api/state` snapshot includes the
same list and `exposeDomain`, null when the feature is unavailable.

Requests whose `Host` header is an expose hostname are not part of this API:
the backend answers them with the public relay, for every path and method,
and never with product routes. Lifetime, relay behavior and the
`demi host expose` commands are defined in [Host expose](expose.md).

## Account API

`POST /api/setup` and `POST /api/auth/login` take `{ email, password }`.
`POST /api/users` takes `{ email, password, role }` with role `admin` or `user`.
`PATCH /api/users/:id` resets a lower-ranked account with `{ password }`.
Setup creates the only master account and returns 404 after setup is complete. HTTP validation trims and lowercases addresses; storage
uniqueness and lookups are case-insensitive. User responses expose `id`, `email`,
`nickname`, `role` and `createdAt`, never the password hash.

- `PATCH /api/auth/me` takes `{ nickname }` (1–80 characters after trimming).
- `PUT /api/auth/password` takes `{ current, next }` and checks the current password.
- `POST /api/auth/email` takes `{ email, password }`, checks the current password,
  and sends a six-digit code to the new address. Its 202 response contains
  `{ challenge: { id, email, expiresAt } }`; the code is never returned.
- `POST /api/auth/email/confirm` takes `{ id, code }`. A successful response
  contains the updated user. Existing sessions keep their user identity; the
  next login uses the new address. Resending repeats the start request after
  its 60-second cooldown and replaces the previous code.

`auth/email-change.ts` owns issuance and delivery through the injected
`BackendOptions.accountMail` (`AccountMailSender`). No sender returns 503
`mail_unavailable`; delivery failure returns 503 `mail_failed` and removes that
challenge so the user can retry. Codes expire after ten minutes, allow at most
five wrong attempts, and can be used only once. A password change invalidates
previously issued challenges. The control service atomically checks uniqueness,
updates the email, and consumes the challenge. Tests use captured mail only.
Setup/admin-created accounts can sign in without a verification email; this
flow proves the new address when an existing account changes it. Registration
and password recovery are not provided.

## User preferences

`GET /api/settings/preferences` returns `{ preferences: { appearance, shortcuts, lastModel?, locale? } }`
for the signed-in user. These objects contain saved overrides; absent values use
the browser host's defaults. `PATCH` accepts any subset of appearance fields
(`theme`, `tone`, `accent`, `fontSize`) and shortcut keys (`new`, `sidebar`,
`settings`). A null shortcut removes that override. `lastModel` stores the explicit new-conversation
default as `{ providerId, modelId, thinkingEffort, serviceTierId }`, with nullable
thinking and tier. Choosing a model in an unsent draft saves this preference;
existing conversations keep their own selection. `locale` stores the time
zone and languages the user's browser last reported, as
`{ timeZone, languages }` with an IANA zone name and BCP 47 tags in preference
order; the product sends it whenever either changes. A time zone the backend
does not know, a malformed language tag, or more than 16 languages is
rejected. Commands receive it in their
[command context](native-runtime.md#command-context), and the
[conversation browser](browser.md#native-driver) starts with it. Unknown fields
are rejected.
The control service reads, merges and writes in one transaction, preserving
concurrent changes to other fields. Preferences persist across restarts and are
separate for every user in both instance modes. The product browser reads and
writes these overrides through its preference state adapter.

## Model configuration and provider inspection

`POST /api/providers` takes either `{ vendorId, label, apiKey, baseUrl?, models? }`
or `{ providerType, wireApi?, label, apiKey, baseUrl?, models? }`. The first derives
the family from the vendor catalog; the second names it explicitly. `wireApi` is
`responses` or `chat-completions`. PATCH accepts label and, for API-key entries,
key, endpoint, and model list. `baseUrl: null` removes the configured override.
The Test endpoint deliberately makes a real inference request; process providers
must be tried through a conversation with an execution target instead.

API providers accept `models` as a complete manual list of 1–1000 entries. Each entry supplies
`id`, `displayName`, positive `contextWindow`, nullable `outputLimit`,
`thinkingEfforts`, nullable `acceptedExtensions` (without dots) and nullable
`fastTier`. IDs must be unique and output limits cannot exceed context length.
`models: null` on PATCH explicitly returns to the live catalog; refreshing never
clears saved models. Provider adapters supply subscription catalogs. Cache behavior
is defined in [Model catalog caching](../model-catalog-cache.md).

The LLM assembly supplies these parameters to API provider factories and maps
manual metadata onto agent model selections at open/model-switch. The session
provider resolves the current configured output limit again at every inference
boundary, including after a settings edit. Browser thinking and tier choices
remain explicit; changing a catalog does not resend a failed message.

`GET /api/models` is an account-wide catalog, independent of conversation id.
Fresh entries use the shared cache; expired entries return immediately while
one background request refreshes them. `GET /api/models?refresh=true` waits for
a shared forced refresh. Each provider returns `sourceFetchedAt`, `stale` and `warnings`; one
catalog failure does not erase other providers or saved models. Static or never
fetched catalog timestamps use the framework's epoch sentinel.

`GET /api/providers/:id/status` returns auth/runtime state, account metadata,
active account, capabilities and the last real quota snapshot. No query returns
key/token material or raw vendor quota envelopes. `POST /api/providers/:id/quota`
refreshes only a provider's free quota probe, using request cancellation. A
provider requiring inference for a probe returns `quota_requires_inference`;
no data returns null, not a fabricated percentage. Reading status never invokes
inference. Shared users can read state and refresh free quota; configuring or
explicitly testing providers still requires admin rights.

## Subscription accounts

Claude subscription creation uses `POST /api/providers/setup-token` with
`{ token, label }`; no Claude copy-back OAuth flow is exposed. Existing Claude
providers accept another `{ token }` through `POST /api/providers/:id/accounts`.
Tokens are imported through the framework's credential API and never returned.
`GET /api/providers/:id/accounts` lists public account metadata and the active
account. `PUT …/accounts/active` takes `{ credentialId }`. `DELETE
…/accounts/:credentialId` refuses the active account: switch first, or delete
the provider to remove its last account.

Codex/Grok use `POST /api/providers/subscription-login { providerType, label? }`
for the first account and
`POST /api/providers/:id/accounts/login` for another account on an existing
provider. A start returns 202 with `{ login: { id, status: "pending" } }`.
Poll `GET /api/providers/subscription-login/:id`; cancel with DELETE at the same
path. Terminal results are retained for ten minutes. Login lifetime, credential
publication, and account selection follow
[Providers](providers-and-vault.md#credential-vault).

Provider account mutations obey the same shared-admin/isolated-owner rule as
configuration. A switch invalidates the assembly cache so subsequent requests
use the selected account, while an already-running request finishes with its
original runtime. An empty subscription pool is refused before inference; the
backend never falls back to a server operator's default account.

`GET /api/models` is independent of conversation ID. It returns provider health
and process requirements; the browser combines them with the selected target's
state. This inspection does not wake Cloud or execute a model. Inference
admission checks the actual target. Unknown auth/runtime status stays explicit;
catalog availability is not a successful inference test.

## Sidebar mutations, read state and page synchronization

`conversation/updates.ts` applies PATCH fields independently and shares the
existing agent-tree and target/file admission gates. `PATCH /api/conversations/:id`
accepts title, archived, pinned, target and provider/model selection. A single
refused field returns its 404/409 status; mixed outcomes return 207 with
`results: [{ field, status, code?, message?, httpStatus? }]` and the current
conversation. Unexpected operation failures are reported as 500 field results.
Applied fields remain applied. Archive is evaluated before the other fields,
so archiving and renaming together archives successfully but refuses the rename.
`POST /api/conversations/batch` accepts up to 100 `{ id, patch }` items and returns
207 with an outcome per item; missing conversations are reported individually.

Running root or child work refuses archive. Archive also refuses while a file
operation or asynchronous frame admission is in progress, except a file
transfer, which archive ends instead
([Host operations](sessions-and-targets.md#host-operations)). Archived conversations
allow transcript reads and read acknowledgements; stream upgrades, existing
socket writes, attachment delivery, host changes and metadata edits are refused until restore.
The transport waits for frame handling to finish before releasing admission;
it does not wait for an entire inference turn.

`POST /api/sidebar/reorder` takes `{ kind: "conversation" | "workspace", id,
beforeId: string | null }`; null appends. Conversation moves stay within the same
project and pin partition. [Storage](storage.md) owns persistent ordering. Activity timestamps never reorder rows.

`GET /api/conversations?archived=true|false` includes `status`, `revision`,
`readRevision`, `unread`, and `cwd`, the directory the conversation's work
runs in, resolved the same way for a device directory, a workspace, and the
Cloud, so the browser never derives it. Status is running/compacting from the live agent tree,
otherwise completed/error/stopped from its latest terminal block, or idle.
An unfinished checkpoint without a live session is interrupted. Checkpoint output
changes advance a persisted revision; user input alone does not. A browser sends
`POST /api/conversations/:id/read { revision }` for the output it actually showed.
Acknowledgements only move forward, and revisions beyond current output are refused.

`sync/product-state.ts` assembles `GET /api/state`: current user, mode, preferences,
projects, active and archived conversation summaries, devices (the paired ones
and the user's Cloud device, which the file and working-tree routes address
alike), public provider status and Cloud state. It never starts Cloud or runs inference. Responses use a
private ETag; `If-None-Match` returns 304 when unchanged. Browsers revalidate
on open, reconnect and a polling interval. This is a reconstructible snapshot,
not an atomic transaction across the control and conversation databases; a later
poll includes changes made during a read. Chat continues using agent frames.

## Device files and remote references

`GET /api/devices` includes the connected runner's `home` (null while unknown).
`GET /api/devices/:id/fs` defaults to that home when path is omitted and returns
`{ path, home, entries }`. Each entry has name, isDirectory, isSymbolicLink, byte
size and ISO modifiedAt. Metadata comes through the existing runner filesystem;
entries disappearing during the listing are omitted, other errors are returned.
Metadata requests await each reply to avoid overrunning the runner transport.
These device routes serve paired-device target selection only;
the work panel and remote attachment picker use the conversation routes below.
Managed devices are not accepted by the device directory routes.

Send and steer content may contain `{ type: "remote_file", deviceId, path }`, where
path is absolute. The backend validates ownership and current connectivity for
all referenced devices before adding any attachment grant. It attaches non-main
devices through the existing host mechanism, then supplies text preserving the
device identity and a shell-quoted `demi host shell --host` read command. The agent
reads the file's contents at execution time. Revocation or disconnect before that
read produces the existing host-command error; the reference is not a byte snapshot.

## File text and working tree changes

The work panel uses `GET /api/conversations/:id/fs?path=...` to list a directory
as `{ path, home, entries }`, with the same entry shape as device browsing.
Omitting `path` selects the conversation's execution directory. `POST` to the
same route with `{ path }` creates a directory recursively and returns `{ path }`
with status 201. `GET /api/conversations/:id/fs/file?path=...` returns
`{ path, text }`. Paths follow the Host's filesystem rules; the execution directory
is a starting directory, not a permission boundary. Missing paths answer 404,
and permission failures answer 403. A directory too large to list in one
runner message ([Runner](runner.md#connection-and-identity)) answers 413
`directory_too_large`.

All three operations use [conversation Host access](sessions-and-targets.md#host-operations),
including Cloud wake and the file gate. Archived conversations answer 409
`conversation_archived`. A paired device without a live runner answers 409
`device_offline`; a Cloud that cannot wake answers 503 with the lifecycle's code.
A file over 8 MiB answers 413 `file_too_large`, the same limit a retained edit
snapshot has ([Edit tracking](edit-tracking.md#scope)); one that is not UTF-8
text, or contains a NUL byte, answers 415 `not_text`.

`GET /api/conversations/:id/fs/raw?path=...` streams a file's bytes for
[file previews](file-previews.md) and downloads, through the same Host access
and with the same failures, and without a size limit. A media type the
preview table shows in place is served as itself; any other file, and every
file when `download=true`, is served as `application/octet-stream` with
`Content-Disposition: attachment` and the file's name. A path that is not a
regular file answers 404. A streamed answer has no `Content-Length`; `HEAD`
reports it, with the ETag and `Last-Modified`. One byte range in `Range` answers 206 with that
range; several ranges answer the whole file with 200, and a range that starts
past the end answers 416. The ETag derives from the file's size and
modification time, and `If-None-Match` answers 304. A request that names the
`version` it expects, an ETag it saw earlier, answers 412 `file_changed` once
the file no longer has it: a player's retries carry no validator, and they must never
splice two versions of a file together. Every answer carries
`X-Content-Type-Options: nosniff`, `Cache-Control: private, no-cache`,
`Vary: Cookie` and `X-Accel-Buffering: no`, the last so that a proxy in front
streams it instead of buffering it; an image served in place also carries the
[content policy](file-previews.md#keeping-file-content-inert). `HEAD` answers
the headers alone. How long a transfer may last, and what ends it, follows
[Host operations](sessions-and-targets.md#host-operations); while an archive,
a target change or a detach is ending the conversation's transfers, a new one
answers 409 `conversation_busy`.

The remote attachment picker lists and creates directories through
`GET/POST /api/conversations/:id/hosts/:deviceId/fs`, with the same directory
contract. The device must be the conversation's main or an attached host,
otherwise the request answers 404 `host_not_attached`. Omitting `path` lists
that Host's starting directory. These routes use the same Host access as the
work panel, including waking an attached Cloud. They do not read file contents;
the selected path becomes a remote reference when the message is sent.

`GET /api/conversations/:id/changes` lists the uncommitted changes of the
conversation's execution directory as `{ root, repository, head, files,
truncated, watched }`, the runner's reply ([Runner](runner.md#working-tree))
plus `root`, the directory the paths are relative to. The request reaches the
host the way every conversation file operation does
([Sessions and targets](sessions-and-targets.md#host-operations)): a stopped
Cloud wakes for it, a paired device without a live runner answers 409
`device_offline`, and a Cloud that cannot wake answers 503 with the lifecycle's
code. A request beyond the runner's working-tree capacity waits for a slot
([Load](runner.md#load)); the runner's timeout answers 504 `changes_timeout`,
and the browser then keeps its previous list and says the refresh failed.

`GET /api/conversations/:id/changes/file?path=...` returns `{ original,
modified }` for one changed file: `original` as the last commit has it (empty
for an added file), `modified` as the working tree has it (empty for a deleted
one), under the text limits of the file route.

`GET /api/conversations/:id/changes/raw?path=...` streams one file as the last
commit has it, the committed side of a previewed change, with the headers,
range handling and `download` of the raw file route but no validators. The working-tree side
comes from the raw file route. A path the last commit does not have answers
404, and a file over 8 MiB answers 413 `file_too_large`, since git's copy is
decoded whole before it is sent ([Runner](runner.md#working-tree)).

`GET /api/conversations/:id/commands/:commandId/changes/file?path=...&edit=0` returns
the same shape for one retained edit segment of a tool call, from the conversation's change
store, without touching the host ([Edit tracking](edit-tracking.md#the-change-store)).

The browser lists the working tree again when its change view is shown, after each of the
conversation's tool calls finishes while it shows (a call that finishes while
the view is away marks the list stale for its next showing), when the page
becomes visible again, and on its Refresh control. It never polls while idle.

## Serving the browser build

`createBackend({ webDirectory })`, or `DEMI_WEB_DIRECTORY` for the executable,
serves a built browser directory alongside the API and conversation sockets.
Extensionless HTML navigations fall back to index.html, enabling deep-page refresh.
Missing assets and `/api/*` misses remain 404. The product frontend builds
separately and uses a Vite proxy during development;
its fetch and WebSocket adapters connect the shared UI to this backend.
