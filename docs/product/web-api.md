# Web API reference

The web app and backend ship together, so routes have no version prefix.
Application data uses HTTP JSON, each page follows the user's state through
its [synchronization channel](web-api-sync.md#page-synchronization), and an open conversation
uses the agent WebSocket protocol.
[Backend architecture](../backend/backend.md) defines authentication and
transport boundaries. This reference owns product endpoint contracts;
domain rules remain in the linked topic documents.

The routes are grouped by what they serve:

- [Conversations](web-api-conversations.md): creation and Fork, history, drafts, saved messages, permissions, credentials, workflows, the sidebar, sharing and search.
- [Hosts and files](web-api-hosts.md): workspaces, devices, uploads, the Cloud, user streams, device files, file text and the direct channel.
- [Accounts and settings](web-api-settings.md): accounts, preferences, subagents, instructions and providers.
- [Page synchronization and plugins](web-api-sync.md).

The `web-api-protocol` crate defines every JSON body of this reference, request and
response, every message of the synchronization channel, and every error code;
the conversation stream carries the agent protocol's frames
([Frame protocol](../agent/rendering.md#frame-protocol)). The backend decodes
each request into its type and serializes each response from its type, and
the web app's REST types and schemas are generated from the same types
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
A response carries the fields its type declares and nothing more: fields the
backend keeps for itself in a stored record, such as a conversation's owner or
its message counts, never reach the web app.

## Resource index

Paths in the index are relative to `/api`, except the public installation row.
Detailed sections use the same relative paths or spell out the `/api` prefix.
Every response carries `X-Request-Id`, and JSON errors carry
`{ code, message, params?, requestId }`, `params` the values the page's text for the code needs ([Localization](localization.md#text-from-the-backend)) ([Request IDs and the server log](../backend/backend.md#request-ids-and-the-server-log)). `code` is a value of `ErrorCode`, the
`web-api-protocol` crate's list of every code the web app can see, which the web
app receives as a string union. A situation has one code on every route:
for example, an archived conversation that refuses an operation always answers
`conversation_archived`, and a conversation whose database the server cannot
read answers 500 `conversation_unreadable` on every route that reads it, its
history, its attachments and its read acknowledgement, the full error going
to the server's log only. A `message` is for the log and the developer; the
page words what the user sees from the `code`, so no server path or operator
command reaches the user. Validation normally returns 400, missing or
inaccessible objects 404, and an operation refused by current state 409.
Partial conversation mutations use the explicit outcomes described below.

| Resource | Routes and request shape |
|---|---|
| Setup | `GET /setup` returns `{ needed }`; `POST /setup` creates the first master account and signs it in |
| Authentication | `POST /auth/login`, `POST /auth/logout`, `GET/PATCH /auth/me`, `PUT /auth/password`, `POST /auth/email`, `POST /auth/email/confirm` |
| Users | `GET/POST /users`, `PATCH /users/:id`; role hierarchy restricts administration |
| Page synchronization | `WS /sync` sends the product state, then each part of it that changes ([Page synchronization](web-api-sync.md#page-synchronization)) |
| Settings | `GET /settings` returns fixed instance mode; `GET/PATCH /settings/preferences` |
| Subagents | `PUT /subagents { enabled }`; `POST /subagents/profiles`, `PATCH /subagents/profiles/:id`, `DELETE /subagents/profiles/:id`; the switch and the profiles are part of the product state ([Subagents](web-api-settings.md#subagents)) |
| Instructions | `PUT /instructions { text }`; the text is part of the product state ([Instructions](web-api-settings.md#instructions)) |
| Conversations | `GET /conversations?archived=true\|false`, `POST /conversations { id, ... }`, `PATCH /conversations/:id`, `DELETE /conversations/:id`, `POST /conversations/batch`, `POST /conversations/:id/fork { id, blockId }`, `POST /conversations/:id/read { revision }` |
| Search | `GET /search?q=<query>` finds the caller's conversations ([Search](web-api-conversations.md#search)) |
| Sharing | `POST /conversations/:id/shares { kind, blockId? }`, `PUT /conversations/:id/shares/:token`, `DELETE /conversations/:id/shares/:token`; public `GET /shares/:token`, `GET /shares/:token/transcript?before=\|after=\|around=`, `GET /shares/:token/transcript/blocks/:blockId`, `GET /shares/:token/blobs/:sha256` ([Sharing](web-api-conversations.md#sharing)) |
| Conversation history | `GET /conversations/:id/transcript?node=&before=\|after=\|around=` reads one page of an agent's transcript, `GET /conversations/:id/transcript/blocks/:blockId?node=` one block whole, `GET /conversations/:id/subagents` the conversation's subagents and `GET /conversations/:id/commands/:commandId` where the call that started a command lies ([Conversation history](web-api-conversations.md#conversation-history)); `WS /conversations/:id/stream` carries the [agent frames](../agent/rendering.md#frame-protocol) of that one conversation |
| Conversation files | `GET/POST /conversations/:id/fs`, `DELETE /conversations/:id/fs?path=...`, `GET /conversations/:id/fs/file?path=...`, `POST /conversations/:id/fs/exists`, `WS /conversations/:id/fs/watch`, `GET /conversations/:id/fs/raw?path=...&version=...&download=true\|false`, `PUT /conversations/:id/fs/raw?path=...&replace=true\|false` with raw bytes, `GET/POST /conversations/:id/hosts/:deviceId/fs` |
| Working tree | `GET /conversations/:id/changes`, `GET /conversations/:id/changes/file?path=...`, `GET /conversations/:id/changes/raw?path=...&download=true\|false` |
| User streams | `WS /conversations/:id/streams/:name` opens a declared [user stream](web-api-hosts.md#user-streams) |
| Work panel | `GET /conversations/:id/panel` reads the [work panel's tabs](web-api-conversations.md#work-panel-state); `POST /conversations/:id/panel/changes { changes }` changes them |
| Conversation draft | `GET/PUT /conversations/:id/draft` reads and saves the [draft](web-api-conversations.md#conversation-drafts); `POST /conversations/:id/draft/replaced { action, revision }` restores or dismisses the version a save replaced |
| Conversation permissions | `GET /conversations/:id/permissions` reads the conversation's [permission requests](web-api-conversations.md#conversation-permissions); `POST /conversations/:id/permissions/requests/:request { decision }` decides a request |
| Device log | `GET /devices/:id/log?since=<cursor>&limit=<n>&source=<source>` reads the [Host's log](../execution/runner.md#host-log) |
| Direct channel | `WS /devices/:deviceId/direct` introduces the page to a paired device's runner ([Direct channel](web-api-hosts.md#direct-channel)) |
| Sidebar | `POST /sidebar/reorder { kind, id, beforeId }` |
| Plugins | `PUT /plugins/:plugin { enabled }` turns a plugin on or off for the caller ([A user's plugins](web-api-sync.md#a-users-plugins)); `GET /conversations/:id/plugins/:plugin/state` reads a plugin's [state for one conversation](web-api-sync.md#conversation-state-of-plugins); `POST /plugins/:plugin/calls/:method` and `POST /conversations/:id/plugins/:plugin/calls/:method` with the method's parameters call a [plugin's page method](web-api-sync.md#plugin-calls), for the user or for one conversation; `POST /conversations/:id/reload` reopens the conversation's tree with the user's current plugins |
| Models | `GET /models?refresh=true\|false` returns the account-wide catalog |
| Providers | `GET /providers/catalog`, `GET/POST /providers`, `PATCH/DELETE /providers/:id`, `GET /providers/:id/status`, `POST /providers/:id/test`, `POST /providers/:id/quota`; account routes below |
| Usage | `GET /usage` for the caller; `GET /usage/instance` for admins in shared mode |
| Diagnostics | `POST /page-errors { title, message, detail }`, `GET /server/log?request=&since=&limit=&level=` (administrator) |
| Devices | `GET /devices`, `POST /devices/claim { code }`, `PATCH /devices/:id { name?, route? }`, `DELETE /devices/:id`, `GET /devices/:id/fs?path=<absolute>`, `POST /devices/:id/fs { path }` |
| Workspaces | `GET/POST /workspaces`, `PATCH /workspaces/:id { name }`, `DELETE /workspaces/:id` |
| Cloud | `GET /cloud`, `POST /cloud/reset { operationId }` |
| Attachments | `POST /attachments` with raw bytes; `GET /blobs/:sha256?type=...` |
| Attached hosts | `GET /conversations/:id/hosts`, `DELETE .../hosts/:deviceId`; agents attach with `demi host attach` ([What the agent can change](../execution/sessions-and-targets.md#what-the-agent-can-change)) |
| Runner transport | `WS /runner`; device-authenticated pipes at `/pipes/:id`: `PUT` for a content pipe and `WS` for a stream pipe from the runner, `GET` for a pipe to it ([Host operations](../execution/runner.md#host-operations)) |
| Public installation | `GET /install.sh`, `GET /install.ps1`, `GET /runner-artifacts/:release/:target/:file`, and, with a local object store, the command artifacts, `GET /native-artifacts/:sha256` ([Backend deployment configuration](../execution/native-runtime.md#backend-deployment-configuration)) (root paths, outside `/api`) |

### Authentication

Each route authenticates its caller in one of these ways
([Authentication and ownership](../backend/backend.md#authentication-and-ownership)):

| Routes | Authentication |
|---|---|
| `GET/POST /setup`, `POST /auth/login`, `POST /auth/passkey/options`, `POST /auth/passkey`, `POST /auth/handoff` | None; a successful setup, login, passkey sign-in or handoff sets the session cookie |
| `WS /sync` | The session cookie, which the route checks without renewing the session |
| Every other path under `/api`, unknown paths included, except the runner transport | The session cookie, which the session gate checks |
| The runner transport, `WS /runner` and the pipes at `/pipes/:id` | A runner's device token; an unpaired runner's socket waits without one until a user claims its code |
| The public share routes under `/shares/:token`, whose token is the key ([Sharing](web-api-conversations.md#sharing)) | None; a session cookie is ignored |
| The public installation routes and the [web app build](#serving-the-web-app-build) | None |

The first three rows are the web app's routes. Such a route answers 403
`forbidden_origin` to a request that could act with the user's session and
comes from a page that is not the product's: a request whose method is not
GET, HEAD, OPTIONS or TRACE, or an upgrade, such as a WebSocket's. It answers
before any other check, so the request changes nothing and is refused with or
without a session. A request without `Origin` does not come from a web browser
and passes, so `curl` can call `POST /api/setup` without one.

### Request bodies

The backend reads a JSON body whole, up to 1 MiB, and an attachment's bytes up
to 25 MiB ([Uploads and media](web-api-hosts.md#uploads-and-media)); a larger body answers 413
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

## Serving the web app build

When its [server release](../delivery/builds-and-releases.md#server-release)
has a `web/`, the backend serves that built web app directory alongside the
API and conversation sockets. The directory's `build.json`,
`{ "build": "<id>" }`, which `vite build` writes, names the build; the
backend reads it as it starts, refuses to start without it, and sends it in
every snapshot as `webBuild`
([A page of another build](web-application.md#a-page-of-another-build)). Extensionless HTML navigations
fall back to index.html, enabling deep-page refresh. `index.html` and
`build.json` are answered with `Cache-Control: no-cache` and an `ETag` of
their content, so a browser or a cache in front of the backend asks again
on every load and gets the new build's page even when its files are older
than the old build's, as after a rollback; the hashed assets are immutable. Missing assets and
`/api/*` misses remain 404. The product frontend builds separately and uses a
Vite proxy during development; its fetch and WebSocket adapters connect the
shared UI to this backend.
