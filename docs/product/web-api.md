# Web API reference

The web app and backend ship together, so routes have no version prefix.
Application data uses HTTP JSON, each page follows the user's state through
its [synchronization channel](#page-synchronization), and an open conversation
uses the agent WebSocket protocol.
[Backend architecture](../backend/backend.md) defines authentication and
transport boundaries. This reference owns product endpoint contracts;
domain rules remain in the linked topic documents.

The `web-api-protocol` crate defines every JSON body of this reference, request and
response, every message of the synchronization channel, and every error code;
the conversation stream carries the agent protocol's frames
([Frame protocol](../agent/runtime.md#frame-protocol)). The backend decodes
each request into its type and serializes each response from its type, and
the web app's REST types and schemas are generated from the same types
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
A response carries the fields its type declares and nothing more: fields the
backend keeps for itself in a stored record, such as a conversation's owner or
its message counts, never reach the web app.

## Resource index

Paths in the index are relative to `/api`, except the public installation row.
Detailed sections use the same relative paths or spell out the `/api` prefix.
JSON errors carry `{ code, message }`. `code` is a value of `ErrorCode`, the
`web-api-protocol` crate's list of every code the web app can see, which the web
app receives as a string union. A situation has one code on every route:
for example, an archived conversation that refuses an operation always answers
`conversation_archived`. Validation normally returns 400, missing or
inaccessible objects 404, and an operation refused by current state 409.
Partial conversation mutations use the explicit outcomes described below.

| Resource | Routes and request shape |
|---|---|
| Setup | `GET /setup` returns `{ needed }`; `POST /setup` creates the first master account and signs it in |
| Authentication | `POST /auth/login`, `POST /auth/logout`, `GET/PATCH /auth/me`, `PUT /auth/password`, `POST /auth/email`, `POST /auth/email/confirm` |
| Users | `GET/POST /users`, `PATCH /users/:id`; role hierarchy restricts administration |
| Page synchronization | `WS /sync` sends the product state, then each part of it that changes ([Page synchronization](#page-synchronization)) |
| Settings | `GET /settings` returns fixed instance mode; `GET/PATCH /settings/preferences` |
| Subagents | `PUT /subagents { enabled }`; `POST /subagents/profiles`, `PATCH /subagents/profiles/:id`, `DELETE /subagents/profiles/:id`; the switch and the profiles are part of the product state ([Subagents](#subagents)) |
| Conversations | `GET /conversations?archived=true\|false`, `POST /conversations { id, ... }`, `PATCH /conversations/:id`, `POST /conversations/batch`, `POST /conversations/:id/fork { id, blockId }`, `POST /conversations/:id/read { revision }`, `POST /conversations/:id/title` requests a [generated title](product.md#conversation-titles) |
| Conversation history | `GET /conversations/:id/transcript` returns root blocks and subagent histories, each with the [failure facts](../backend/backend.md#failure-facts) of its error blocks; `WS /conversations/:id/stream` carries the [agent frames](../agent/runtime.md#frame-protocol) of that one conversation |
| Conversation files | `GET/POST /conversations/:id/fs`, `DELETE /conversations/:id/fs?path=...`, `GET /conversations/:id/fs/file?path=...`, `WS /conversations/:id/fs/watch`, `GET /conversations/:id/fs/raw?path=...&version=...&download=true\|false`, `PUT /conversations/:id/fs/raw?path=...&replace=true\|false` with raw bytes, `GET/POST /conversations/:id/hosts/:deviceId/fs` |
| Working tree | `GET /conversations/:id/changes`, `GET /conversations/:id/changes/file?path=...`, `GET /conversations/:id/changes/raw?path=...&download=true\|false` |
| User streams | `WS /conversations/:id/streams/:name` opens a declared [user stream](#user-streams) |
| Work panel | `GET /conversations/:id/panel` reads the [work panel's tabs](#work-panel-state); `POST /conversations/:id/panel/changes { changes }` changes them |
| Conversation draft | `GET/PUT /conversations/:id/draft` reads and saves the [draft](#conversation-drafts); `POST /conversations/:id/draft/replaced { action, revision }` restores or dismisses the version a save replaced |
| Conversation permissions | `GET /conversations/:id/permissions` reads the conversation's [permission requests](#conversation-permissions); `POST /conversations/:id/permissions/requests/:request { decision }` decides a request |
| Device log | `GET /devices/:id/log?since=<cursor>&limit=<n>&source=<source>` reads the [Host's log](../execution/runner.md#host-log) |
| Direct channel | `WS /devices/:deviceId/direct` introduces the page to a paired device's runner and grants it conversations ([Direct channel](#direct-channel)) |
| Sidebar | `POST /sidebar/reorder { kind, id, beforeId }` |
| Plugins | `PUT /plugins/:plugin { enabled }` turns a plugin on or off for the caller ([A user's plugins](#a-users-plugins)); `GET /conversations/:id/plugins/:plugin/state` reads a plugin's [state for one conversation](#conversation-state-of-plugins); `POST /plugins/:plugin/calls/:method` and `POST /conversations/:id/plugins/:plugin/calls/:method` with the method's parameters call a [plugin's page method](#plugin-calls), for the user or for one conversation; `POST /conversations/:id/reload` reopens the conversation's tree with the user's current plugins |
| Models | `GET /models?refresh=true\|false` returns the account-wide catalog |
| Providers | `GET /providers/catalog`, `GET/POST /providers`, `PATCH/DELETE /providers/:id`, `GET /providers/:id/status`, `POST /providers/:id/test`, `POST /providers/:id/quota`; account routes below |
| Usage | `GET /usage` for the caller; `GET /usage/instance` for admins in shared mode |
| Devices | `GET /devices`, `POST /devices/claim { code }`, `DELETE /devices/:id`, `GET /devices/:id/fs?path=<absolute>`, `POST /devices/:id/fs { path }` |
| Workspaces | `GET/POST /workspaces`, `PATCH /workspaces/:id { name }`, `DELETE /workspaces/:id` |
| Cloud | `GET /cloud`, `POST /cloud/reset { operationId }` |
| Attachments | `POST /attachments` with raw bytes; `GET /blobs/:sha256?type=...` |
| Attached hosts | `GET /conversations/:id/hosts`, `POST .../hosts { deviceId }`, `PATCH .../hosts/:deviceId { name }`, `DELETE .../hosts/:deviceId` |
| Runner transport | `WS /runner`; device-authenticated pipes at `/pipes/:id`: `PUT` for a content pipe and `WS` for a stream pipe from the runner, `GET` for a pipe to it ([Host operations](../execution/runner.md#host-operations)) |
| Public installation | `GET /install.sh`, `GET /install.ps1`, `GET /runner-artifacts/:release/:target/:file`, and, with a local object store, the command artifacts, `GET /native-artifacts/:sha256` ([Backend deployment configuration](../execution/native-runtime.md#backend-deployment-configuration)) (root paths, outside `/api`) |

### Authentication

Each route authenticates its caller in one of these ways
([Authentication and ownership](../backend/backend.md#authentication-and-ownership)):

| Routes | Authentication |
|---|---|
| `GET/POST /setup`, `POST /auth/login` | None; a successful `POST` sets the session cookie |
| `WS /sync` | The session cookie, which the route checks without renewing the session |
| Every other path under `/api`, unknown paths included, except the runner transport | The session cookie, which the session gate checks |
| The runner transport, `WS /runner` and the pipes at `/pipes/:id` | A runner's device token; an unpaired runner's socket waits without one until a user claims its code |
| The public installation routes and the [web app build](#serving-the-web-app-build) | None |
| Every path on an expose hostname | None; the public relay answers it, never a route of this API ([Expose hostnames](#expose-hostnames)) |

The first three rows are the web app's routes. Such a route answers 403
`forbidden_origin` to a request that could act with the user's session and
comes from a page that is not the product's: a request whose method is not
GET, HEAD, OPTIONS or TRACE, or an upgrade, such as a WebSocket's. It answers
before any other check, so the request changes nothing and is refused with or
without a session. A request without `Origin` does not come from a web browser
and passes, so `curl` can call `POST /api/setup` without one.

### Request bodies

The backend reads a JSON body whole, up to 1 MiB, and an attachment's bytes up
to 25 MiB ([Uploads and media](#uploads-and-media)); a larger body answers 413
`too_large` before the rest of it is read, so no request holds more memory
than that. A body the backend streams to a Host, a file upload or a runner's
pipe, has no size limit: it moves as fast as the Host writes it, and only what
is in flight is held.

A page's WebSocket message is at most 1 MiB as well, on the conversation
socket, the synchronization channel and a user stream alike
(`MAX_PAGE_MESSAGE_BYTES`, which the generated tables give the page). No page
needs more: a frame refers to an upload and never carries its bytes, and a
user stream frames its own messages, so its bytes may travel in messages of
any size. A larger message fails the socket, which closes without a close
code. The conversation client never sends one: it answers a frame over the
limit as the backend answers a frame it refuses, with `rejected` (a
`steer_result` that rejects, for a steer) and the size in the reason, and the
socket stays open.

A JSON body must match its request type exactly. A field the type does not
define, a missing required field, or a value outside its bounds answers 400
`invalid_body`, with a message that names the field and the reason; the
backend never drops an unknown field silently. A request that takes one of
several shapes names its shape in one field, such as a workspace's `kind`.
String lengths count Unicode scalar values, as the web app's schemas count
them ([Validation at entry](../architecture/contracts.md#validation-at-entry)).

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
Cloud as its target without waking a machine. The body may also carry what a
new conversation starts with, so creating it is one request: the fields
`PATCH /conversations/:id` takes except `archived` (`title`, `pinned`,
`target` and the model settings), and `hosts`, the devices to attach, each
`{ deviceId, name? }` as `POST .../hosts` attaches it. They apply as part of
the creation: one that fails refuses the creation with its status and
`{ code, message }`, and nothing is created. Retrying the same ID for the same
owner returns the existing record (200 instead of 201) and applies nothing.
Both answer `{ conversation }`, the conversation as the list shows it, and
`hosts`, its attached hosts as `GET .../hosts` lists them. An ID owned by
another user or reserved for a Fork returns 409 `id_unavailable`.

`POST /conversations/:id/fork` takes a destination UUID and an assistant block ID.
It copies a history boundary into a new conversation and returns
`{ conversation }`, with 201 on creation and 200 for a completed retry; the
destination's model settings are the source record's at the time of the Fork.
It does not mutate the source. The operation reserves the destination and
records its metadata so publication can recover after interruption. A source the caller does not own
answers 404 `conversation_not_found`; the destination UUID of another creation
attempt, with another source or block, 409 `fork_conflict`; a UUID another
conversation holds, 409 `id_unavailable`; and a block that is not a completed
assistant text of the source's history, 400 `invalid_fork_target`. Which
texts can end a Fork, and what the destination keeps, belong to
[Conversation Fork](../agent/conversation-fork.md).

## Workspaces, devices, and attached hosts

Workspace creation takes `{ kind: "device", deviceId, path, name }` or
`{ kind: "cloud", name }`. The first names an absolute directory on a
caller-owned device; another device answers 404 `device_not_found`. The second
creates a project directory on the user's Cloud and can wake it. A name is
trimmed and has 1 to 256 characters. A creation answers `{ workspace }` with
201 and a rename, `PATCH /workspaces/:id { name }`, with 200; a workspace is
`{ id, deviceId, path, name, createdAt }`, and `GET /workspaces` returns
`{ workspaces }` in the user's order, which the product state carries too. A
workspace is a pointer; renaming or deleting it does not rename or delete
files. Deletion answers 204, and 409 `workspace_in_use` while conversations
still target it; an id the caller does not have answers 404
`workspace_not_found`.

Device revocation applies to user-paired devices, not the managed Cloud device,
and nothing refuses it. It removes the device's projects with it, the
workspaces whose device it is, but not their files: a workspace is a pointer.
Their conversations stay, with their history, outside any project, targeting
the project's directory on the removed device as a conversation that targets
a device directly does, so they cannot run there any more. Before the user
revokes a device in Settings, the page names the projects that go with it and
says that their files and conversations stay. The answer, 200, is `{ removed
}`, the ids of the workspaces removed. Revocation tells a connected runner to
remove itself
([Installation, pairing and removal](../execution/runner.md#installation-pairing-and-removal)),
closes its connection and removes its conversation attachments.
Pairing accepts a live code and returns the claimed device; expired/unknown codes
return 404 and excessive attempts 429. The installation routes contain no
credential and do not grant device access. The install command the page shows
fetches the installer from the origin of the state's `publicUrl`, not from the
page's own origin, which in development is Vite's, and lets `curl` use only
that URL's scheme, TLS 1.2 or newer for `https`; a development backend serves
plain `http`. The runner receives a pending code; the signed-in page claims
it through `POST /api/devices/claim`. Device tokens are delivered only to the
runner.

Attached-host responses contain device identity, name, cwd, online state, and
attachment time. A conversation's primary device cannot also be attached: attaching
it answers 409 `host_is_primary`. Names are unique within the conversation; a
conflicting rename returns 409 `name_taken`, and renaming a device that is not
attached answers 404 `host_not_attached`. A detach is a transition, like a target
change; detaching a device that is not attached answers 204.
Changes follow [Attached hosts](../execution/sessions-and-targets.md#attached-hosts).

## Uploads and media

`POST /attachments?name=<file name>` accepts a nonempty raw request body with
its media type in `Content-Type`. The header must name one `type/subtype`, and
`multipart/*` — a form envelope rather than a file's bytes — is rejected. The
maximum body is 25 MiB. The name, 1 to 255 characters, decides whether the
answer carries a text file's opening, as it decides for a message that sends the
file; a missing name answers 400 `invalid_query`. It answers 201 with
`{ attachment }`: the attachment's id and metadata, including the media type the
backend reads from the file's bytes when it recognizes them and, for a text
file, its snippet. The snippet is the opening of the file with leading blank
space removed and line endings normalized, cut to 160 characters (Unicode scalar
values). The composer, for a new message or for an edited one, shows the media
type and the snippet on the file's capsule
([Attachments](product.md#attachments)), so the capsule shows what the backend
determined.

The page sends no bytes the caller's namespace holds already. Before an upload
it computes the file's SHA-256 and asks with
`POST /attachments?name=<file name>&sha256=<hash>` and no body: when the
caller's namespace holds that blob, the answer is 201 with `{ attachment }`
for it, read from the stored bytes; otherwise 404 `blob_missing`, and the page
then uploads the bytes as above, naming the same `sha256`, which the backend
checks against what it received. The check reads only the caller's own
namespace, so it says nothing of another user's files.

`GET /api/conversations/:id/attachments/:attachment` answers an attachment
the agent uploaded ([Attachment commands](../execution/commands.md#attachment-commands))
as `{ id, name, mediaType, size, blob }`, where `attachment` is its number,
such as `a3`, and `blob` the hash its bytes are served under, below; a number
the conversation does not have answers 404 `not_found`. It reads the
conversation's database alone and wakes no Host.

`GET /blobs/:sha256` serves a blob of the caller's own namespace, under the
headers [Media by reference](../backend/backend.md#media-by-reference)
gives it. A name that is not a SHA-256 in lowercase hexadecimal, or that the
caller's namespace does not hold, answers 404 `not_found`, whoever else holds
that name. Byte ranges are answered as `fs/raw` answers them: one range in
`Range` answers 206 with that range, several answer the whole blob with 200,
and a range that starts past the end answers 416, so a player can play and
seek a video ([Media a tool returned](file-previews.md#media-a-tool-returned)).
The change view reads the two sides of a tool call's edit from it, by the
hashes the call's view holds ([Edit copies](../execution/edit-tracking.md#edit-copies)).

Uploading alone stores a backend blob and the upload's record, which keeps
the media type and the snippet the answer carried. A send, steer or edit frame
names an upload in its content as `{ type: "upload", ref, fileName }`; the
backend resolves the upload and writes the file to the selected Host's Demi
attachment directory, which can require an available execution target. No
frame carries a file's bytes. Media handling and failure behavior are defined
in [Backend media handling](../backend/backend.md#media-by-reference).

A [draft](#conversation-drafts) names an upload the same way, so every page
shows the file's capsule from its record, and any of them can send it. An
upload stays readable for as long as a draft names it: nothing deletes an
upload's record or its bytes
([Open decisions](../backend/storage.md#open-decisions)).

## Cloud

Each user has one Cloud device. [Managed hosts](../cloud/managed-hosts.md)
owns its allocation, wake, shared-device admission and reset;
[Sessions and targets](../execution/sessions-and-targets.md) owns how a
conversation selects it. A workspace on the Cloud is a directory on that
device, not a machine of its own.

`GET /api/cloud` returns `{ device, state, operation, error, volumes, limits, newerImage }`,
and the product state carries the same object as `cloud`:

| Field | Meaning |
| --- | --- |
| `device` | `{ id, name }`, the logical device identity; null until the first use makes it |
| `state` | `unallocated`, `off`, `booting`, `running`, `saving` or `resetting` |
| `operation` | The latest reset, `{ id, phase, error }`, with `phase` one of `stopping`, `saving`, `rebuilding`, `booting`, `ready` and `failed`; null before the first |
| `error` | Why the Cloud's last boot, save or reset failed; null once a boot succeeds |
| `volumes` | The capacities of its system and home filesystems, `{ systemBytes, homeBytes }`; null until its first boot made them, or while the machine manager does not answer |
| `limits` | The most each may grow to, in the same shape |
| `newerImage` | Whether the server runs another Cloud image than the one the Cloud's system is on, which a reset moves it to ([Demi's programs in a Cloud](../cloud/managed-hosts.md#demis-programs-in-a-cloud)); false before the first boot and while the machine manager does not answer |

The fields describe one moment: a reset reads `ready` only once `state` reads
`running` again.

Reading the status never wakes the Cloud. `POST /api/cloud/reset` takes
`{ operationId }`, a UUID the page chooses, and answers 202 `{ operation }`.
The backend selects and records the shipped base version at admission.
Retrying the same operation id returns the same operation, and retrying one
that failed resumes it; a reset with another operation id while one runs
answers 409 `cloud_resetting`, and a stopped Cloud that finds no capacity
permit answers 409 `cloud_capacity`. A reset affects every job on the user's
Cloud and preserves home. Requests cannot name another user's device or
arbitrary image paths. Settings use this API even when the guest is offline or
broken.

An operation that needs the Cloud running waits while a reset holds it
([How a conversation uses a device](../execution/sessions-and-targets.md#how-a-conversation-uses-a-device)),
and answers 503 with the lifecycle's code when it cannot have it:
`cloud_capacity` when every capacity permit of the backend is taken,
`cloud_crash_loop` once repeated runtime losses have stopped its automatic
boots, which a reset recovers, and `cloud_unavailable` when it cannot start,
when the reset it waited for failed, or when it reached the Cloud through a
Host handle while the Cloud was changing state
([Lifecycle and capacity](../cloud/managed-hosts.md#lifecycle-and-capacity)).

## User streams

`WS /api/conversations/:id/streams/:name` opens the declared
[user stream](../execution/native-runtime.md#user-streams) `name` on the
conversation's primary Host. A plugin declares each stream
([Calling its command package](../architecture/plugins.md#calling-its-command-package)):
`plugin-browser`'s `browser` is the [live browser view](../browser/live-view.md). The upgrade requires the session
cookie and a conversation the user owns, and is refused from a page that is
not the product's ([Authentication](#authentication)), which matters all the
more here since the stream operates the conversation browser, which is signed in
to the user's sites.
The route answers before the upgrade:

| Answer | When |
| --- | --- |
| 403 `forbidden_origin` | The upgrade comes from a page that is not the product's |
| 404 `unknown_stream` | No stream has that name, or the user has its plugin off |
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
| 4001 `plugin_disabled` | The user turned off the plugin that declares the stream |

## Work panel state

The backend keeps the [work panel](web-application.md#work-panel)'s tabs of
each conversation: `{ revision, tabs: [{ id, kind, data }] }`, in order.
`revision` counts the changes, 0 before the first. The backend does not
interpret `kind` and `data`: they mean something to the page, which validates
each tab's `data` against its kind's schema when it reads the panel, and to
the plugin that owns the kind ([Panel kinds](../architecture/plugins.md#panel-kinds)).
The selection is not part of it: each page keeps its own.

For example, a page creates a browser tab, and the browser plugin binds it:

```text
POST  .../panel/changes     { changes: [{ op: "create", id: "a1", kind: "browser",
                                          data: { url: "about:blank" } }] }        -> { revision: 4 }
      (the plugin)          update a1 with { tab: "t3" }                          -> revision 5
GET   .../panel             -> { revision: 5, tabs: [{ id: "a1", kind: "browser",
                                                       data: { url: "about:blank", tab: "t3" } }] }
```

`GET /api/conversations/:id/panel` returns the panel, `{ revision: 0, tabs:
[] }` for a conversation never changed. `POST
/api/conversations/:id/panel/changes { changes }` applies a list of changes in
order, as one change of the panel: Close Others, which removes eight tabs, is
one request and one new revision, and other pages see the eight tabs go at
once.

| Change | Fields | Does |
| --- | --- | --- |
| `create` | `id, kind, data, index?` | Creates the tab at `index`, after the others without one |
| `update` | `id, data`, an object | Sets each of its fields in the tab's `data`, removing those that are null, and leaves the other fields |
| `delete` | `id` | Removes the tab |
| `move` | `id, index` | Moves the tab to `index` among the others |

The request answers `{ revision, changed }`: the panel's revision once the
changes are in it, and whether the request changed the panel. A change that
has nothing to do changes nothing, and a request of only such changes answers
the current revision with `changed` false. A page that made a change reads the
panel only when the answer shows another change it does not hold: `changed`
with a revision past its own plus one, or `changed` false with a revision past
its own: an update, move or removal of a tab the panel no longer has,
and a create of an id the panel has, or had. **A tab's id is used once per
conversation.** The page makes a new one for each tab it creates, so sending a
create again after a lost answer creates nothing more, and a create that
arrives after the tab's removal brings nothing back. A plugin derives the ids
of its tabs from what they show, such as `browser-t7` for the browser's tab
`t7`, so it can add a tab for each such thing and none again once its user
removed it. An id is 1 to 64 characters.

Every change marks the conversation's summary, whose `panelRevision` is the
panel's revision, so every page that shows the conversation reads the panel
when the revision is higher than its own
([Page synchronization](web-application.md#page-synchronization)). The backend
applies the changes of a conversation one at a time, the page's and the
plugin's alike, so `revision` orders them all.

A request's changes apply all or none: a change that is refused refuses the
request with its status, and the panel stays as it was. A panel holds at most
64 tabs and 64 KiB of `data` in all: a create past either answers 409
`panel_full`, and an update past the size 413 `too_large`. A create of a `kind` no plugin the user has on declares answers
400 `unknown_panel_kind`. Archived conversations allow the read and refuse
every change with 409 `conversation_archived`.

## Conversation drafts

The backend keeps one draft per conversation: the message its composer holds
before it is sent. For example, a user types "Fix the login" into the
composer in one tab and drops `trace.txt` into it. Every other page that shows
the conversation, in another tab or on the user's phone, shows the same text
and the file's capsule moments after each save, and any of them can send the
message. [Drafts](web-application.md#drafts) says when a page saves and what
it shows.

A draft is `{ revision, text, files, replaced }`:

| Field | Meaning |
| --- | --- |
| `revision` | How many times the draft changed: 0 before its first save, one more with every save, restore and dismissal |
| `text` | The message's Markdown, with the attachment mark U+FFFC where each file's capsule stands |
| `files` | The files in the order of their marks: an upload as `{ type: "upload", ref, fileName, mediaType, sha256, snippet? }`, the last three from its record, or a file on a paired device as `{ type: "remote_file", deviceId, path }` |
| `replaced` | The version a save replaced without having been built on it, `{ revision, text, files }` with the revision it had as the draft, or null |

`GET /api/conversations/:id/draft` answers `{ draft }`; a conversation that
never saved one has revision 0, empty text, no files and no replaced version.

`PUT` takes `{ base, text, files }` and answers `{ draft }` as saved. `base` is
the revision the page's text was built on. The request names an upload as a
frame does, `{ type: "upload", ref, fileName }`, and a remote file as the
frame does; the backend adds the upload's media type, content hash and
snippet from its record, and an upload the caller does not have answers 404
`upload_not_found`. A text without exactly one mark per file answers 400
`invalid_body`. The draft as stored, text and files, is at most 256 KiB; a
larger one answers 413 `too_large`.

A save always takes effect: the last save wins. When `base` is older than
the revision at which the draft's text was written, the page never showed
that text, and the save keeps it as `replaced`, unless it is empty or the
same as the saved one. A dismissal, or a save that repeats the text, changes
the revision and not the text, so it makes no page's next save replace
anything. Only one replaced version is kept, so an earlier one goes. For
example, two tabs show revision 4, "Fix the login", and their users type at
the same time:

1. Tab A saves "Fix the login bug" with base 4, which becomes revision 5.
2. Tab B saves "Fix the login test" with base 4, older than revision 5, at
   which the draft's text was written, so the backend keeps revision 5 as
   `replaced` and saves B's text as revision 6.
3. Every page shows "Fix the login test" and offers "Fix the login bug" for
   restore.

`POST /api/conversations/:id/draft/replaced { action, revision }` acts on the
replaced version whose revision it names. `restore` exchanges it with the
draft's text and files, so the version it displaces becomes the replaced one
and nothing is lost: in the example, the draft becomes "Fix the login bug" as
revision 7, and revision 6 is offered in its place. `dismiss` drops it. Each
answers `{ draft }`; when the replaced version is no longer the one named,
because another save or action changed it, the answer is 409 `draft_changed`.
Otherwise the replaced version stays until a later save replaces a version:
a send, an archive or a reload keeps it.

Other pages learn of a change from the conversation's summary, which their
synchronization channel brings once the change commits and which carries the
draft's revision as `draftRevision`
([Page synchronization](#page-synchronization)); they read the draft only
then. A page takes a draft only when its revision is higher than the one it
holds, since a save's answer and a read can reach it in either order. An
archived conversation reads its draft and refuses the other operations with
409 `conversation_archived`; after a restore it has the draft it had. A draft
lasts as long as its conversation, and a Fork starts with an empty one
([Conversation Fork](../agent/conversation-fork.md)).

## Conversation permissions

A conversation's [permission requests](../agent/permissions.md) reach a page
by revision, as a draft does. For example, an agent of `c_81`
runs `demi skills add` without the grant. Every page of the user receives the
conversation's summary with `permissionRequests: 1` and a higher
`permissionsRevision`, and a page that shows `c_81` reads:

```text
GET /api/conversations/c_81/permissions

200 { "revision": 12,
      "requests": [ { "id": "pr_3k9",
                      "category": { "id": "skills.manage", "action": "manage skills",
                                    "description": "..." },
                      "command": "demi skills add vercel-labs/agent-skills --skill web-design-guidelines",
                      "agent": null, "createdAt": "..." } ] }

POST /api/conversations/c_81/permissions/requests/pr_3k9
{ "decision": "allow" }

204
```

| Route | Body | Does |
| --- | --- | --- |
| `GET /api/conversations/:id/permissions` | None | Returns `{ revision, requests }`: the undecided requests, oldest first, each with its agent as null for the root or `{ number, description }`; a category no longer in the user's command set has its `id` alone |
| `POST /api/conversations/:id/permissions/requests/:request` | `{ decision: "allow" \| "deny" }` | Decides the request ([Requests](../agent/permissions.md#requests)), sends its message to the agent, and answers 204 |

A request that is no longer undecided, because another page decided it, an
Allow of its category decided it, or a newer request replaced it, answers 404
`permission_request_not_found`; the page shows the outcome from the
conversation's summary. An archived conversation has no requests and
refuses a decision with 409 `conversation_archived`. The page takes an answer only when its revision is
higher than the one it holds, since a read and the summary can reach it in
either order.

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

## Expose hostnames

Requests whose `Host` header is an expose hostname are not part of this API:
the backend answers them with the public relay, for every path and method,
and never with product routes. The records, their lifetime, the relay, and the
`expose` plugin's commands, state and page methods are defined in
[Host expose](../execution/expose.md).

## Account API

`POST /api/setup` takes `{ nickname, email, password }`, the nickname as
`PATCH /api/auth/me` takes it, and `POST /api/auth/login` takes
`{ email, password }`.
The users routes are an administrator's: a user answers 403 `forbidden`.
`GET /api/users` returns `{ users }`, every account in the order they were
created. `POST /api/users` takes `{ email, password, role }` with role `admin`
or `user`, creates an account of a role the caller outranks and returns
`{ user }` with 201; a role the caller does not outrank answers 403
`forbidden`, and an address an account has answers 409 `email_taken`.
`PATCH /api/users/:id` resets a lower-ranked account with `{ password }` and
answers 204; an id that names no account answers 404 `user_not_found` and an
account the caller does not outrank 403 `forbidden`, both before the body is
checked.
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

`GET /api/settings/preferences` returns `{ preferences: { appearance, shortcuts, lastModel?, lastProjectHost?, locale?, contextLimits? } }`
for the signed-in user. These objects contain saved overrides; absent values use
the web app's defaults. `PATCH` accepts any subset of appearance fields
(`theme`, `tone`, `accent`, `fontSize`) and shortcut keys (`new`, `sidebar`,
`settings`). A null shortcut removes that override. `lastModel` stores the explicit new-conversation
default as model settings, `{ providerId, modelId, thinkingEffort, serviceTierId }`
([A conversation's model settings](../providers/models.md#a-conversations-model-settings)).
Every explicit choice of a model, an effort or a tier saves it, in an unsent
draft or in an existing conversation. A new conversation starts with it, and
its first send writes it to the new conversation's record, an effort the
model no longer offers replaced by the model's first effort and a tier it no
longer offers by the vendor's default. Its effort is null only for a model
that lists no efforts. From then on the
conversation has its own settings, which a later change of the preference does
not touch. `lastProjectHost` stores where New Project starts,
`{ kind: "cloud" | "device", deviceId? }`: the kind and the device the user
last chose in it ([Conversations and projects](product.md#conversations-and-projects)).
A patch replaces it whole; a `deviceId` the user no longer has is kept and
the dialog chooses no device for it. `locale` stores the time
zone and languages the user's browser last reported, as
`{ timeZone, languages }` with an IANA zone name and BCP 47 tags in preference
order; the product sends it whenever either changes. A time zone the backend
does not know, a malformed language tag, or more than 16 languages is
rejected. The backend keeps the time zone in its IANA spelling, each tag in
its canonical form and each language once, in the reported order: the report
`{ timeZone: "asia/shanghai", languages: ["zh-cn", "EN", "zh-CN", "iw"] }` is
kept as `{ timeZone: "Asia/Shanghai", languages: ["zh-CN", "en", "he"] }`,
as `Intl.getCanonicalLocales` in the user's browser would write the tags.
Commands receive it in their
[command context](../execution/native-runtime.md#command-context), and the
[conversation browser](../browser/browser.md#native-driver) starts with it.
`contextLimits` holds the user's
[context limits](../providers/models.md#context-limit), by provider entry id
and then model id, each in tokens, such as
`{ "<entry id>": { "claude-opus-4-8": 300000 } }`; a model it does not name
uses its full window. A patch changes one model's limit with
`contextLimit: { providerId, modelId, tokens }`: 500000, 300000 or 200000
stores that limit, null removes it, and any other number is refused. The
backend does not look the model up: a limit its window does not offer has no
effect. The backend reads, merges and writes in one transaction, preserving
concurrent changes to other fields. Preferences persist across restarts and are
separate for every user in both instance modes. The web app reads and
writes these overrides through its preference state adapter.

## Subagents

The caller's subagent settings, which the settings page's Subagent section
shows, are part of the product state, as `subagents`:
`{ enabled, profiles }`. `enabled` is the
[Subagent switch](../agent/subagents.md#profiles), true for a user who never
turned it off. `profiles` holds the caller's
[subagent profiles](../agent/subagents.md#profiles), in name order, each
`{ id, name, description, model, instructions, canSpawn, enabled }`:

| Field | Value |
|---|---|
| `id` | The backend's id for the profile, which its name can change under |
| `name`, `description` | The `--profile` value and when the agent should use the profile |
| `model` | null for the parent's model, or model settings `{ providerId, modelId, thinkingEffort, serviceTierId }`, as a conversation's ([Sidebar mutations and read state](#sidebar-mutations-and-read-state)); `thinkingEffort` is always an effort the model lists or `disabled`, never null |
| `instructions` | null for the parent's instructions, or the text that replaces them |
| `canSpawn` | Whether the profile's children may spawn children of their own |
| `enabled` | Whether agents may use the profile |

`PUT /api/subagents { enabled }` turns subagents on or off for the caller
and answers 204.

`POST /api/subagents/profiles` takes `{ name, description, model,
instructions, canSpawn }`, creates the profile enabled, and answers 201 with
`{ profile }`. `PATCH /api/subagents/profiles/:id` takes any subset of those
fields and `enabled`, and answers `{ profile }`; `model` and `instructions`
are whole values, and null returns either to the parent's.
`DELETE /api/subagents/profiles/:id` answers 204. An id that names none of
the caller's profiles answers 404 `profile_not_found`. The backend checks a
body before it writes:

- A name that breaks the [name rule](../agent/subagents.md#profiles),
  `default` included, a description that is blank, longer than 500
  characters or holds a line break, and instructions that are blank or longer
  than 65,536 characters answer 400 `invalid_body`.
- A name another of the caller's profiles has answers 409 `profile_exists`.
- A model is checked against the entry's catalog as a conversation's model
  settings are, with the same answers: 404 `provider_not_found` for an entry
  outside the caller's scope, 404 `model_not_found`, and 409
  `setting_unavailable` for an effort or a tier the model does not offer. A
  body whose model names no effort gets the first effort the model lists
  ([A conversation's model settings](../providers/models.md#a-conversations-model-settings)).
  Turning a profile on or off checks nothing else, so an unavailable profile
  can be disabled and enabled as it is.

Each change, to the switch or to a profile, reaches every page of the user as
the `subagents` message of the [synchronization channel](#page-synchronization),
and the next spawn of every conversation, an open one included
([Profiles](../agent/subagents.md#profiles)). The settings are the user's in
both instance modes; on a shared instance a profile's model names one of the
master's entries, as every model a user picks does.

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
from that health, and `cliPackage`: the command package that installs the
CLI its requests start on the user's Cloud, such as `demi.claude-code`, or
null when its requests are HTTP; the page knows by it that the provider
needs the Cloud. The entries of the product state carry the same `cliPackage`. `availability` is
`{ type: "unavailable", reason: "authentication", message }` while the
credential is missing or refused, `{ type: "unavailable", reason: "runtime",
message }` with the runtime's message while the provider cannot run, and
`{ type: "available" }` otherwise, unknown health included. Each model carries
the `selection` the backend built from it. The first effort of a model's
`supportedThinkingEfforts` is the one a conversation's model settings hold on
the model when a change names none
([A conversation's model settings](../providers/models.md#a-conversations-model-settings)).
One provider's catalog failure does not remove the other providers or saved
models. A static catalog, or one never fetched, reports the Unix epoch as
`sourceFetchedAt`. `refresh=true` waits for
a shared forced refresh; how catalogs are cached and refreshed is defined in
[Catalog cache](../providers/models.md#catalog-cache). The route does not wake
Cloud or execute a model. The web app combines each provider's health with the
state of the conversation's target and, for a provider whose transport is a
process, of the user's Cloud. Inference admission checks the actual target.
Unknown auth/runtime status stays explicit; catalog availability is not a
successful inference test.

`GET /api/providers/:id/status` returns auth/runtime state, account metadata,
active account, capabilities and quota. An entry whose provider cannot be built
or read answers 502 `provider_status_failed` with the reason, the message its
`failed` details carry in the sync state. Each account carries its own `quota`,
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

The CLI of an entry whose provider runs a process is read and managed under
`/api/providers/:id/cli`; for any other entry these routes answer 404
`provider_not_found`:

| Route | Meaning |
|---|---|
| `GET …/cli?refresh=` | `{ newest, install, machines }`: the vendor's newest version (`{ type: "read", version }`, or `{ type: "unreadable", message }` when the vendor cannot be read; `refresh=true` asks it at once), the caller's last Cloud install since the backend started (`{ state: "installing" }`, `{ state: "installed", path }`, `{ state: "failed", message }`, or null), and the caller's Cloud as `{ deviceId, name, versions }` with its installed versions, newest first, while its runner is connected (`versions` is null when it did not answer). It wakes nothing: a stopped Cloud is simply absent. |
| `POST …/cli/install` | Starts the install on the caller's Cloud again, unless one is under way; 202 `{ install }` with its state. |

Adding an account to such an entry starts the same install on the acting
user's Cloud; its failure is this state and never the failure of adding the
account.

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
null until the vendor names them and `expiresAt` the moment the login ends
unless the user finishes it, then `{ status: "completed", providerId,
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

## Sidebar mutations and read state

The backend applies the fields of `PATCH /api/conversations/:id` independently:
`title` (trimmed, 1 to 256 characters), `archived`, `pinned`, `target`, and
the model settings: `model`, the provider entry and model as
`{ providerId, modelId }`, `thinkingEffort` and `serviceTierId`. A patch whose
only field fails answers that field's status with `{ code, message }`;
otherwise the answer is the current conversation with
`results: [{ field, status, code?, message?, httpStatus? }]`, 200 when every
field applied and 207 when any was refused. Unexpected operation failures are
reported as 500 field results. Applied fields remain applied. Archive is
evaluated before the other fields, so archiving and renaming together archives
successfully but refuses the rename. A rename follows [Conversation
titles](product.md#conversation-titles). `POST /api/conversations/batch` accepts
up to 100 `{ id, patch }` items and returns 207 with an outcome per item:
`{ id, status: "updated", conversation, results }`, or
`{ id, status: "refused", code, message }` for a conversation the caller does
not have.

A conversation's model settings are one value, the model selection its record
holds ([A conversation's model settings](../providers/models.md#a-conversations-model-settings)).
The conversation lists carry the settings as `model`,
`{ providerId, modelId, thinkingEffort, serviceTierId }`, or null while the
conversation has no model yet; `thinkingEffort` is null only for a model that
lists no efforts. The record also keeps the model's facts. Only a
patch changes the value, and each of its three fields changes one part:
`model` switches to that model, with the effort and the tier the same patch
names, and the new model's first effort and the vendor's default tier for a
part it leaves out; `thinkingEffort`, an effort or `disabled` and never null,
and `serviceTierId`, a tier or null for the vendor's default, set their part
for the conversation's model. The backend
applies one conversation's changes one at a time, in the order they arrive,
each to the value the one before left:

1. It checks each part against the entry's catalog. An entry outside the
   caller's scope answers 404 `provider_not_found`, a model the catalog does
   not list 404 `model_not_found`, an effort or a tier the model does not
   offer 409 `setting_unavailable`, and an effort or a tier for a
   conversation without a model 409 `model_not_selected`.
2. When the conversation's tree is live and the new model belongs to another
   provider entry, it builds the tree's runtime for that entry first, so a
   model that cannot run is refused and changes nothing.
3. It commits the record.
4. It switches the live tree to the new selection, which the tree's next
   provider request uses, inside a running turn too
   ([Model switch](../agent/runtime.md#model-switch)).

A crash before the commit changes nothing. A crash after it leaves the change
in the record, and the tree takes it when it is next opened. A page whose
connection broke before the answer shows the outcome from its synchronization
channel, whichever it was. Opening the conversation's socket names no model,
and the backend opens a tree only with the record's selection, so an open
never changes the settings, whatever the opening page last saw
([Connections and the live tree](../agent/runtime.md#connections-and-the-live-tree)).

Every page shows the record's value. The page that made a change shows it from
the patch's answer at once, and every other page, in another tab or on another
device, from the conversation's summary, which its synchronization channel
brings once the change commits ([Page synchronization](#page-synchronization)).
A page keeps no model settings of its own for a conversation with a record,
and never sends the value back: a change names only the part its user
changed, so a page that has not yet seen another page's change cannot undo
it. Two changes of one part end with the one applied last,
and changes of different parts both stay. A new conversation keeps its
settings in the user's browser until its first send writes them to its record
([User preferences](#user-preferences)), and a change on a page of a
conversation whose record has no model yet writes the whole value.

Archive and a target change are transitions: each holds the conversation while
it runs, and neither waits for other work. Running root or child work refuses
archive with 409 `turn_in_flight`, and so does another transition, an
asynchronous frame admission or a Host operation in progress, such as reading
a file or opening a tab in the conversation browser. File transfers, user
streams and the [conversation browser's tab methods](../browser/live-view.md#the-tab-methods)
that do not wake a stopped Cloud do not refuse it: archive ends them instead
([Host operations](../execution/sessions-and-targets.md#host-operations)). A
target change refuses and ends the same way
([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)).
A target that names a workspace or a device the user does not have answers
404 `workspace_not_found` or `device_not_found`, and a target change that
another one overtook answers 409 `target_conflict`.

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
`conversation_archived` where the refusal is an HTTP answer. An upgrade of
the stream from a page that is not the product's answers 403
`forbidden_origin` ([Authentication](#authentication)), and a request to the
stream that is not a WebSocket upgrade 426 `upgrade_required`.

`POST /api/sidebar/reorder` takes `{ kind: "conversation" | "workspace", id,
beforeId: string | null }`; null appends, and a success answers 204.
Conversation moves stay within the same project and pin partition, and a
workspace moves among the user's workspaces; a move out of them, or of a row
that is archived or the caller does not have, answers 409 `invalid_order`.
[Storage](../backend/storage.md#control-records) owns persistent ordering.
Activity timestamps never reorder rows.

`GET /api/conversations?archived=true|false` includes `status`, `revision`,
`readRevision`, `unread`, and `cwd`, the directory the conversation's work runs
in, resolved the same way for a device directory, a workspace, and the Cloud, so
the web app never derives it. Status is running/compacting from the live agent
tree, otherwise completed/error/stopped from its latest terminal block, or idle.
An unfinished checkpoint without a live session is interrupted. `titleCurrent`
says whether the title has read every message the user sent, when asking for a
new one could say nothing new, and `titleGenerating` whether a title request is
in flight. `pluginsChanged` says whether the conversation's tree is open
with commands of plugins the user has since turned on or off, so a reload
would change them ([Reload](#a-users-plugins)). `draftRevision` is
the revision of the conversation's
[draft](#conversation-drafts), 0 before its first save: the page reads the
draft itself only when this number is higher than the revision it holds, so a
summary carries no draft's text. `panelRevision` is the revision of the
conversation's [work panel](#work-panel-state), 0 before its first change,
read the same way. `pluginRevisions` does the same for each
plugin's [conversation state](#conversation-state-of-plugins), as
`{ plugin, revision }` for every plugin that declares one, in registration
order, so turning a plugin on or off changes no summary. What changes in the
conversation's files is not in the summary: the Host reports it on the
[file watch](#file-watch).
`permissionRequests` counts the conversation's undecided
[permission requests](#conversation-permissions), which the sidebar shows as
the needs-you mark, and `permissionsRevision` rises with each change of its
requests; a page that shows the conversation reads them when it is newer
than the revision it holds. `pluginRevisions` and
`permissionsRevision` are [counted in memory](#revisions-counted-in-memory).

`POST /api/conversations/:id/title`, without a body, asks the conversation's
model selection for a new title from every message the user sent and answers
202; the title arrives with the conversation's summary once written. A
conversation without message text answers 409 `no_messages`, one without a
model 409 `model_not_selected`, an archived one 409 `conversation_archived`,
and one whose provider is no longer in the caller's scope 404
`provider_not_found`.

A conversation is running while its tree will go on working without the user: an
agent of the tree is acting, or a child is still open. An agent acts until the
save that ends its action commits, which is also when the page sees its phase
go idle ([A turn](../agent/runtime.md#a-turn)), so once a conversation no
longer runs, its transcript route shows what the live tree showed. An open child counts
whatever it is doing, a wait for its own `yield` wakeup included, because it
resumes by itself and its close wakes its parent
([Subagents](../agent/subagents.md#result)). A shell command that outlives its
turn does not count: its exit wakes no one, so nothing follows until the user
writes. For example, the root answers "two children are looking into it" and
ends its turn: the conversation stays running until both children close and the
root has answered their results. The root answers "the build has started" while
`npm run build` still runs: the conversation is completed, and the command shows
as running only on its terminal tab. `web-ui` applies the same rule to a
conversation the page is attached to.

Checkpoint output changes advance a persisted revision; user input alone does
not. A page sends `POST /api/conversations/:id/read { revision }` for the
output it actually showed. Acknowledgements only move forward, and revisions
beyond current output are refused.

## Page synchronization

Each page follows the user's state through one WebSocket, `WS /api/sync`,
rather than asking for it. For example, a user renames a conversation in a tab
on their laptop. Once the rename commits, the backend sends the conversation's
new summary on the channel of every page the user has open: the laptop's
other tabs and the phone show the new title a few milliseconds later, and none
of them asked for anything. The channel carries what a page shows around its
conversations; an open conversation's transcript, phase and commands stay on
that conversation's own [stream](../agent/runtime.md#frame-protocol).

The upgrade requires the session cookie. The channel shows the user's state
to whatever reads it, so an upgrade from a page that is not the product's
answers 403 `forbidden_origin` ([Authentication](#authentication)). A request
that is not an upgrade answers 426 `upgrade_required`.

After the upgrade, the backend sends JSON text messages, each a `SyncEvent`,
and the page sends none. The first message is the whole product state; each
later one is the current value of one part of it that changed:

| Message | Carries | Sent when |
| --- | --- | --- |
| `snapshot` | `state`, the product state below | First, on every connection |
| `conversation` | `conversation`, the conversation's summary as the conversation lists carry it | Its record changes: a patch or a batch item, an archive, a restore or a target switch once it completes, a read acknowledgement, a draft saved, restored or dismissed, a message sent, a title requested or written. It is created or forked. Its tree saves a checkpoint, starts or stops working, or is disposed. A permission request is raised, decided, replaced or withdrawn |
| `conversation_order` | `ids`, the id of every conversation, in the product state's order | A conversation is created, forked, moved, pinned or unpinned, archived or restored |
| `preferences` | `preferences` | A preferences patch |
| `user` | `user` | The nickname or the email address changes |
| `workspaces` | `workspaces`, in the user's order | A workspace is created, renamed, moved or deleted |
| `devices` | `devices`, the paired ones and the Cloud's | A device is paired or revoked, its runner connects or disconnects, its runner's installed artifacts change, or the Cloud's device is made |
| `providers` | `providers`, each with its details | An entry the user infers with, or an account of it, is created, changed or removed, a sign-in completes, an account's credential is renewed, or its quota snapshot is stored |
| `cloud` | `cloud` | The Cloud's lifecycle or its reset moves |
| `plugins` | `plugins`, the plugin list below | The user turns a plugin on or off |
| `subagents` | `subagents`, the user's [Subagent switch and profiles](#subagents) | The switch is turned on or off; a profile is created, changed, enabled, disabled or deleted |
| `plugin` | `plugin`, the plugin's id, and `state`, its user state | The plugin marks its user state changed ([The page](../architecture/plugins.md#the-page)) |
| `heartbeat` | Nothing | 30 seconds pass without another message |

The product state holds the current user, the instance mode, preferences, the
provider entries of the user's scope, workspaces, devices (the paired ones and
the user's Cloud device, which the file and working-tree routes address
alike, each with `installed`, the artifacts its runner's cache holds, as the runner last
reported them and kept while it is away, each as `{ package, name, version }`
([Installed artifacts](../execution/native-runtime.md#installed-artifacts))),
the summaries of the active and then the
archived conversations, the Cloud's state, `subagents`, the user's
[Subagent switch and profiles](#subagents), `plugins`, the plugin list, and
`pluginStates`, the user state of each plugin the user has on that declares
one, by plugin id, `publicUrl`,
the URL runners connect to (`DEMI_BACKEND_PUBLIC_URL`), `webBuild`, the
build of the web app the backend serves, or null when it serves none
([A page of another build](web-application.md#a-page-of-another-build)), and
`run`, the id of this run of the backend
([Revisions counted in memory](#revisions-counted-in-memory)). Each provider entry carries its
`details`: `{ type: "read", ... }` with what `GET /api/providers/:id/status`
answers, or `{ type: "failed", message }` for an entry whose provider could
not be read, which leaves the others intact. Reading the state never starts
the Cloud or runs inference. It is not one atomic read across the control and
conversation databases, and it need not be: a change made while it is read is
sent after it.

Drafts and the plugins' conversation states are not sent: they can be
large, and only the pages that show their conversation need them. The summary
carries their revisions, and a page that shows the conversation reads each
when its revision is higher than the one it holds
([Conversation drafts](#conversation-drafts),
[Conversation state of plugins](#conversation-state-of-plugins)).

**Order.** The backend reads a part when it sends it, after the change that
caused the message, and reads it again for a change made meanwhile. So a
part's messages come in the order of its changes, and a later one is never
older than an earlier one.

**Changes no write makes.** Each message follows a change the backend
commits, except one: an expose expires when its time comes, so the channel
also sends the `plugin` message of each plugin whose user state follows the
`exposes` topic, the `expose` plugin, when the earliest expiry passes
([Topics](../architecture/plugins.md#topics)). No other part
changes with time alone. A provider entry's `details`
report each account's sign-in as it is stored, whether or not the vendor
would still take it: an API key the vendor revoked, a Codex or Grok Build
sign-in whose refresh the vendor refuses, and a Claude Code setup token past
its life all read as they were stored. The backend learns of such a lapse
only when a request uses the credential. That request fails with the
provider's reason, in the conversation that sent it, and the stored sign-in,
with the entry's `details`, stays as it was. So a lapse changes nothing a
page shows, and needs no message.

**A page that falls behind.** The backend keeps no queue of messages for a
channel: it keeps the set of parts that changed since it last sent them, and
sends each of them once, as it is when the page takes more. For example, the
user types a draft on the laptop while the phone's page reads slowly: the
phone may never see some of the draft's revisions in between, and receives
the conversation's summary with the latest one once it reads again. A page
is never closed for falling behind and loses no change, and its channel costs
the backend at most one entry per part.

**Reconnecting.** A page whose channel closed connects again and receives a
new `snapshot`, which replaces everything it held. The backend registers the
channel for changes before it reads the snapshot, so each change is in the
snapshot or sent after it, and a page that was away misses nothing.

The backend closes the channel with a code and a reason:

| Code and reason | When |
| --- | --- |
| 1001 `backend_closing` | The backend shuts down; the page shows the restart screen until it is back ([A page of another build](web-application.md#a-page-of-another-build)) |
| 1008 `unexpected_message` | The page sent a message |
| 1011 `internal_error` | A part could not be read |
| 4002 `session_ended` | The session the channel opened with ended: it was signed out, or it expired |

The channel never renews its session; only requests do
([Authentication and ownership](../backend/backend.md#authentication-and-ownership)).

### Revisions counted in memory

Two revisions of a conversation's summary count changes in the backend's
memory rather than in storage: each of `pluginRevisions` and
`permissionsRevision`. The answers of the
permissions and plugin state reads carry the same counts. A backend that
starts again counts from 0, so a count compares only with counts of the same
run. The product state's `run` names the run: an id the backend chooses when
it starts.

A page holds each such revision together with the `run` of the snapshot it
held when it took it, and a revision of another run is older than any
revision of the current one. For example, a page shows `c_81` and holds its
requests at `permissionsRevision` 5. The backend restarts; the page connects
again, and the snapshot carries a new `run` and `permissionsRevision: 1`. The
page reads the requests again, though 1 is lower than 5. Comparing the
numbers alone, it would keep the requests it held until the count passed 5.

An answer belongs to the run of the snapshot the page holds when it arrives:
a run's answers are all sent before it exits, and the next run starts, and
sends its first snapshot, only after that.

## A user's plugins

The plugin list is every plugin of the backend, in its order of
registration, each with `id`, `name`, `description`, `enabled`, `packages`,
the ids of the command packages its commands, user streams and page methods
bind that the backend's catalog serves. `PUT /api/plugins/:plugin { enabled }` turns a plugin on or off
for the caller and answers 204; the new list and the states of the plugins
that changed reach every page of the user on the synchronization channel. An
unknown plugin answers 404 `unknown_plugin`. What each change does, and when, is
[A user's plugins](../architecture/plugins.md#a-users-plugins)'s.

`POST /api/conversations/:id/reload`, without a body, closes the
conversation's tree and opens it again with the user's current plugins, and
answers 204. A conversation socket that was attached to the tree receives
`closed` and connects again, as when another page's socket disposed the tree
([Connections and the live tree](../agent/runtime.md#connections-and-the-live-tree)). A
conversation whose tree is not open answers 204 and changes nothing: it opens
with the current plugins anyway. A tree that works answers 409
`turn_in_flight`, and an archived conversation 409 `conversation_archived`.

## Conversation state of plugins

A plugin's state for one conversation reaches a page by revision, as a draft
does. For example, the agent runs `demi browser open https://example.com/` in
conversation `c_81`. When that job ends, the `browser` plugin's conversation
state, the tab list, follows the `jobs` topic, so its revision rises from 6 to
7, and every page of the user receives the conversation's summary with
`pluginRevisions: [{ "plugin": "browser", "revision": 7 }]`. A page that shows `c_81` and holds
revision 6 reads the state:

```text
GET /api/conversations/c_81/plugins/browser/state

200 { "revision": 7, "state": { "tabs": [ { "id": "t3", "title": "Example Domain", ... } ] } }
```

The state matches the conversation state schema of the plugin's manifest. A
page takes an answer only when its revision is higher than the one it holds,
since a call's answer and a read can reach it in either order. The revision
counts the changes the plugin marked, in the plugin host's memory, so it
compares only within one run of the backend
([Revisions counted in memory](#revisions-counted-in-memory)). An unknown plugin answers 404
`unknown_plugin`, one without a conversation state 404 `unknown_plugin`, and a
plugin the user has off 409 `plugin_disabled`; the route needs a conversation
the user owns, as every conversation route does. A read is admitted as a
user stream is, so it never wakes a stopped Cloud: the plugin answers what it
can without it, as the browser's tab list of a stopped Cloud is empty
([The tab methods](../browser/live-view.md#the-tab-methods)). A refusal of the
conversation's host access, or a plugin that fails, answers as a plugin call
does ([Plugin calls](#plugin-calls)).

## Plugin calls

A plugin's page calls its plugin through one route, or, for a call about one
conversation, through that conversation's
([The page](../architecture/plugins.md#the-page)). For example, the Skills
settings section adds a source, and the work panel's `browser` kind opens a
tab in a conversation's browser:

```text
POST /api/plugins/skills/calls/add_source
{ "origin": "vercel-labs/agent-skills" }

200 { "source": "src_7fq2" }

POST /api/conversations/c_81/plugins/browser/calls/open
{ "url": "https://example.com/" }

200 { "tab": { "id": "t3", "title": "", "url": "https://example.com/", "createdBy": "user" } }
```

The body is the method's parameters, and the response's body is its result.
The plugin's manifest declares each method with a JSON Schema for its
parameters and one for its result, and its page package's types are
generated from the same Rust types. The routes work like the other
session routes ([Authentication](#authentication)); the conversation's route
also needs a conversation the user owns, as every conversation route does:

| Situation | Answer |
| --- | --- |
| The backend has no plugin of that id | 404 `unknown_plugin` |
| The plugin has no method of that name for the route: a method of the user scope called for a conversation, or the other way round, has none | 404 `unknown_plugin_method` |
| The user has the plugin off | 409 `plugin_disabled` |
| The body is not JSON, or does not match the method's parameter schema | 400 `invalid_body`, naming the field and the reason |
| The plugin refuses the call, such as a source already added or a tab the browser does not have | 409 `plugin_refused`, with the plugin's own `reason`, a snake_case word such as `tab_not_found`, and its message |
| A port operation the plugin made was refused by the conversation's host access, and the plugin passes the refusal on | That refusal's own answer: 409 `conversation_archived`, `device_offline`, `host_stopped` or `conversation_busy` |
| The plugin fails, such as a value that does not read or a package call whose operation failed | 500 `plugin_failed`, with the plugin's message |

A call changes only the calling user's state. What it changed reaches every
page of the user as the plugin's new user state on the synchronization
channel ([Page synchronization](#page-synchronization)), or as a higher
revision of its [conversation state](#conversation-state-of-plugins),
including the page that called, so a page shows what it called for the way
it shows every other change.

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
all referenced devices before adding any attachment grant. It attaches non-primary
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
`{ path, text }` with the file's ETag, the one the raw route reports; a
request whose `If-None-Match` names the file's current ETag answers 304
without reading the text past the Host, since the runner compares the version
before it streams ([File contents](../execution/runner.md#file-contents)). The
files service checks a text it keeps this way
([What the service keeps](../architecture/plugin-pages.md#what-the-service-keeps)). Paths follow the Host's filesystem rules; the execution directory
is a starting directory, not a permission boundary. Missing paths answer 404,
and permission failures answer 403. A directory too large to list in one
runner message ([Runner](../execution/runner.md#connection-and-identity))
answers 413 `directory_too_large`.

All three operations use [conversation Host access](../execution/sessions-and-targets.md#host-operations),
including Cloud wake and the file gate. Archived conversations answer 409
`conversation_archived`, and a conversation whose target device no longer
exists answers 404 `device_not_found`. A paired device without a live runner answers 409
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
answer carries `X-Content-Type-Options: nosniff`, `Vary: Cookie`, `Cache-Control:
private, no-cache`, or `private, max-age=31536000, immutable` for a request
that names its `version`, whose bytes can only be that version's, and
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
A directory at the path answers 409 `is_directory`, and a file there answers
409 `file_exists` unless `replace=true`; the Host checks as it writes, and no
check precedes the write ([Host operations](../execution/runner.md#host-operations)).
The write makes the missing directories above the path, so an uploaded
folder's files make its folders, and the web app makes only an empty folder
with `POST`. Success answers 204. An upload is a file transfer whose pace the
user's browser sets, under the same Host access, stall rule and ending as a
download
([Host operations](../execution/sessions-and-targets.md#host-operations)).

`DELETE /api/conversations/:id/fs?path=...` deletes a file, or a directory
with everything in it, and answers 204; nothing at the path answers 204 too.
It refuses, with 409 `protected_path`, the filesystem root, the Host's home
directory, the conversation's execution directory, and any directory that
holds one of them: the file tree deletes only what an upload replaces inside
the workspace.

The remote attachment picker lists and creates directories through
`GET/POST /api/conversations/:id/hosts/:deviceId/fs`, with the same directory
contract. The device must be the conversation's primary or an attached host,
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
504 `changes_timeout`, and the page then keeps its previous list and says
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

The page lists the working tree again when the [file watch](#file-watch)
reports a path in it or under its repository's `.git`, and on its Refresh
control ([What the service keeps](../architecture/plugin-pages.md#what-the-service-keeps)).
It never polls.

### File watch

`WS /api/conversations/:id/fs/watch` carries the Host's reports of what
changed in the conversation's files to a page that shows them. The page holds
one while a component shows the conversation's files, and closes it when none
does. The upgrade checks the session cookie and the `Origin` header as the
browser stream does, and is admitted through the conversation's host access
without waking a stopped Cloud.

The backend watches the conversation's working tree on the Host for it, and,
non-recursively, each folder or file outside that tree the page names, up to
64 ([Watching files](../execution/runner.md#watching-files)). Pages that watch
the same Host paths share one watch on the Host. The page sends JSON text
messages:

| Message | Carries |
| --- | --- |
| `paths` | `paths`, the folders and files outside the working tree the page shows now; each message replaces the last |

The backend sends:

| Message | Carries | Sent when |
| --- | --- | --- |
| `state` | `state`: `live`, `lost`, `unavailable` or `offline`, and for `unavailable` the Host's `reason` | `live` once the Host's watches run, so what is read from then on is covered, and again once the watches of each `paths` message run, so the page trusts a path outside the working tree only after the `live` that answers the message naming it; `lost` when a watch lost reports, so nothing read before is confirmed, followed by `live` again; `unavailable` when the Host cannot watch, after which the socket sends no other state; `offline` while the Host is out of reach or a stopped Cloud |
| `changed` | `paths`, absolute paths on the Host that changed | The Host reports them, at most every 100 ms, a path once per message. More than 1,000 paths at once come as `lost`. |
| `heartbeat` | Nothing | 30 seconds pass without another message, so the page tells a quiet watch from a dead one ([Liveness and reconnection](web-application.md#liveness-and-reconnection)) |

The backend closes the socket with the user streams' codes: 1011
`host_unreachable` when the Host became unreachable, 4000
`conversation_changed` when an archive, a target or directory change, or a
detach ended the watch, and 1003 `invalid_message` for a message the page
should not have sent.

A watch that ends, by a closed socket, a lost connection to the Host or an
archive, a target change or a detach, ends with the socket; the page opens a
new one after its [reconnect waits](web-application.md#liveness-and-reconnection),
and treats what it read before as unconfirmed until `live` arrives again.


## Direct channel

`WS /api/devices/:deviceId/direct` is the signaling socket of a
[direct channel](../execution/direct-channel.md) between the page and a paired
device's runner. The upgrade checks the session cookie and the `Origin`
header as the other sockets do, and that the caller owns the device; a
device that is the caller's Cloud answers 409 `not_a_paired_device`, and a
device whose runner is not connected 409 `device_offline`. Messages are JSON
text.

The page sends:

| Message | Carries | Sent when |
| --- | --- | --- |
| `offer` | `sdp` | The page makes a peer: at first, and on each new attempt |
| `use` | `conversation` | The page shows a conversation whose primary Host is this device |
| `release` | `conversation` | It no longer shows it |

The backend sends:

| Message | Carries | Sent when |
| --- | --- | --- |
| `answer` | `sdp` | The runner answered the offer |
| `unanswered` | `code` | The runner refused the offer (`busy`) or did not answer within 10 seconds |
| `granted` | `conversation` | `use` was admitted and the runner holds the grant |
| `refused` | `conversation`, `code` | `use` was not admitted: `conversation_archived`, `not_found`, or `not_on_device` when the conversation's primary Host is another device |
| `revoked` | `conversation` | A transition took the grant back |
| `heartbeat` | Nothing | 30 seconds pass without another message |

A new `offer` replaces the socket's peer: the runner closes the old one. The
socket closes with 1011 `host_unreachable` when the runner's connection
ends, and 1003 `invalid_message` for a message the page should not have sent.

## Serving the web app build

When its [server release](../delivery/builds-and-releases.md#server-release)
has a `web/`, the backend serves that built web app directory alongside the
API and conversation sockets. The directory's `build.json`,
`{ "build": "<id>" }`, which `vite build` writes, names the build; the
backend reads it as it starts, refuses to start without it, and sends it in
every snapshot as `webBuild`
([A page of another build](web-application.md#a-page-of-another-build)). Extensionless HTML navigations
fall back to index.html, enabling deep-page refresh. Missing assets and
`/api/*` misses remain 404. The product frontend builds separately and uses a
Vite proxy during development; its fetch and WebSocket adapters connect the
shared UI to this backend.
