# Backend architecture

The backend is the product server, one Rust executable named `demi-backend`
([Builds and releases](../delivery/builds-and-releases.md) lists its targets).
It authenticates browser requests, hosts conversation agent trees, assembles
providers and commands, and connects those sessions to devices through their
runners. Runners execute device work; the backend owns conversation state and
product policy.

## Request paths and responsibilities

Two parts of the backend appear in every path. The edge accepts connections,
authenticates requests and copies bytes. A user's shard holds everything the
backend decides for one user: that user's conversations, agent trees, devices
and Cloud. [Runtime model](#runtime-model) explains both.

```text
Browser
  +-- HTTP product data ----> edge ---> a shared service, or the user's shard
  +-- conversation socket --> edge ---> user's shard: agent tree -> conversation database
                                                     |
                               +---------------------+---------------------+
                               v                                           v
                       provider runtime                     the conversation's host access
                         |          |                                      |
                   vendor HTTP API  +-- process on the user's Cloud ---+   |
                                                                       v   v
                                                                runner connection
                                                                       |
                                                                     device
```

For example, opening history reads stored blocks without starting a runner.
Sending a message validates its frame and provider selection. Inference can
use a backend HTTP provider. A shell tool reaches the conversation's Host
through the conversation's host access, and a provider that needs a process
runs that process on the user's Cloud
([Claude Code](../providers/claude-code.md#where-it-runs)). The same
conversation keeps its history if its target changes.

Besides the browser, three kinds of client reach the backend: runners, over a
WebSocket and HTTP pipes authenticated by their device token; anonymous
visitors of an expose hostname, whom the public relay serves; and anyone who
downloads the runner installers. The backend itself calls the machine manager
over the manager's Unix socket.

| Module | Responsibility | Design contract |
|---|---|---|
| `edge` | The listener and router, the session gate, request extractors and body limits, error codes, installer and browser-asset routes, and the byte copies of file transfers, pipes, user streams and the expose relay | [Web API](../product/web-api.md) |
| `shard` | Shard threads, each user's shard, calls into it, socket adoption, leases | [Runtime model](#runtime-model) |
| `config` | The typed configuration, validated at startup | [Configuration](#configuration) |
| `auth` | Accounts, password hashing, web sessions, login lockout, email-change delivery | [Authentication and ownership](#authentication-and-ownership), [Product](../product/product.md#user-system) |
| `settings` | Per-user preferences | [Web API](../product/web-api.md#user-preferences) |
| `sync` | The reconstructible application snapshot for browser polling | [Browser synchronization](#browser-synchronization) |
| `conversation` | Agent-tree hosting, frame scoping, attachment and remote-file references, history and Fork, summaries and titles, target resolution and transitions, the conversation's host access, file transfers and user streams | [Sessions and targets](../execution/sessions-and-targets.md) |
| `runner` | Pairing, device links and runner connections, the rpc relay and each session's commands, the product's `demi host` group, installer scripts, native artifact publication | [Runner](../execution/runner.md), [Commands](../execution/commands.md) |
| `lifecycle` | The conversation idle clock and the conversation release | [Conversation idle and Host resource release](../execution/resource-lifecycle.md) |
| `managed` | Cloud policy and capacity, machine transitions, reset and recovery, the machine manager's client | [Managed hosts](../cloud/managed-hosts.md) |
| `expose` | Expose records and their lifetime, live relay connections, the `demi host expose` leaves | [Host expose](../execution/expose.md) |
| `llm`, `vault`, `usage` | Provider assembly and model catalogs; credential records, scope and login flows; metering and the request rate limit | [Providers](../providers/providers.md), [Models](../providers/models.md), [Usage and quota](../providers/usage-and-quota.md) |
| `storage` | The control service, conversation databases and the tree store, the object store | [Storage](storage.md) |

These are modules of one backend, not independently deployed services.
[Crates and packages](../architecture/crates-and-packages.md#crates) names the
crates the backend builds on.

## Runtime model

The edge is a multi-threaded Tokio runtime that serves HTTP with axum. A shard
thread is a single-threaded Tokio runtime that hosts the shards of every user
pinned to it. Each open SQLite connection has a thread of its own, and other
blocking work runs on Tokio's blocking pool.
[Programs and threads](../architecture/concurrency.md#programs-and-threads)
draws the whole program.

For example, when a user sends a message, the edge authenticates the
conversation socket's upgrade and moves the socket into the user's shard. The
agent tree that runs the turn, the conversation's host access that each tool
call passes, and the runner connection that carries the tool's job all live in
that shard. The edge takes part again only when the shard hands it bytes to
copy, such as those of a download
([Host operations](../execution/sessions-and-targets.md#host-operations)
follows one step by step).

Each module's state lives in one of these places:

| Place | Holds | Examples |
|---|---|---|
| Edge | No per-user state | Request parsing and authentication, ownership checks, and the byte copies of file transfers (with their 60-second stall rule), pipes, user streams and the expose relay |
| Shared services | State that spans users, or that is needed before the user is known | The control service, the conversation stores, the object store, the vault, provider assembly with model catalogs and one credential refresh at a time per account, the machine manager's client, [Cloud capacity](../cloud/managed-hosts.md#lifecycle-and-capacity) across users, runners waiting to be paired, login lockout |
| A user's shard | Everything the backend decides for that user | Each conversation's file gate, open transfers and user streams, and idle watch; agent trees; device links and one task per runner connection; pipe records; the Cloud machine; live expose connections; title requests; Fork requests, one at a time per destination; each session's command router for the rpc relay; the request rate limit |
| Database threads | One per open SQLite connection | The control database; up to 64 conversation writer connections ([Storage](storage.md#conversation-state-and-transactions)) |
| Blocking pool | Work that would stall an async thread | Disk IO; password and blob hashing; serializing request bodies with media and large transcript frames; read-only conversation reads |

[The user shard](../architecture/concurrency.md#the-user-shard) gives the
rules this placement follows: why all of a user's work shares one thread, what
crosses between the edge and a shard, and how a call ends when its requester
goes away. A call that panics answers 500 `internal_error`, and the shard goes
on serving.

A stable hash of the user id pins each user to one shard thread, and the
backend starts with one shard thread. A user's shard is created by the first
request for that user and loads only the user's Cloud device record and its
reset intent. Conversations, agent trees and devices load when first used, and
an agent tree that no socket watches closes again once it is idle
([Connections and the live tree](../agent/runtime.md#connections-and-the-live-tree)).
The machine manager's death events reach one edge task, which calls the shard
of the device's owner.

The edge uses axum because its handlers are thin: parse, authenticate, check
ownership, then call a shared service or a shard. axum's requirement that a
handler's future be `Send` therefore costs nothing, and its extractors, tower
middleware and socket-free router tests come with it. axum serves over the
backend's own listener, which gives every connection an idle deadline and a
close handle and exposes the peer address. A download arms the 60-second
deadline, a lease the shard ends closes the connection at once even when the
browser has stopped reading, and the expose relay forwards the peer address.

## Authentication and ownership

Browser API routes use the `demi_session` cookie. It contains a random 256-bit
token whose SHA-256 identifies the stored session. The cookie is `HttpOnly`,
`SameSite=Lax`, and `Path=/`; HTTPS requests, including forwarded HTTPS, set
`Secure`. A missing or expired session returns 401 `unauthenticated`; an
invalid supplied cookie is cleared.

Sessions expire 30 days after their last renewal. A request with less than
15 days remaining renews the session and cookie.

Login failures are counted per email address. For example, five wrong
passwords for `ana@example.com`, each within a minute of the one before, lock
that address for one minute, during which even the right password answers 429
`too_many_attempts`. An address without an account counts too, and a
successful login clears the count. The lockout is separate from the inference
rate limit and lives in the memory of the backend process. A login can reach
any worker of a multi-worker deployment, so there each worker counts an
address's failures on its own.

Passwords are hashed with argon2id
([Storage](storage.md#passwords-and-credentials-at-rest)). Hashing runs on the
blocking pool, no more hashes at once than the machine has CPUs, because each
hash holds its memory until it finishes. A login for an address without an
account verifies the password against a fixed dummy hash, so it takes as long
as a wrong password and its timing does not reveal which addresses have
accounts.

Setup and login are public entrances. Runner and pipe routes use device
credentials instead of browser cookies. Public installer downloads contain no
credential. All other `/api` resources, unknown paths included, pass through
the browser session gate, so an unauthenticated request for a path that does
not exist answers 401, not 404. Inaccessible user-owned objects return 404,
insufficient role returns 403, and missing authentication returns 401.

The edge checks ownership before it hands a request to a shard: it resolves
the caller from the cookie, loads the conversation, device, workspace or
provider entry the path names, and answers 404 when it belongs to someone
else. A request therefore only ever reaches its owner's shard, and the shard
checks a conversation's owner again whenever it loads the record.

A runner authenticates with its device token in the first message of its
WebSocket. The edge reads that message, finds the device by the token's hash,
and moves the socket into the owner's shard. A runner without a token is
unpaired: it waits at the edge and prints a claim code, which changes every 10
minutes while it waits. A signed-in user claims the code with
`POST /api/devices/claim`; the backend creates the device, sends its token to
the waiting runner only, and hands the socket to the user's shard. The edge
accepts at most 10 claim attempts per user per minute and answers 429 beyond
that. A Cloud guest always presents its token and never pairs
([Runner](../execution/runner.md#managed-guests-and-verification)). Pipe
requests carry the device token as a bearer token.
[Runner](../execution/runner.md#connection-and-identity) defines the
connection itself.

[Instance mode](../product/product.md#instance-mode-shared-vs-isolated)
controls provider ownership. Every provider lookup, catalog, configuration
route, and inference resolution uses the same scope.

## Browser synchronization

Ordinary product state uses REST with stable error codes and
`{ code, message }` errors. Every request and response type, and every error
code, is defined once in the contract crates and generated for the browser
([Generated TypeScript](../architecture/contracts.md#generated-typescript));
[Web API](../product/web-api.md) lists the routes.

Each open conversation uses one WebSocket at
`/api/conversations/:id/stream`, carrying the agent protocol's client and
server frames ([Frame protocol](../agent/runtime.md#frame-protocol)). After
the upgrade the socket moves into the user's shard, which serves it until it
closes. Execution context comes from the conversation's server-side target;
the browser cannot override it with an arbitrary frame cwd.

`GET /api/state` returns a conditional application snapshot: account,
instance mode, preferences, projects, conversation summaries, devices,
providers, exposes and Cloud status
([Web API](../product/web-api.md#sidebar-mutations-read-state-and-page-synchronization)).
It does not start Cloud or infer. The product browser polls and
revalidates this snapshot; chat output remains on the conversation stream. The
user's shard assembles the snapshot from several stores and its own live
state, so it is reconstructible rather than one global atomic read. A later
poll catches changes made during assembly.

How the browser consumes both, with its adapters, polling and session
handling, is defined in [Web application](../product/web-application.md).
Model discovery has its own account-wide cache and loading state, so it does
not block readable history ([Models](../providers/models.md#catalog-cache)).

## Media by reference

Transcript media sent to the browser uses blob references instead of inline
bulk bytes. The conversation socket externalizes inline media in root and
subagent reset and patch frames, preserving frame order. The browser retrieves
the referenced bytes through the cookie-authenticated blob route in its user's
namespace.

Only the media types [file previews](../product/file-previews.md#keeping-file-content-inert)
show in place are served inline, under the same content policy; the page and
the backend read one file-type table
([Logic the browser and backend share](../architecture/contracts.md#logic-the-browser-and-backend-share)).
Other content downloads as `application/octet-stream`; responses include
`X-Content-Type-Options: nosniff`, private immutable caching for one year, and
`Vary: Cookie`. A hash is not an authorization token across users.

An upload stores the file's bytes in the caller's blob namespace with an
attachment record. The backend reads the file's media type from its bytes,
and for a text file a short opening snippet; the upload response carries both
([Web API](../product/web-api.md#uploads-and-media)), so the composer shows
them without reading the file itself.

Send, steer and edit content refers to uploads and remote files with typed
`upload` and `remote_file` references; a frame never carries media bytes. The
socket validates the entire frame before changing metadata or granting
attachments. It resolves upload references to caller-owned blobs and writes
them under the selected Host's `~/.demi/attachments/<conversation>/`, outside
the workspace. The agent receives an attachment record, with the same snippet
for a text file, and any native media block it can consume. A missing or
inaccessible upload becomes an explicit attachment-unavailable text block.

A remote-file reference retains its device and absolute path instead of
copying bytes. The backend checks ownership and connectivity before adding
attached-host grants; the model reads the file later through the host command.
Its contents can change before that read.
[Web API](../product/web-api.md#device-files-and-remote-references) defines
the wire shape.

Resolving uploads and externalizing media keep the socket's frame order
([Order and delivery](../agent/runtime.md#order-and-delivery)). A frame whose
resolution fails reports an error without affecting the frames behind it, and
closing the socket stops resolution that has not reached the session yet. The
underlying persistence and blob ownership are defined in [Storage](storage.md).

## Failure facts

An error block keeps the vendor's failure record as it arrived; what the
browser shows from it is read when the block is sent
([Failures and recovery](../agent/failures-and-recovery.md#reading-a-failure)).
For every error block in a transcript it sends, the backend asks the provider
named in the block's model selection to read the record, and attaches the
result as `failures`, keyed by block id, beside the blocks:

- The conversation socket attaches it to root and subagent
  `transcript_reset` and `transcript_patch` frames, for the blocks each frame
  carries.
- `GET /api/conversations/:id/transcript` attaches it to the root blocks and to
  each subagent history.

One backend function reads the failures of a list of blocks, and both paths
call it. The facts are never stored, and the blocks themselves are sent
unchanged. A frame or history without an error block that yields a fact
carries no `failures`.

## Startup and shutdown

At startup the backend:

1. Reads its configuration and validates all of it; an error names the
   variable ([Configuration](#configuration)).
2. Loads the native command releases and publishes their artifacts
   ([Native runtime](../execution/native-runtime.md#publish-artifacts-before-enabling-commands)).
   An interrupt or termination signal during publication stops the start.
3. Opens the data directory and loads the instance secret.
4. Opens the control database; a new database receives its schema.
5. Starts the shared services and the shard threads.
6. Recovers before it serves: the machine manager reconciles its machines, an
   interrupted Cloud reset finishes committing its disks and is marked failed
   so that a retry starts the Cloud
   ([Managed hosts](../cloud/managed-hosts.md#system-reset)), and Fork
   creations whose destination root was committed are published
   ([Conversation Fork](../agent/conversation-fork.md#backend-creation-and-retries)).
7. Opens its listener.

Shutdown closes the listener first, so that no new work starts and no runner
reconnects into a backend that is closing. A new request on a connection that
is already open answers 503 `backend_closing`. Runner connections and pipes
keep working, because the steps below need them:

1. Login flows are cancelled.
2. Each shard ends its user's work in this order. Idle watches stop, and a
   retirement already running finishes. Title requests are aborted and expose
   connections end. Open file transfers and user streams end. Conversation sockets
   close, so no frame reaches a tree after its shutdown. Agent turns are
   aborted: a running turn records that its session was shut down, and its
   jobs are killed while their runners are still connected. Claude Code CLI
   installs are cancelled; the next need starts them again. The Cloud
   hibernates. Then the shard's runner connections close, and after them its
   pipes fail.
3. Runners waiting to be paired are disconnected, and the machine manager's
   client closes
   ([Control and ownership](../cloud/managed-hosts.md#control-and-ownership));
   the manager itself keeps running.
4. Connections still open, such as a download or a relayed visitor, are
   closed.
5. Model catalog refreshes and quota snapshot writes drain. The conversation
   databases close, then the control database.

Transfers end before the Cloud hibernates because an open download holds the
Cloud's gate, and a Cloud whose gate is busy would skip its save. Every step
runs even when an earlier one fails; the failures are reported together, and
the process exits with a failure status.

## Configuration

The backend reads its configuration from command-line flags or `DEMI_*`
environment variables through clap, which parses both into one typed
configuration. The whole configuration is validated before anything starts,
and an error names the variable. `demi-backend --help` lists the flags.

| Variable | Meaning | Defined in |
|---|---|---|
| `DEMI_BACKEND_DATA` | The data directory. Default `~/.demi/backend`. | [Storage](storage.md#ownership-and-layout) |
| `DEMI_BACKEND_PORT` | The TCP port the backend listens on, 1 to 65535. Default 3271. | — |
| `DEMI_INSTANCE_MODE` | `shared` or `isolated`. Required. | [Product](../product/product.md#instance-mode-shared-vs-isolated) |
| `DEMI_BACKEND_PUBLIC_URL` | The URL runners and Cloud guests connect to; installers embed it, and expose URLs take their scheme and port from it. Required. | [Cloud setup](../cloud/setup.md#configuration) |
| `DEMI_MACHINES_SOCKET` | The machine manager's Unix socket. Required: every deployment has Cloud. | [Cloud setup](../cloud/setup.md#configuration) |
| `DEMI_NATIVE_CONFIG` | The native command releases and the object storage they are published to. Required. | [Native runtime](../execution/native-runtime.md#backend-deployment-configuration) |
| `DEMI_CHANGE_STORE_CONFIG` | Puts the object store in an S3 bucket. Optional: the data directory holds it otherwise. | [Storage](storage.md#the-object-store) |
| `DEMI_INSTANCE_SECRET` | The instance secret as 64 hexadecimal digits. Optional: generated into the data directory otherwise. | [Storage](storage.md#passwords-and-credentials-at-rest) |
| `DEMI_EXPOSE_DOMAIN` | The domain of expose hostnames. Optional: without it, exposes are unavailable. | [Host expose](../execution/expose.md#deployment) |
| `DEMI_WEB_DIRECTORY` | A built browser directory to serve beside the API. Optional. | [Web API](../product/web-api.md#serving-the-browser-build) |
| `DEMI_RUNNER_RELEASE_DIR` | The runner releases the installer routes serve. Optional: without it, those routes answer 503. | [Builds and releases](../delivery/builds-and-releases.md) |

## Deployment and user ownership

The single-backend deployment runs one backend process that owns every user.
Its control service runs in process, its conversation databases are local, its
object store is the data directory or an S3 bucket, and the machine manager
supplies [Cloud](../cloud/managed-hosts.md), which every deployment has. It
can serve the built browser directory alongside the API, and with
`DEMI_EXPOSE_DOMAIN` configured it answers expose hostnames with the public
relay. [Cloud setup](../cloud/setup.md) describes installing the machine
manager.

A user is the unit of placement at both levels. Inside a process, all of a
user's work runs on one shard thread ([Runtime model](#runtime-model)). The
multi-worker deployment pins each user to one worker process, a complete
backend for its assigned users, and adds one internal control service:

```text
Browser / runner -> reverse proxy -> worker for that user
                                      +-- the user's shard: conversations and live
                                      |   sessions, runner connections, the Cloud machine
                                      +-- control service client -> control service
```

A deployment route map pins each user's HTTP, conversation sockets, runner
sockets, and managed machines to one worker. The selected routing design uses
a browser route cookie and a runner reconnect header containing the owner's
routing key. Routing hints select placement; authentication still establishes
identity. Login requests can reach any worker because account lookup uses
shared control records. Pairing must reach the worker that holds the unclaimed
runner's connection, so its routing must preserve that connection-to-code
relationship. An off-the-shelf reverse proxy applies the map; the product
supplies deployment configuration rather than a custom router.

At most one worker serves a user at a time, and a route map alone does not
guarantee it. A worker that has lost a user's route can still be running, with
the user's conversation databases open, the user's Cloud disks in use, and
runner connections that still run jobs. For example, if worker A loses user
U's route while one of U's turns is saving, and worker B restores U's
conversation databases from their replicas, A goes on writing its copy while B
writes its own, and the two copies of U's conversations diverge. Therefore,
before another worker takes a user, the worker that owned the user is fenced:
its storage and execution authority end first, and only then does the
destination gain write access to the user's databases and disks.

Existing WebSockets remain on their worker. Moving a user to another worker
requires draining or stopping the user's sessions, publishing the Cloud
machine's state, fencing the worker that owned the user, restoring the user's
replicated conversation state on the destination, and updating the route map.
The fence comes before the restore. Runners then reconnect to the destination.
The experience is a backend restart; running sessions do not move
transparently.

Storage publication and control-service requirements are defined in
[Storage](storage.md#multi-worker-storage-placement). The decisions still open
for this deployment, among them how a worker is fenced, how a claim reaches
the worker that holds its unclaimed runner, and how an expose hostname reaches
its owner's worker, are listed in the
[Roadmap](../delivery/roadmap.md#decisions-before-expanding-deployment).
