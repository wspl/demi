# Web API reference

The browser and backend ship together, so routes have no version prefix.
Application data uses HTTP JSON; an open conversation uses the agent WebSocket
protocol. [Backend architecture](../backend/backend.md) defines authentication
and transport boundaries. This reference owns product endpoint contracts;
domain rules remain in the linked topic documents.

The `web-api` crate defines every JSON body of this reference, request and
response, and every error code; the conversation stream carries the agent
protocol's frames ([Frame protocol](../agent/runtime.md#frame-protocol)). The
backend decodes each request into its type and serializes each response from
its type, and the browser's REST types and schemas are generated from the same
types ([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
A response carries the fields its type declares and nothing more: fields the
backend keeps for itself in a stored record, such as a conversation's owner or
its message counts, never reach the browser.

## Resource index

Paths in the index are relative to `/api`, except the public installation row.
Detailed sections use the same relative paths or spell out the `/api` prefix.
JSON errors carry `{ code, message }`. `code` is a value of `ErrorCode`, the
`web-api` crate's list of every code the browser can see, which the browser
receives as a string union. A situation has one code on every route:
for example, an archived conversation that refuses an operation always answers
`conversation_archived`. Validation normally returns 400, missing or
inaccessible objects 404, and an operation refused by current state 409.
Partial conversation mutations use the explicit outcomes described below.

| Resource | Routes and request shape |
|---|---|
| Setup | `GET /setup` returns `{ needed }`; `POST /setup` creates the first master account and signs it in |
| Authentication | `POST /auth/login`, `POST /auth/logout`, `GET/PATCH /auth/me`, `PUT /auth/password`, `POST /auth/email`, `POST /auth/email/confirm` |
| Users | `GET/POST /users`, `PATCH /users/:id`; role hierarchy restricts administration |
| Application state | `GET /state`; conditional snapshot with private ETag |
| Settings | `GET /settings` returns fixed instance mode; `GET/PATCH /settings/preferences` |
| Conversations | `GET /conversations?archived=true\|false`, `POST /conversations { id }`, `PATCH /conversations/:id`, `POST /conversations/batch`, `POST /conversations/:id/fork { id, blockId }`, `POST /conversations/:id/read { revision }`, `POST /conversations/:id/title` requests a [generated title](product.md#conversation-titles) |
| Conversation history | `GET /conversations/:id/transcript` returns root blocks and subagent histories, each with the [failure facts](../backend/backend.md#failure-facts) of its error blocks; `WS /conversations/:id/stream` carries the [agent frames](../agent/runtime.md#frame-protocol) of that one conversation |
| Conversation files | `GET/POST /conversations/:id/fs`, `DELETE /conversations/:id/fs?path=...`, `GET /conversations/:id/fs/file?path=...`, `GET /conversations/:id/fs/raw?path=...&version=...&download=true\|false`, `PUT /conversations/:id/fs/raw?path=...&replace=true\|false` with raw bytes, `GET/POST /conversations/:id/hosts/:deviceId/fs` |
| Working tree | `GET /conversations/:id/changes`, `GET /conversations/:id/changes/file?path=...`, `GET /conversations/:id/changes/raw?path=...&download=true\|false`, `GET /conversations/:id/commands/:commandId/changes/file?path=...&edit=...` |
| User streams | `WS /conversations/:id/streams/:name` opens a declared [user stream](#user-streams); `POST /conversations/:id/activity` reports a user operation |
| Work panel | `GET/PUT /conversations/:id/panel` reads and saves the [work panel's state](#work-panel-state) |
| Conversation browser | `GET/POST /conversations/:id/browser/tabs`, `DELETE /conversations/:id/browser/tabs/:tab`, `POST /conversations/:id/browser/tabs/:tab/navigate { url }`, `POST /conversations/:id/browser/tabs/:tab/history { action }`; see [Conversation browser tabs](#conversation-browser-tabs) |
| Device log | `GET /devices/:id/log?since=<cursor>&limit=<n>&source=<source>` reads the [Host's log](../execution/runner.md#host-log) |
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

### Request bodies

The backend reads a JSON body whole, up to 1 MiB, and an attachment's bytes up
to 25 MiB ([Uploads and media](#uploads-and-media)); a larger body answers 413
`too_large` before the rest of it is read, so no request holds more memory
than that. A body the backend streams to a Host, a file upload or a runner's
pipe, has no size limit: it moves as fast as the Host writes it, and only what
is in flight is held.

A JSON body must match its request type exactly. A field the type does not
define, a missing required field, or a value outside its bounds answers 400
`invalid_body`, with a message that names the field and the reason; the
backend never drops an unknown field silently. A request that takes one of
several shapes names its shape in one field, such as a workspace's `kind`.
String lengths count UTF-16 code units, as the browser counts them
([Validation at entry](../architecture/contracts.md#validation-at-entry)).

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
owner returns the existing record (200 instead of 201). Both answer
`{ conversation }`, the conversation as the list shows it. An ID owned by
another user or reserved for a Fork returns 409 `id_unavailable`.

`POST /conversations/:id/fork` takes a destination UUID and an assistant block ID.
It copies a history boundary into a new conversation and returns `{ conversation,
model }`, with 201 on creation and 200 for a completed retry. It does not mutate
the source. The operation reserves the destination and records its metadata so
publication can recover after interruption. History boundary and command-state
semantics belong to [Conversation Fork](../agent/conversation-fork.md).

## Workspaces, devices, and attached hosts

Workspace creation takes `{ kind: "device", deviceId, path, name }` or
`{ kind: "cloud", name }`. The first names a directory on a caller-owned
device. The second creates a project directory on the user's Cloud and can
wake it. A workspace is a pointer; renaming or deleting it does not rename or
delete files. Deletion returns 409 `workspace_in_use` while conversations
still target it.

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
Changes follow [Attached hosts](../execution/sessions-and-targets.md#attached-hosts).

## Uploads and media

`POST /attachments` accepts a nonempty raw request body with its media type in
`Content-Type`. The header must name one `type/subtype`, and `multipart/*` — a
form envelope rather than a file's bytes — is rejected. The maximum body is
25 MiB. It answers 201 with `{ attachment }`: the attachment's id and
metadata, including the media type the backend reads from the file's bytes
when it recognizes them and, for a text file, its snippet. The snippet is the
opening of the file with leading blank space removed and line endings
normalized, cut to 160 characters (Unicode scalar values). The composer, for a
new message or for an edited one, shows the media type and the snippet on the
file's capsule ([Attachments](product.md#attachments)), so the capsule shows
what the backend determined.

`GET /blobs/:sha256` serves a blob of the caller's own namespace, under the
headers [Media by reference](../backend/backend.md#media-by-reference)
gives it. A name that is not a SHA-256 in lowercase hexadecimal, or that the
caller's namespace does not hold, answers 404 `not_found`, whoever else holds
that name.

Uploading alone stores a backend blob. A send, steer or edit frame names an
upload in its content as `{ type: "upload", ref, fileName }`; the backend
resolves the upload and writes the file to the selected Host's Demi
attachment directory, which can require an available execution target. No
frame carries a file's bytes. Media handling and failure behavior are defined
in [Backend media handling](../backend/backend.md#media-by-reference).

## Cloud

Each user has one Cloud device. [Managed hosts](../cloud/managed-hosts.md)
owns its allocation, wake, shared-device admission and reset;
[Sessions and targets](../execution/sessions-and-targets.md) owns how a
conversation selects it. A workspace on the Cloud is a directory on that
device, not a machine of its own.

`GET /api/cloud` returns the logical device identity, lifecycle state, disk
usage and limits, and the pending operation. `POST /api/cloud/reset` takes
`{ operationId }`. The backend selects and records the shipped base version at
admission. The response identifies the operation; the status response reports
stopping, saving, rebuilding, booting, ready or failure. Retrying the same
operation id returns the same operation. A reset affects every job on the
user's Cloud and preserves home. Requests cannot name another user's device or
arbitrary image paths. Settings use this API even when the guest is offline or
broken.

## User streams

`WS /api/conversations/:id/streams/:name` opens the declared
[user stream](../execution/native-runtime.md#user-streams) `name` on the
conversation's main Host; `browser` is the
[live browser view](../browser/live-view.md). The upgrade requires the session
cookie, a conversation the user owns, and an `Origin` of the product: the
stream operates a browser signed in to the user's sites, so an upgrade from
any other origin answers 403 `forbidden_origin` before the backend reaches the
Host. The route answers before the upgrade:

| Answer | When |
| --- | --- |
| 404 `unknown_stream` | No stream has that name |
| 426 `upgrade_required` | The request is not a WebSocket upgrade |
| 409 `conversation_archived` | The conversation is archived |
| 409 `device_offline` | A paired device has no live runner |
| 409 `host_stopped` | The Cloud is stopped; a user stream never wakes it |
| 409 `conversation_busy` | An archive, a target change or a detach is ending the conversation's streams |
| 502 `stream_failed` | The Host could not open the stream: its service failed to start, or refused it |

Admission follows [Host operations](../execution/sessions-and-targets.md#host-operations).
After the upgrade, binary messages carry the stream's bytes both ways, in
order; message boundaries carry no meaning, since the stream frames its own
messages. The backend reads from the Host only as fast as the page takes the
bytes. It closes the socket when the stream ends, with a code and a reason:

| Code and reason | When |
| --- | --- |
| 1000 `completed` | The invocation completed |
| 1003 `binary_only` | The page sent a text message |
| 1011 `host_unreachable` | The Host became unreachable, or a pipe to it failed |
| 4000 `conversation_changed` | An archive, a target or directory change, or a detach ended the stream |

`POST /api/conversations/:id/activity` records one user operation on the
conversation's Host as [activity](../execution/resource-lifecycle.md#activity)
and answers 204. The page calls it at most every 30 seconds while the user
operates a user stream's view.

## Work panel state

The [work panel](web-application.md#work-panel) saves one document per
conversation: `{ selection, tabs: [{ id, kind, data }] }`. `selection` is
`"change"`, `"file"` or a tab's id; `tabs` is in the user's order. The backend
stores the document and does not interpret it: `kind` and `data` mean
something only to the page, which validates each tab's `data` against its
kind's schema when it reads the document. The backend checks the shape above,
at most 64 tabs, and at most 64 KiB in all.

`GET /api/conversations/:id/panel` returns the document, or the empty one,
`{ selection: "change", tabs: [] }`, for a conversation that never saved.
`PUT` replaces it and answers 204. The page applies every change to itself
first and then saves the whole document, one save at a time so that the
latest is the one that stays. A page reads a conversation's document once:
from then on its own state is the newest there is, and reading again could
only bring back something older. Two pages open on one conversation each keep
their own view of it, and the last to save decides what the next page reads.
Archived conversations allow the read and refuse the save with 409
`conversation_archived`.

## Conversation browser tabs

These routes are how the page lists, opens, closes and navigates the tabs of
the [conversation browser](../browser/browser.md) on the conversation's main
Host. Each runs the operation the agent's command runs, `browser.tabs`,
`browser.open`, `browser.close`, `browser.goto`, `browser.back`,
`browser.forward` or `browser.reload`, with a `user` caller, as a
[one-shot user call](../execution/native-runtime.md#user-streams) through the
conversation's [host access](../execution/sessions-and-targets.md#host-operations).
What happens inside a tab travels on the [`browser` user stream](#user-streams).
A user waits in the panel, not in a script: for a `user` caller `open`, `goto`,
`back`, `forward` and `reload` start their work and answer at once, without
waiting for the page to load, and the tab's content shows the loading.

| Route | Does | A stopped Cloud |
| --- | --- | --- |
| `GET …/browser/tabs` | Returns `{ tabs: [{ id, title, url, createdBy }] }`; a browser that does not run has none | Is not woken: answers `{ tabs: [] }` |
| `POST …/browser/tabs { url? }` | Opens a tab, starting the environment when needed, and returns the tab; `url` defaults to `about:blank` | Is woken: opening a tab is ordinary demand |
| `DELETE …/browser/tabs/:tab` | Closes the tab and answers 204, also when the browser no longer has it | Is not woken: answers 204 |
| `POST …/browser/tabs/:tab/navigate { url }` | Starts loading the URL in the tab and answers 204 | Is not woken: answers 409 `host_stopped` |
| `POST …/browser/tabs/:tab/history { action }` | `back`, `forward` or `reload`; answers 204 | Is not woken: answers 409 `host_stopped` |

A tab the browser does not have answers 404 `tab_not_found`. Other refusals
follow the user stream's: 409 `conversation_archived`, `device_offline` and
`conversation_busy`, and 502 `browser_failed` with the operation's own code
and message when the browser refuses or cannot start. A call on a route that
does not wake a stopped Cloud is admitted as a user stream is and ends as one
does: when an archive, a target change or a detach ends the conversation's
streams, a call still running answers 409 `conversation_busy`
([Host operations](../execution/sessions-and-targets.md#host-operations)).

## Device log

`GET /api/devices/:id/log` returns lines of the
[Host's log](../execution/runner.md#host-log) for a device the user owns, the
user's Cloud included: `{ lines: [{ at, source, conversationId?, text }], next }`,
oldest first. `since` is the `next` of an earlier answer; without it the
answer ends at the newest line. `limit` defaults to 200 and is at most 1000.
`source` keeps only the lines of one source. The route wakes nothing: a
stopped Cloud or an offline device answers 409 as
[Host operations](../execution/sessions-and-targets.md#host-operations) do,
and its log is read when it runs again, since the log outlives a restart.

## Exposes

An expose record carries `id`, `deviceId`, `address`, `url`, `createdAt`
and `expiresAt`. `GET /api/exposes` returns `{ exposes }`, the caller's
exposes soonest expiry first. `POST /api/exposes` takes `{ deviceId, address }`
for a caller-owned, connected device and returns 201 with `{ expose }`; a
device that is offline or a stopped Cloud answers 409 `device_offline`, and
an instance without an expose domain answers 409 `expose_unavailable`.
`POST /api/exposes/:id/renew` sets the expiry to one hour from now and
returns `{ expose }`. `DELETE /api/exposes/:id` destroys it, ends its
connections, and returns 204. An expose the caller does not own, or one that
has expired, answers 404 `expose_not_found`. The `GET /api/state` snapshot
includes the same list and `exposeDomain`, null when the feature is
unavailable.

Requests whose `Host` header is an expose hostname are not part of this API:
the backend answers them with the public relay, for every path and method,
and never with product routes. Lifetime, relay behavior and the
`demi host expose` commands are defined in [Host expose](../execution/expose.md).

## Account API

`POST /api/setup` and `POST /api/auth/login` take `{ email, password }`.
`POST /api/users` takes `{ email, password, role }` with role `admin` or `user`.
`PATCH /api/users/:id` resets a lower-ranked account with `{ password }`.
Setup creates the only master account and answers 404 `already_set_up` after
setup is complete. HTTP validation trims and lowercases addresses; storage
uniqueness and lookups are case-insensitive. A password being set, at setup,
for a new account, by a reset or as the `next` password, has 8 to 1024
characters; a password checked against the account's, at login, as the
`current` password or to start an email change, only has to be nonempty, so
a wrong one answers 401 `invalid_credentials` rather than a validation error.
User responses expose `id`, `email`, `nickname`, `role` and `createdAt`, never
the password hash.

- `PATCH /api/auth/me` takes `{ nickname }` (1–80 characters after trimming).
- `PUT /api/auth/password` takes `{ current, next }` and checks the current password.
- `POST /api/auth/email` takes `{ email, password }`, checks the current password,
  and sends a six-digit code to the new address. Its 202 response contains
  `{ challenge: { id, email, expiresAt } }`; the code is never returned.
- `POST /api/auth/email/confirm` takes `{ id, code }`. A successful response
  contains the updated user. Existing sessions keep their user identity; the
  next login uses the new address. Resending repeats the start request after
  its 60-second cooldown and replaces the previous code.

The backend issues the code and delivers it through its account mail sender.
Without a sender, the start answers 503 `mail_unavailable`; a delivery failure
answers 503 `mail_failed` and removes that challenge so the user can retry.
Codes expire after ten minutes, allow at most five wrong attempts, and can be
used only once. A password change invalidates the challenges issued before it.
Confirmation checks uniqueness, updates the email, and consumes the challenge
in one transaction. Tests use captured mail only. Setup/admin-created accounts
can sign in without a verification email; this flow proves the new address
when an existing account changes it. Registration and password recovery are
not provided.

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
rejected. The backend keeps the time zone in its IANA spelling, each tag in
its canonical form and each language once, in the reported order: the report
`{ timeZone: "asia/shanghai", languages: ["zh-cn", "EN", "zh-CN", "iw"] }` is
kept as `{ timeZone: "Asia/Shanghai", languages: ["zh-CN", "en", "he"] }`,
as the browser's `Intl.getCanonicalLocales` would write the tags. Commands
receive it in their
[command context](../execution/native-runtime.md#command-context), and the
[conversation browser](../browser/browser.md#native-driver) starts with it.
The backend reads, merges and writes in one transaction, preserving
concurrent changes to other fields. Preferences persist across restarts and are
separate for every user in both instance modes. The product browser reads and
writes these overrides through its preference state adapter.

## Model configuration and provider inspection

`POST /api/providers` takes
`{ source: "vendor", vendorId, label, apiKey, baseUrl?, models? }` or
`{ source: "custom", providerType, wireApi?, label, apiKey, baseUrl?, models? }`.
The first derives the family, its wire and its endpoint from the vendor
catalog, `GET /api/providers/catalog`, which lists the
[vendors from models.dev](../providers/providers.md#vendors-from-modelsdev);
the second names the family explicitly. `wireApi` is
`responses` or `chat-completions`, and only a family that speaks both takes
it. A label has 1 to 80 characters after trimming, a key is one line of text,
and `baseUrl` is an `http` or `https` URL. PATCH accepts label and, for
API-key entries, key, endpoint, and model list; a subscription entry takes
only a new label (400 `subscription_only`). `baseUrl: null` removes the
configured override. The Test endpoint deliberately makes a real inference
request. A change of an entry, its accounts included, holds the entry while it
runs; another change meanwhile answers 409 `provider_busy`.

API providers accept `models` as a complete manual list of 1–1000 entries. Each entry supplies
`id`, `displayName`, positive `contextWindow`, nullable `outputLimit`,
`thinkingEfforts`, nullable `acceptedExtensions` (without dots) and nullable
`fastTier`. IDs must be unique and output limits cannot exceed context length.
`models: null` on PATCH explicitly returns to the live catalog; refreshing never
clears saved models. A subscription entry's catalog comes from its provider.

How a manual list's models become a conversation's model selection, and how
an edit of the list reaches a running conversation, is defined in
[Request parameters](../providers/models.md#request-parameters). Changing a
catalog never resends a failed message.

`GET /api/models` is an account-wide catalog, independent of any conversation.
Each provider carries its models, `sourceFetchedAt`, `stale` and `warnings`,
its authentication and runtime health, the `availability` the backend derives
from that health, and whether its transport is a process. `availability` is
`{ type: "unavailable", reason: "authentication", message }` while the
credential is missing or refused, `{ type: "unavailable", reason: "runtime",
message }` with the runtime's message while the provider cannot run, and
`{ type: "available" }` otherwise, unknown health included. Each model carries
the `selection` the backend built from it. One provider's catalog failure does
not remove the other providers or saved models. A static catalog, or one never
fetched, reports the Unix epoch as `sourceFetchedAt`. `refresh=true` waits for
a shared forced refresh; how catalogs are cached and refreshed is defined in
[Catalog cache](../providers/models.md#catalog-cache). The route does not wake
Cloud or execute a model. The browser combines each provider's health with the
state of the conversation's target and, for a provider whose transport is a
process, of the user's Cloud. Inference admission checks the actual target.
Unknown auth/runtime status stays explicit; catalog availability is not a
successful inference test.

`GET /api/providers/:id/status` returns auth/runtime state, account metadata,
active account, capabilities and quota. Each account carries its own `quota`,
the last real snapshot kept for it; the top-level `quota` is the active
account's. `quotaCapability` is `{ type: "none" }` for a family without quota,
or `{ type: "supported", probe }` with the probe's cost, `free` or
`inference`, or null for a family that only observes. No query returns
key/token material or raw vendor quota envelopes.
`POST /api/providers/:id/quota` takes `{ credentialId? }`, or no body, and
refreshes that account's free quota probe (the active account's without one);
cancelling the request stops the probe. An unknown account is
`account_not_found`. A provider requiring inference for a probe returns
`quota_requires_inference`, and a probe the vendor fails answers 502
`quota_unavailable`; a family that cannot probe answers the kept snapshot, and
no data returns null, not a fabricated percentage
([Vendor quota](../providers/usage-and-quota.md#vendor-quota)). Reading status
never invokes inference.
`POST /api/providers/:id/test` takes `{ modelId, credentialId? }` and tests
with that account, in use or not; an unknown account is `account_not_found`.
The test sends one request to the model and answers `{ type: "passed", model }`
when the provider answers, or `{ type: "failed", message, model? }` with the
provider's own reason: a test that ran and failed is a result, not an error. A
provider whose transport is a process is tested on the acting user's Cloud,
where its requests run too
([Where it runs](../providers/claude-code.md#where-it-runs)); the test wakes
it.

A subscription entry's CLI is read and managed under `/api/providers/:id/cli`:

| Route | Meaning |
|---|---|
| `GET …/cli?refresh=` | `{ newest, install, machines }`: the vendor's newest version (`{ version }`, or `{ error }` when the vendor cannot be read; `refresh=true` asks it at once), the last Cloud install (`installing`, `installed` with its path, `failed` with its message, or null), and the user's Cloud with its installed versions when it is running now (null when it did not answer). It wakes nothing: a stopped Cloud is simply absent. |
| `POST …/cli/install` | Starts the Cloud install again; 202 with its state. |

Adding an account to such an entry starts the same install; its failure is this
state and never the failure of adding the account.

On a shared instance every user reads provider state, but only the master sees
accounts, plan and usage: for everyone else `accounts` is empty, `active` names
no account, `quota` is null and an authenticated `auth` carries no account
label, in the status, in the account list and in the product state alike.
Configuring, testing and refreshing usage are the master's, and anyone else is
refused with 403 `forbidden` ([Scope](../providers/providers.md#scope)).

## Subscription accounts

A Claude subscription account comes from importing a setup token.
`POST /api/providers/setup-token` with `{ token, label }` creates the entry
(409 `provider_exists` when the scope has one), and
`POST /api/providers/:id/accounts` with `{ token }` adds another account to an
existing one; each answers 201. An imported token becomes an account record of
the [credential vault](../providers/providers.md#credential-vault) and is never
returned; a token the family refuses answers 400 `token_import_failed` with a
message that does not repeat it. `GET /api/providers/:id/accounts` lists public
account metadata and the active account as `{ accounts, active }`.
`PUT …/accounts/active` takes `{ credentialId }` and answers `{ active }`.
`DELETE …/accounts/:credentialId` refuses the active account with 409
`active_account`: switch first, or delete the provider to remove its last
account. An entry that does not take accounts this way answers 400
`accounts_unsupported`.

Codex/Grok use `POST /api/providers/subscription-login { providerType, label? }`
for the first account and
`POST /api/providers/:id/accounts/login` for another account on an existing
provider. A start returns 202 with `{ login: { id, status: "pending" } }`; a
family without a device login answers 400 `no_login_flow`. Poll
`GET /api/providers/subscription-login/:id`, which answers `{ login }` with
`{ status: "pending", verificationUrl, userCode, expiresAt }`, the first two
null until the vendor names them, then `{ status: "completed", providerId,
credentialId }` with the account the login added, or `{ status: "failed",
message }`. Cancel with DELETE at the same path. A login is its starter's to
read and cancel; another id answers 404 `login_not_found`. Terminal results are
retained for ten minutes. Login lifetime, credential publication, and account
selection follow [Providers](../providers/providers.md#credential-vault).

Provider account mutations obey the same shared-master/isolated-owner rule as
configuration. A switch takes effect at the next request, while an
already-running request finishes with its original runtime. An entry without
an account is refused before inference; the backend never falls back to a
vendor login of the machine it runs on.

## Sidebar mutations, read state and page synchronization

The backend applies the fields of `PATCH /api/conversations/:id`
independently: title, archived, pinned, target and provider/model selection.
A single refused field returns its 404/409 status; mixed outcomes return 207
with `results: [{ field, status, code?, message?, httpStatus? }]` and the
current conversation. Unexpected operation failures are reported as 500 field
results. Applied fields remain applied. Archive is evaluated before the other
fields, so archiving and renaming together archives successfully but refuses
the rename. `POST /api/conversations/batch` accepts up to 100 `{ id, patch }`
items and returns 207 with an outcome per item; missing conversations are
reported individually.

Archive and a target change are transitions: each holds the conversation while
it runs, and neither waits for other work. Running root or child work refuses
archive with 409 `turn_in_flight`, and so does another transition, an
asynchronous frame admission or a Host operation in progress, such as reading
a file or opening a browser tab. File transfers, user streams and the
[browser tab calls](#conversation-browser-tabs) that do not wake a stopped
Cloud do not refuse it: archive ends them instead
([Host operations](../execution/sessions-and-targets.md#host-operations)). A
target change refuses and ends the same way
([Switch the main target](../execution/sessions-and-targets.md#switch-the-main-target)).

A held conversation is waiting, not failed
([held conversations](../execution/sessions-and-targets.md#how-a-conversation-uses-a-device)).
A title, pin or model change that arrives while an archive, a target change or
a Cloud reset holds the conversation waits for it to end and is then applied
as if it had arrived afterwards; only the request's own cancellation, such as
the page closing the connection, ends the wait early. A frame waits the same
way, and the socket's later frames queue behind it in order. The transport
waits for frame handling to finish before releasing admission; it does not
wait for an entire inference turn.

Archived conversations allow transcript reads and read acknowledgements;
stream upgrades, frames on an open socket, attachment delivery, host changes
and metadata edits are refused until restore, with 409
`conversation_archived` where the refusal is an HTTP answer. A request to
the stream that is not a WebSocket upgrade answers 426 `upgrade_required`.

`POST /api/sidebar/reorder` takes `{ kind: "conversation" | "workspace", id,
beforeId: string | null }`; null appends. Conversation moves stay within the same
project and pin partition. [Storage](../backend/storage.md#control-records)
owns persistent ordering. Activity timestamps never reorder rows.

`GET /api/conversations?archived=true|false` includes `status`, `revision`,
`readRevision`, `unread`, and `cwd`, the directory the conversation's work
runs in, resolved the same way for a device directory, a workspace, and the
Cloud, so the browser never derives it. Status is running/compacting from the
live agent tree, otherwise completed/error/stopped from its latest terminal
block, or idle. An unfinished checkpoint without a live session is
interrupted.

A conversation is running while its tree will go on working without the user:
an agent of the tree is acting, or a child is still open. An open child counts
whatever it is doing, a wait for its own `yield` wakeup included, because it
resumes by itself and its close wakes its parent
([Subagents](../agent/subagents.md#result)). A shell command that outlives its
turn does not count: its exit wakes no one, so nothing follows until the user
writes. For example, the root answers "two children are looking into it" and
ends its turn: the conversation stays running until both children close and
the root has answered their results. The root answers "the build has started"
while `npm run build` still runs: the conversation is completed, and the
command shows as running only on its terminal tab. `web-ui` applies the same
rule to a conversation the page is attached to.

Checkpoint output changes advance a persisted revision; user input alone does
not. A browser sends `POST /api/conversations/:id/read { revision }` for the
output it actually showed. Acknowledgements only move forward, and revisions
beyond current output are refused.

`GET /api/state` returns the current user, mode, preferences, projects, active
and archived conversation summaries, devices (the paired ones and the user's
Cloud device, which the file and working-tree routes address alike), public
provider status and Cloud state. Each provider entry of the user's scope
carries its `details`: `{ type: "read", ... }` with what
`GET /api/providers/:id/status` answers, or `{ type: "failed", message }` for
an entry whose provider could not be read, which leaves the others intact. It
never starts Cloud or runs inference.
Responses use a private ETag; `If-None-Match` returns 304 when unchanged.
Browsers revalidate on open, reconnect and a polling interval. This is a
reconstructible snapshot, not an atomic transaction across the control and
conversation databases; a later poll includes changes made during a read.
Chat continues using agent frames.

## Device files and remote references

`GET /api/devices` includes the connected runner's `home` (null while unknown).
`GET /api/devices/:id/fs` defaults to that home when path is omitted and returns
`{ path, home, entries }`. Each entry has name, isDirectory, isSymbolicLink, byte
size and modifiedAt. Metadata comes through the runner's filesystem
operations; entries disappearing during the listing are omitted, other errors
are returned. Metadata requests await each reply to avoid overrunning the
runner transport. These device routes serve paired-device target selection
only; the work panel and remote attachment picker use the conversation routes
below. Managed devices are not accepted by the device directory routes. These
routes reach the device through device access
([Every way to a Host](../execution/sessions-and-targets.md#every-way-to-a-host)),
so a device that is not connected answers 409 `device_offline`.

The content of a send, steer or edit frame may contain
`{ type: "remote_file", deviceId, path }`, where path is absolute. The backend validates ownership and current connectivity for
all referenced devices before adding any attachment grant. It attaches non-main
devices as [attached hosts](../execution/sessions-and-targets.md#attached-hosts),
then supplies text preserving the device identity and a shell-quoted
`demi host shell --host` read command. The agent reads the file's contents at
execution time. Revocation or disconnect before that read produces the host
command's error; the reference is not a byte snapshot.

## File text and working tree changes

The work panel uses `GET /api/conversations/:id/fs?path=...` to list a directory
as `{ path, home, entries }`, with the same entry shape as device browsing.
Omitting `path` selects the conversation's execution directory. `POST` to the
same route with `{ path }` creates a directory recursively and returns `{ path }`
with status 201. `GET /api/conversations/:id/fs/file?path=...` returns
`{ path, text }`. Paths follow the Host's filesystem rules; the execution directory
is a starting directory, not a permission boundary. Missing paths answer 404,
and permission failures answer 403. A directory too large to list in one
runner message ([Runner](../execution/runner.md#connection-and-identity))
answers 413 `directory_too_large`.

All three operations use [conversation Host access](../execution/sessions-and-targets.md#host-operations),
including Cloud wake and the file gate. Archived conversations answer 409
`conversation_archived`. A paired device without a live runner answers 409
`device_offline`; a Cloud that cannot wake answers 503 with the lifecycle's code.
A file over 8 MiB answers 413 `file_too_large`, the same limit a retained edit
snapshot has ([Edit tracking](../execution/edit-tracking.md#scope)); one that
is not UTF-8 text, or contains a NUL byte, answers 415 `not_text`.

`GET /api/conversations/:id/fs/raw?path=...` streams a file's bytes for
[file previews](file-previews.md) and downloads, through the same Host access
and with the same failures, and without a size limit. A media type the
preview table shows in place is served as itself; any other file, and every
file when `download=true`, is served as `application/octet-stream` with
`Content-Disposition: attachment` and the file's name. A path that is not a
regular file answers 404. A streamed answer has no `Content-Length`; `HEAD`
reports it, with the ETag and `Last-Modified`. One byte range in `Range`
answers 206 with that range; several ranges answer the whole file with 200,
and a range that starts past the end answers 416. The ETag derives from the
file's size and modification time, and `If-None-Match` answers 304. A request
that names the `version` it expects, an ETag it saw earlier, answers 412
`file_changed` once the file no longer has it: a player's retries carry no
validator, and they must never splice two versions of a file together. Every
answer carries `X-Content-Type-Options: nosniff`,
`Cache-Control: private, no-cache`, `Vary: Cookie` and
`X-Accel-Buffering: no`, the last so that a proxy in front streams it instead
of buffering it; an image served in place also carries the
[content policy](file-previews.md#keeping-file-content-inert). `HEAD` answers
the headers alone. How long a transfer may last, and what ends it, follows
[Host operations](../execution/sessions-and-targets.md#host-operations); while
an archive, a target change or a detach is ending the conversation's
transfers, a new one answers 409 `conversation_busy`.

`PUT /api/conversations/:id/fs/raw?path=...&replace=true|false` writes the
request body, a file's raw bytes, to `path`, streaming it to the Host as the
Host takes it. The file appears whole or not at all: an upload cut short
leaves the path as it was ([Runner](../execution/runner.md#file-contents)).
When the upload starts, a directory at the path answers 409 `is_directory`,
and a file there answers 409 `file_exists` unless `replace=true`. A missing
directory above the path answers 404; the browser makes directories first
with `POST`. Success answers 204. An upload is a file transfer whose pace the
browser sets, under the same Host access, stall rule and ending as a download
([Host operations](../execution/sessions-and-targets.md#host-operations)).

`DELETE /api/conversations/:id/fs?path=...` deletes a file, or a directory
with everything in it, and answers 204; nothing at the path answers 204 too.
It refuses, with 409 `protected_path`, the filesystem root, the Host's home
directory, the conversation's execution directory, and any directory that
holds one of them: the file tree deletes only what an upload replaces inside
the workspace.

The remote attachment picker lists and creates directories through
`GET/POST /api/conversations/:id/hosts/:deviceId/fs`, with the same directory
contract. The device must be the conversation's main or an attached host,
otherwise the request answers 404 `host_not_attached`. Omitting `path` lists
that Host's starting directory. These routes use the same Host access as the
work panel, including waking an attached Cloud. They do not read file contents;
the selected path becomes a remote reference when the message is sent.

`GET /api/conversations/:id/changes` lists the uncommitted changes of the
conversation's execution directory as `{ root, repository, head, files,
truncated, watched }`, the runner's reply
([Runner](../execution/runner.md#working-tree)) plus `root`, the directory the
paths are relative to. The request reaches the host the way every conversation
file operation does
([Sessions and targets](../execution/sessions-and-targets.md#host-operations)):
a stopped Cloud wakes for it, a paired device without a live runner answers
409 `device_offline`, and a Cloud that cannot wake answers 503 with the
lifecycle's code. A request beyond the runner's working-tree capacity waits
for a slot ([Load](../execution/runner.md#load)); the runner's timeout answers
504 `changes_timeout`, and the browser then keeps its previous list and says
the refresh failed.

`GET /api/conversations/:id/changes/file?path=...` returns `{ original,
modified }` for one changed file: `original` as the last commit has it (empty
for an added file), `modified` as the working tree has it (empty for a deleted
one), under the text limits of the file route.

`GET /api/conversations/:id/changes/raw?path=...` streams one file as the last
commit has it, the committed side of a previewed change, with the headers,
range handling and `download` of the raw file route but no validators. The
working-tree side comes from the raw file route. A path the last commit does
not have answers 404, and a file over 8 MiB answers 413 `file_too_large`,
since git's copy is decoded whole before it is sent
([Runner](../execution/runner.md#working-tree)).

`GET /api/conversations/:id/commands/:commandId/changes/file?path=...&edit=0` returns
the same shape for one retained edit segment of a tool call, from the
conversation's change store, without touching the host
([Edit tracking](../execution/edit-tracking.md#the-change-store)). A
conversation the caller does not own answers 404 `conversation_not_found`, as
every conversation route does; a segment the change store does not keep
answers 404 `not_found`.

The browser lists the working tree again when its change view is shown, after each of the
conversation's tool calls finishes while it shows (a call that finishes while
the view is away marks the list stale for its next showing), when the page
becomes visible again, and on its Refresh control. It never polls while idle.

## Serving the browser build

With `DEMI_WEB_DIRECTORY` set, the backend serves that built browser directory
alongside the API and conversation sockets. Extensionless HTML navigations
fall back to index.html, enabling deep-page refresh. Missing assets and
`/api/*` misses remain 404. The product frontend builds separately and uses a
Vite proxy during development; its fetch and WebSocket adapters connect the
shared UI to this backend.
