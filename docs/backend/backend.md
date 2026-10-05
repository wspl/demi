# Backend architecture

The backend is the product server, one Rust executable named `demi-backend`
([Builds and releases](../delivery/builds-and-releases.md) lists its targets).
It authenticates the web app's requests, hosts conversation agent trees,
assembles providers and commands, and connects those sessions to devices
through their runners. Runners execute device work; the backend owns
conversation state and product policy.

## Request paths and responsibilities

Two parts of the backend appear in every path. The edge accepts connections,
authenticates requests and copies bytes. A user's shard holds everything the
backend decides for one user: that user's conversations, agent trees, devices
and Cloud. [Runtime model](#runtime-model) explains both.

```text
Web app
  +-- HTTP product data ----> edge ---> a shared service, or the user's shard
  +-- sync channel ---------> edge ---> user's shard: each change the page shows
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

Besides the web app, three kinds of client reach the backend: runners, over a
WebSocket and HTTP pipes authenticated by their device token; anonymous
visitors of an expose hostname, whom the public relay serves; and anyone who
downloads the runner installers. The backend itself calls the machine manager
over the manager's Unix socket.

| Module | Crate | Responsibility | Design contract |
|---|---|---|---|
| `edge` | `backend-http` | The listener and router, the session gate, request extractors and body limits, error codes, installer, native artifact and web app asset routes, runner acceptance, and the byte copies of file transfers, pipes, user streams and the expose relay | [Web API](../product/web-api.md) |
| `shard` | `backend-user-shard` | Shard threads, each user's shard, calls into it, the shared services every shard is given, socket adoption and the page socket both of a page's sockets are served through | [Runtime model](#runtime-model) |
| `config` | `demi-backend` | The typed configuration, validated at startup, and the instance secret with the keys derived from it | [Configuration](#configuration) |
| `auth`, `settings` | `backend-accounts` | Accounts, password hashing, web sessions, login lockout, email-change delivery; per-user preferences and subagent settings | [Authentication and ownership](#authentication-and-ownership), [Product](../product/product.md#user-system), [Web API](../product/web-api.md#user-preferences) |
| `sync` | `backend-page-sync`, `backend-user-shard` | The registry that marks changes on each user's channels (`backend-page-sync`); the pages' synchronization channels with the product state and the parts that changed (`backend-user-shard`) | [Page synchronization](#page-synchronization) |
| `conversation` | `backend-user-shard` | Agent-tree hosting with the agent server's dependencies, the product's instructions and the execution context source, frame scoping, attachment references, history and Fork, summaries and titles, the Claude Code CLI's work on the user's Cloud, and the provider test | [Sessions and targets](../execution/sessions-and-targets.md) |
| `host_access` | `backend-host-access` | The conversation's host access, target resolution and transitions, file transfers, uploads, remote files and user streams with the leases the edge holds of them, the nodes' shell environments with the keeper that stores what a command leaves when it ends, the installation of the plugins' Host directories before a job, the product's `demi host` group | [Host operations](../execution/sessions-and-targets.md#host-operations) |
| `plugins` | `backend-plugins`, `demi-backend` (`plugins`) | The plugin host: the registry and its checks, the command set, instructions and context sources the agent server is given, each user's instances, the port's operations, page state and page calls (`backend-plugins`); the built-in plugins, in their order of registration (the backend's `plugins`) | [Plugins](../architecture/plugins.md) |
| `runner` | `backend-runners` | Pairing, device links and runner connections with the Host handles made over them, the lease of a conversation's file gate a conversation's Host is made against, the rpc relay and each session's commands, installer scripts, native artifact publication into the object store and the local store's artifact route | [Runner](../execution/runner.md), [Commands](../execution/commands.md), [Native runtime](../execution/native-runtime.md#backend-deployment-configuration) |
| `lifecycle` | `backend-idle-watch`, `backend-user-shard` | The idle watch (`backend-idle-watch`); the conversation idle clock, the conversation release, and the daily retention pass that retires expired tool media, removes expired command outputs and collects blobs (`backend-user-shard`) | [Conversation idle and Host resource release](../execution/resource-lifecycle.md), [Retention](storage.md#retention) |
| `managed` | `backend-cloud` | Cloud policy and capacity, machine transitions, reset and recovery, the machine manager's client | [Managed hosts](../cloud/managed-hosts.md) |
| `expose` | `backend-expose` | Expose records and their lifetime, live relay connections | [Host expose](../execution/expose.md) |
| `llm`, `vault`, `usage` | `backend-providers`, `backend` (`families`) | Provider assembly and model catalogs; credential records, scope and login flows; metering and the request rate limit; the built-in provider families (the backend's `families`) | [Providers](../providers/providers.md), [Models](../providers/models.md), [Usage and quota](../providers/usage-and-quota.md) |
| `storage` | `backend-database`, `backend-blobs` | The control service, conversation databases and the tree store with its `blob_refs` index and its records of commands' outputs (`backend-database`); the object store with its record of blob uses (`backend-blobs`) | [Storage](storage.md) |

These are modules of one backend executable, not independently deployed
services; each lives in the crate the table names, so that a change to one
recompiles only it and what builds on it.
[Crates and packages](../architecture/crates-and-packages.md#backend-libraries)
gives each crate's boundary and their layering.

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
| Shared services | State that spans users, or that is needed before the user is known | The control service, the conversation stores, the object store, the vault, provider assembly with model catalogs and one credential refresh at a time per account, the machine manager's client, [Cloud capacity](../cloud/managed-hosts.md#lifecycle-and-capacity) across users, runners waiting to be paired, login lockout, the registry of each user's open synchronization channels |
| A user's shard | Everything the backend decides for that user | Each conversation's file gate, open transfers and user streams, and idle watch; agent trees; device links and one task per runner connection; pipe records; the Cloud machine; live expose connections; title requests; Fork requests, one at a time per destination; each session's command router for the rpc relay; the request rate limit; the pages' synchronization channels |
| Database threads | One per open SQLite connection | The control database; up to 64 conversation writer connections ([Storage](storage.md#conversation-state-and-transactions)) |
| Blocking pool | Work that would stall an async thread | Disk IO; password and blob hashing; serializing request bodies with media and large transcript frames; read-only conversation reads |

[The user shard](../architecture/concurrency.md#the-user-shard) gives the
rules this placement follows: why all of a user's work shares one thread, what
crosses between the edge and a shard, and how a call ends when its requester
goes away. A call that panics answers 500 `internal_error`, and the shard goes
on serving.

A stable hash of the user id pins each user to one shard thread, and the
backend starts with one shard thread. A user's shard is created by the first
request for that user and loads nothing ahead of need: the Cloud's device
record and its latest reset load when the Cloud is first needed, and
conversations, agent trees and devices when they are first used, and
an agent tree that no socket watches closes again once it is idle
([Connections and the live tree](../agent/runtime.md#connections-and-the-live-tree)).
The machine manager's death events reach one edge task, which calls the shard
of the device's owner.

The edge uses axum because its handlers are thin: parse, authenticate, check
ownership, then call a shared service or a shard. axum's requirement that a
handler's future be `Send` therefore costs nothing, and its extractors, tower
middleware and socket-free router tests come with it. The edge serves the
connections of the backend's own listener itself, with hyper's HTTP/1
server: it keeps each header name's case, which the expose relay passes on
and axum's `serve` cannot, and it answers a request for an expose hostname
with the relay before the router sees it. The listener gives every
connection an idle deadline and a close handle and exposes the peer address.
A download arms the 60-second deadline, a lease the shard ends closes the
connection at once even when the user's browser has stopped reading, and the
expose relay forwards the peer address.

The listener turns off Nagle's algorithm (`TCP_NODELAY`) on every connection
it accepts. The runner and conversation sockets carry small messages whose
latency matters. With the algorithm on, a small message written while an
earlier one is still unacknowledged waits for that acknowledgement, which the
peer may delay by tens of milliseconds. For example, the device token a
claimed runner receives follows its pairing code, and would wait for the
runner to acknowledge the code.

## Authentication and ownership

The web app's API routes use the `demi_session` cookie. It contains a random
256-bit token whose SHA-256 identifies the stored session. The cookie is
`HttpOnly`, `SameSite=Lax`, and `Path=/`; HTTPS requests, including forwarded
HTTPS, set `Secure`. A missing or expired session returns 401 `unauthenticated`;
an invalid supplied cookie is cleared.

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
credentials instead of the session cookie. Public installer downloads and the
expose relay carry no credential. The synchronization channel checks the
session cookie itself, since the gate would renew the session and the channel
never does ([Page synchronization](#page-synchronization)). All other
`/api` resources, unknown paths included, pass through the session
gate, so an unauthenticated request for a path that does not exist answers
401, not 404. [Authentication](../product/web-api.md#authentication) lists
the routes of each kind. Inaccessible user-owned objects return 404,
insufficient role returns 403, and missing authentication returns 401.

A request that could act with the user's session must come from a page of
the product. The user's browser sends the `SameSite=Lax` cookie with a request
from any page of the product's site, not only from the product's own pages. An
expose's page is such a page: `<id>.expose.demi.example` is on the site of
`demi.example` ([Host expose](../execution/expose.md#deployment)), and it may
be another user's. The user's browser keeps such a page from reading the
backend's answers, since the backend lets no other origin read them (it sends no
CORS headers), but not from sending requests. For example, without a check, a
script on an expose could pair its author's runner to a signed-in visitor's
account with a `POST /api/devices/claim` whose JSON body it labels
`text/plain`, which the visitor's browser sends without asking the backend
first; or it could open the visitor's conversation socket, read the transcript
and send messages.

So the edge checks the `Origin` of each request to a web app route that could
act: a request whose method is unsafe (any but GET, HEAD, OPTIONS and TRACE),
and every upgrade, such as a WebSocket's. The web app's routes are setup,
login and every route the session cookie authenticates; a runner's routes,
public downloads and the expose relay have no such check, since no cookie
authenticates them. The origin must be the public URL's
(`DEMI_BACKEND_PUBLIC_URL`), or have the host and port the request was sent to
(the `Host` header), as when a development server passes the page's requests
on. Any other origin, `null` included, answers 403 `forbidden_origin` before
anything else, the session gate included. The check stands in front of the
web app's routes rather than in each of them, so a new web app route has it
without asking.

- **A request without `Origin` passes.** The user's browser sends `Origin` with
  every request the check covers (the Fetch standard requires it), so only a
  program that is not a web browser leaves it out, such as `curl` calling the
  setup API
  ([Development and checks](../product/web-application.md#development-and-checks)).
  Such a program holds any cookie it sends and could send any origin it
  liked, so refusing it would protect nothing.
- **WebSockets follow the same rule.** An upgrade is a GET, but the page that
  opens a socket reads it and sends on it, so it acts as an unsafe request
  does.
- **Setup and login follow it too,** although they need no cookie: they set
  one. A login from another page would sign the visitor's browser in to the
  page author's account, where what the visitor did next would land: a device
  paired from Settings would give that account a runner on the visitor's
  computer. A setup from another page could take the master account of an
  instance that the page's author cannot reach but the visitor's browser can,
  such as one on `localhost`.
- **Other GET, HEAD, OPTIONS and TRACE requests are not checked.** HTTP
  defines these methods as safe, the product's routes use them only to read,
  and another page cannot read their answers.

A reverse proxy in front of the backend passes each request's `Origin` and
`Host` headers to it unchanged, since the check reads both. A proxy that
drops `Origin` turns the check off: every request then passes, as a request
from `curl` does. The backend notices a request from the user's browser that
lost its `Origin` on the way. Every current web browser sends Fetch Metadata
(`Sec-Fetch-Site`) with each request to an HTTPS site or to `localhost`, and
`Origin` with each request the check covers, so such a request that has
`Sec-Fetch-Site` and no `Origin` passed a proxy that dropped `Origin`. The
request still passes, as any request without `Origin` does, and the backend
logs one warning, the first time it sees such a request: the proxy drops
`Origin`, so the check against other sites' pages is off. For example, a
user signs in through a proxy that strips `Origin`: the sign-in works, and
the backend's log shows the warning once, however many requests follow.

Behind a proxy that rewrites `Host`, the backend refuses the product's own
pages, unless `DEMI_BACKEND_PUBLIC_URL` is their origin. An operator checks a
deployment from outside with a request that names another origin, which must
answer 403 `forbidden_origin`:

```sh
curl -s -X POST https://demi.example/api/auth/login \
  -H 'Origin: https://elsewhere.example' \
  -H 'Content-Type: application/json' --data '{}'
```

Any other answer means that the proxy did not pass `Origin`: the backend then
took the login as one from no page, and refused only its empty body, with 400
`invalid_body`. The same request with the product's own origin, such as
`https://demi.example`, must answer 400 `invalid_body`, not 403.

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

## Page synchronization

Ordinary product state uses REST with stable error codes and
`{ code, message }` errors. Every request and response type, and every error
code, is defined once in the contract crates and generated for the web app
([Generated TypeScript](../architecture/contracts.md#generated-typescript));
[Web API](../product/web-api.md) lists the routes.

Each page that has a conversation open uses one WebSocket at
`/api/conversations/:id/stream`, carrying the agent protocol's client and
server frames ([Frame protocol](../agent/runtime.md#frame-protocol)); the
sockets of several pages attach to the conversation's one live tree
([Connections and the live tree](../agent/runtime.md#connections-and-the-live-tree)). After
the upgrade the socket moves into the user's shard, which serves it until it
closes. Execution context comes from the conversation's server-side target;
the page cannot override it with an arbitrary frame cwd.

Each page also holds one synchronization channel, `WS /api/sync`. It sends the
page the product state when it connects, then every part of it that changes:
the account, preferences, providers, workspaces, devices, the Cloud, each
plugin's state, and each conversation's summary
([Page synchronization](../product/web-api.md#page-synchronization)). No page
polls. The socket moves into the user's shard, as a conversation socket does.
For example, a rename commits in the user's shard; the shard then marks the
conversation's summary as changed on each of the user's open channels, and the
task of each channel reads the summary and sends it to its page.

- **Marks.** Every change a page shows is marked where it commits, after the
  commit. The shard marks what it changes: conversations, titles, devices
  and the Cloud, and a plugin's state when the plugin marks it or a change it
  follows happens, such as one of the user's exposes being created or
  destroyed. A conversation's tree store marks it when a
  checkpoint is saved, and the agent's notice marks it when its tree starts or
  stops working or is disposed. A shared service marks what it changes for a
  user, such as the vault when it renews an account's credential.
- **The registry.** Marks go through a registry of each user's open channels,
  a shared service, because some changes come from outside the user's shard:
  a shared instance's provider entries serve every user, so a change of one
  marks the channels of every user. A mark adds the part to each channel's set
  of changed parts and wakes the channel's task; nothing else crosses into the
  shard.
- **No queue.** A channel holds at most one mark per part, however far its
  page falls behind; its task reads each marked part when it can send it, so
  a slow page costs a set, never a growing queue, and is never closed for it.
- **Sessions.** A channel belongs to the session it opened with. Signing out
  closes that session's channels, and a channel closes itself when its
  session expires. It never renews the session: only requests do.

The channel and the conversation sockets stay apart. A conversation's frames
are a log the page applies in order, so the socket's outbox keeps every frame
and closes a page that falls 4,096 frames behind
([Order and delivery](../agent/runtime.md#order-and-delivery)); the channel
sends current values, which it can merge, so it never closes a slow page. On
one shared socket, a conversation that lagged would close the page's channel
and every other conversation with it, and a transcript's handshake or a burst
of command output would hold back the sidebar's changes behind it.

Both kinds of socket send a heartbeat once they have sent nothing else for 30
seconds, so that a page can tell a quiet socket from one that died without a
close ([Liveness and reconnection](../product/web-application.md#liveness-and-reconnection)).
The shard serves both through one page socket, which owns that interval and
the bound on the close
([Startup and shutdown](#startup-and-shutdown)).

How the web app consumes both, with its adapters, its synchronization and
its session handling, is defined in
[Web application](../product/web-application.md).
Model discovery has its own account-wide cache and loading state, so it does
not block readable history ([Models](../providers/models.md#catalog-cache)).

## Media by reference

Transcript blocks hold their media by blob reference
([Media](../agent/runtime.md#media)), so the frames that carry them carry
references, and the conversation socket sends each frame as the agent wrote
it: it neither stores nor reads media. A blob is stored before the first frame
that names it, so the page can fetch every reference a frame carries, when
the frame arrives, through the cookie-authenticated blob route in its user's
namespace.

Only the media types [file previews](../product/file-previews.md#keeping-file-content-inert)
show in place are served inline, under the same content policy; the page and
the backend read one file-type table
([Logic the web app and backend share](../architecture/contracts.md#logic-the-web-app-and-backend-share)).
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
for a text file, and the native media block of an image, a video or a PDF,
which references the upload's own blob. An image enters fitted to what every
provider accepts
([Images in the transcript](../agent/runtime.md#images-in-the-transcript)):
when fitting changes it, its block references the fitted image's blob, which
the socket stores first. The socket hands the session the bytes of the
block's medium, those it read to write the file or the fitted ones, so the
session holds them without reading the blob again. A
missing or inaccessible upload becomes an explicit attachment-unavailable text
block.
An edit's references to the files the edited message holds pass to the
agent as they came, and the session resolves them from that message
([Files the edit keeps](../agent/message-editing.md#files-the-edit-keeps)).

A remote-file reference retains its device and absolute path instead of
copying bytes. The backend checks ownership and connectivity before adding
attached-host grants; the model reads the file later through the host command.
Its contents can change before that read.
[Web API](../product/web-api.md#device-files-and-remote-references) defines
the wire shape.

Resolving uploads keeps the socket's frame order
([Order and delivery](../agent/runtime.md#order-and-delivery)). A frame whose
resolution fails reports an error without affecting the frames behind it, and
closing the socket stops resolution that has not reached the session yet. The
underlying persistence and blob ownership are defined in [Storage](storage.md).

## Failure facts

An error block keeps the vendor's failure record as it arrived; what the
page shows from it is read when the block is sent
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
2. Opens the data directory and the object store
   ([The object store](storage.md#the-object-store)).
3. Loads the command package releases of its server release and publishes
   their artifacts into the object store
   ([Native runtime](../execution/native-runtime.md#publish-artifacts-before-enabling-commands)).
   An interrupt or termination signal during publication stops the start.
4. Loads the instance secret and opens the control database; a new database
   receives its schema.
5. Starts the shared services, among them the plugin host, which checks
   every plugin's manifest against the others and the native catalog; a
   manifest that breaks a rule stops the start and the error names the plugin
   ([The plugin host](../architecture/plugins.md#the-plugin-host)). Then it
   starts the shard threads.
6. Recovers before it serves: the machine manager reconciles its machines,
   which stops every Cloud, so the exposes an earlier backend left on a Cloud
   are destroyed ([Host expose](../execution/expose.md#lifetime)), an
   interrupted Cloud reset finishes committing its disks and is marked failed
   so that a retry starts the Cloud
   ([Managed hosts](../cloud/managed-hosts.md#system-reset)), and Fork
   creations whose destination root was committed are published
   ([Conversation Fork](../agent/conversation-fork.md#backend-creation-and-retries)).
   Each saved yield wakeup is armed again: its conversation's shard restores
   the tree when the wakeup is due
   ([Yield wakeups](../agent/runtime.md#yield-wakeups)).
7. Opens its listener, and starts the daily retention pass, whose first pass
   runs at once ([The retention pass](storage.md#the-retention-pass)).

The backend watches for SIGINT and SIGTERM from its first step. A signal that
comes before step 4 stops the start; one that comes during steps 4 to 7 is
kept: the start finishes, and shutdown follows at once.

Shutdown closes the listener first, so that no new work starts and no runner
reconnects into a backend that is closing. A new request on a connection that
is already open answers 503 `backend_closing`. Runner connections and pipes
keep working, because the steps below need them:

1. Login flows are cancelled, and no retention pass starts; one under way
   ends with its shard.
2. Each shard ends its user's work in this order. The synchronization
   channels close, so no page is sent what the steps below change; a page
   reads the state again from the next backend. Idle watches stop, and a
   retirement already running finishes. Title requests are aborted and expose
   connections end. Open file transfers and user streams end. Conversation sockets
   close and the waits for saved wakeups end, so no tree opens and no frame
   reaches a tree after its shutdown. Agent turns are
   aborted: a running turn records that its session was shut down, and its
   jobs are killed while their runners are still connected. Claude Code CLI
   installs are cancelled; the next need starts them again. A Cloud boot
   waiting for its runner ends at once, since the closed listener lets no
   runner connect, and saves what the sandbox wrote, as a failed boot does.
   The Cloud hibernates. Then the shard's runner connections close, and after
   them its pipes fail.
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

A page cannot hold up shutdown. When a socket to a page closes, the
synchronization channel or a conversation socket, the backend stops sending
whatever it was sending on it and sends the close frame, and it waits at most
one second for the page to take that frame. For example, a phone's page
stopped reading in the middle of a long transcript, and the socket's buffers
are full: without the bound, the backend would wait for the phone to read
before the shard could close. After the second, the page loses the connection
without the close frame, and connects again as it does after any close
([Liveness and reconnection](../product/web-application.md#liveness-and-reconnection)).

## Configuration

The backend and the machine manager of a deployment read one configuration,
the environment file `/etc/demi/demi.env` that both services load, or the
same settings as command-line flags. Each parses its settings with clap into
one typed configuration, validates all of it before anything starts, and
names the variable in an error. The prefix `DEMI_MANAGED_` belongs to the
machine manager, which refuses a `DEMI_MANAGED_*` variable it does not read
([Cloud setup](../cloud/setup.md#configuration)); the backend leaves those
names to it and refuses every other `DEMI_*` variable that none of its
settings reads, such as a misspelt `DEMI_BACKEND_LISTENN`, rather than leaving
the setting at its default. The settings the manager shares with the backend
are the backend's names, so the backend's check covers them, and one file
serves both programs without either ignoring a misspelling. `demi-backend
--help` lists the flags.

| Variable | Meaning | Defined in |
|---|---|---|
| `DEMI_RELEASE` | The [server release](../delivery/builds-and-releases.md#server-release) root. The backend serves its `web/` when it has one, installs runners from its `runners/` (without it, the installer routes answer 503), and publishes its `commands/`. Default: the directory above the one that holds the running executable, so `/opt/demi/0.1.3/bin/demi-backend` uses `/opt/demi/0.1.3`. The machine manager reads it too. | [Builds and releases](../delivery/builds-and-releases.md#server-release) |
| `DEMI_BACKEND_DATA` | The data directory. Default `~/.demi/backend`. | [Storage](storage.md#ownership-and-layout) |
| `DEMI_BACKEND_LISTEN` | The address and port the backend listens on, as `<address>:<port>`. Default `0.0.0.0:3271`. | [Public URL and listening address](#public-url-and-listening-address) |
| `DEMI_BACKEND_PUBLIC_URL` | The URL at which browsers, runners and Cloud guests reach the backend: installers embed it, the page's install command fetches them from it, expose URLs take their scheme and port from it, and a local store's downloads are on it. Required. The machine manager reads it too, as the one endpoint its Clouds may reach on the host or a private address. | [Public URL and listening address](#public-url-and-listening-address) |
| `DEMI_MACHINE_MANAGER_SOCKET` | The machine manager's Unix socket, which the manager listens on and the backend connects to. Default `/run/demi-cloud/machines.sock`. | [Cloud setup](../cloud/setup.md#configuration) |
| `DEMI_INSTANCE_MODE` | `shared` or `isolated`. Required: whoever deploys decides it, and nothing chooses for them. | [Product](../product/product.md#instance-mode-shared-vs-isolated) |
| `DEMI_STORAGE` | `local` or `s3`: where the one object store lives. Default `local`. With `s3`, the `DEMI_S3_*` settings name the bucket. | [Storage](storage.md#the-object-store) |
| `DEMI_INSTANCE_SECRET` | The instance secret as 64 hexadecimal digits. Optional: generated into the data directory otherwise. | [Storage](storage.md#passwords-and-credentials-at-rest) |
| `DEMI_EXPOSE_DOMAIN` | The domain of expose hostnames. Optional: without it, exposes are unavailable. | [Host expose](../execution/expose.md#deployment) |
| `DEMI_CLAUDE_RELEASES_URL` | The Claude Code distribution whose newest release the CLI on each Cloud follows. Default `https://downloads.claude.ai/claude-code-releases`, the vendor's. | [Claude Code](../providers/claude-code.md#which-version) |
| `DEMI_LOG` | What the backend writes to its standard error, in `tracing-subscriber`'s `Targets` syntax: comma-separated, a default level and `target=level` pairs, each pair covering its target and the targets below it. For example, `info,demi::provider::claude_code::wire=trace` adds the Claude Code CLI's raw exchange to the default. Default `info`. | [Claude Code](../providers/claude-code.md#process-lifetime) |

For example, a server whose backend sits behind Caddy on the same machine
needs only the two settings without a default and the listening address:

```dotenv
DEMI_BACKEND_PUBLIC_URL=https://demi.example.com
DEMI_INSTANCE_MODE=isolated
DEMI_BACKEND_LISTEN=127.0.0.1:3271
```

### Public URL and listening address

Demi does not terminate TLS. A deployment's public URL is an HTTPS URL on a
domain name, and something in front of the backend holds the certificate:

| Shape | `DEMI_BACKEND_LISTEN` | In front of the backend |
| --- | --- | --- |
| Behind a CDN | `0.0.0.0:<port>` | Cloudflare proxies the domain's DNS record and terminates TLS, then reaches the backend over HTTP on the server's public address |
| Behind a local reverse proxy | `127.0.0.1:<port>` | Caddy on the same machine terminates TLS on port 443 and forwards to the backend on loopback |

Cloudflare forwards HTTP only to ports 80, 8080, 8880, 2052, 2082, 2086 and
2095 ([Cloudflare network ports](https://developers.cloudflare.com/fundamentals/reference/network-ports/)),
so a backend behind it listens on one of them. Either proxy must pass each
request's `Origin` and `Host` unchanged
([Authentication and ownership](#authentication-and-ownership)).

The URL names a domain, not an IP address, because the web app needs a
secure context: browsers give a page the APIs it uses, such as
`crypto.randomUUID()` and the clipboard, only over HTTPS or on `localhost`,
and a certificate for a public IP address is not what either proxy issues.
Development is the exception: there the page is on `localhost`, and the public
URL may be `http://127.0.0.1:<port>` or a LAN address that the runners reach
([Development backend](#development-backend)).

A Cloud reaches the backend through the public URL too. Behind a CDN, its
connection leaves the server as ordinary public traffic to Cloudflare and
comes back through it. Behind a local reverse proxy, the URL resolves to the
server's own public address, which a Cloud may reach only because the machine
manager opens exactly that address and port for it
([Networking](../cloud/managed-hosts.md#networking)).

## Development backend

A developer runs the backend on their own machine with the native programs
they just built, assembled into a server release of the targets in use, which
the backend publishes into the local store of its data directory and serves
its runners itself. For work on the web app alone, one command starts a
backend that needs no machine manager, Cloud image or model account
([One-command development backend](#one-command-development-backend)).
For example, on an x86_64 Linux machine that is also its own Cloud's
execution host, the Hosts in use are the machine as a paired device and the
Cloud guest, and both run `x86_64-unknown-linux-musl`:

1. Build the runner, the command programs, the backend and the machine
   manager for that target, and assemble a server release of it
   ([Server release](../delivery/builds-and-releases.md#server-release)).
   A root is assembled once: to try a changed program, assemble a new one.

   ```sh
   cargo xtask native build --target x86_64-unknown-linux-musl \
     --package demi-runner --package demi-file --package demi-browser \
     --package demi-claude-code --package demi-backend --package demi-machine-manager
   cargo xtask server-release --output /opt/demi/dev-<build> \
     --target x86_64-unknown-linux-musl --server x86_64-unknown-linux-musl
   ```

2. Build the Cloud image into the root's `image/`
   ([guest image build](../../cloud-guest-image/README.md)), and install the
   machine manager from the root
   ([Storage and service setup](../cloud/setup.md#storage-and-service-setup)).
   The configuration file's public URL is the backend's URL at the machine's
   address that the Cloud guest reaches, such as the address of the machine's
   route to the internet; a loopback address does not reach the machine from
   the guest. The Cloud's runner comes from the Cloud image, so a runner
   change reaches the Cloud through a
   [Cloud image refresh](../delivery/builds-and-releases.md#cloud-image-refresh);
   a command program's change reaches it through the object store.
3. Build the backend with the one Cargo selection and start it from the
   repository root with the root and the public URL of the configuration
   file: the Cloud's runner, the installers and the local store's downloads
   use the URL.

   ```sh
   cargo build --workspace --all-targets --features demi-runner/test-fixtures
   DEMI_RELEASE=/opt/demi/dev-<build> \
   DEMI_BACKEND_DATA=~/.demi/development \
   DEMI_INSTANCE_MODE=isolated \
   DEMI_BACKEND_PUBLIC_URL=http://<address the guest reaches>:3271 \
   DEMI_EXPOSE_DOMAIN=expose.localhost \
   target/debug/demi-backend
   ```

4. Run the web application
   ([Development and checks](../product/web-application.md#development-and-checks))
   and create the first account. Pair the machine with the installer, which
   installs the runner from the runner release and prints a pairing code to
   claim in the page:

   ```sh
   curl -fsSL http://<address the guest reaches>:3271/install.sh | sh
   ```

The variables left out keep their defaults ([Configuration](#configuration)):
the backend listens on `0.0.0.0:3271` and connects to the manager's default
socket, the object store is local in the data directory, and the instance
secret is generated there. The root has no `web/`, so the backend serves no
web app and Vite serves the page. `DEMI_EXPOSE_DOMAIN` is optional; without
it, exposes are off.

On a Mac, the machine manager runs in a Lima VM instead; the optional
[Develop on a Mac with Lima](../guides/mac-development.md) guide gives the
differences.

### One-command development backend

`cargo xtask dev` builds the one Cargo selection and runs, until Ctrl-C or a
termination, a backend for the page to talk to. For example, a developer runs
it, then starts the page with the command it prints, signs in with the
account the sign-in page fills in, picks the model the developer's `.env`
names, asks for `uname -a`, and the agent runs it on the Cloud. It starts the backend with the scripted manager
and in the order the
[web app contract suite](../delivery/scenarios.md#web-app-contract-suite)
uses:

- A fresh temporary data directory, removed when the command ends, unless
  `--keep` keeps it.
- The backend scenarios' scripted machine manager, the backend crate's
  example program `scripted_machines`. A conversation's Cloud is a runner it
  starts on this machine, with a temporary home and the artifact cache the
  command names with `--artifacts`.
- `target/debug/demi-backend` in isolated mode on port 3271 (`--port`
  changes it), with the public URL `http://127.0.0.1:<port>`, the manager's
  socket, the local store of the data directory, and a server release root
  beside the data directory that holds only `commands/`: a release of each
  command program the build made, `demi-file`, `demi-browser` and
  `demi-claude-code`, for this machine's target only. The command passes on
  none of its own `DEMI_*` variables.
- With `DEMI_DEV_ECHO=1`, an Anthropic-compatible Messages endpoint inside
  `xtask`, on a free port of the loopback interface, that answers each
  request with `Echo: <the last user message's text>` as a stream; without
  it, none.

Through the web API it seeds the master account and an entry for each
development model `.env` turns on; it selects nothing else. The account is the one
`DEMI_DEV_EMAIL` and `DEMI_DEV_PASSWORD` name in the repository's ignored
`.env`, which the web app's sign-in page fills in during development; with
neither set it is `developer@example.test` with the password `development`,
and setting only one stops the command before it builds anything. Since every
run starts on a fresh data directory, that account is the first user of the
database each time. It prints the backend's URL, the account and the page's
command, which names the backend in `DEMI_BACKEND_URL`, and the account too
when `.env` does not:

```sh
DEMI_BACKEND_URL=http://127.0.0.1:3271 bun run web:dev
```

The backend, the manager and the runners run in process groups of their
own, so the terminal's interrupt reaches only `xtask`, which stops them in
order on every exit, the failure of a start included: the backend first,
since it hibernates the Cloud through the manager as it shuts down, then the
manager, whose input it closes and which ends its runners with it; then it
removes the data directory. A process that does not stop in time, 10 seconds
for the backend and 5 for the manager, is killed. Only a kill of `xtask`
itself leaves the backend running.

A real model lets a turn call tools on the Cloud.
When the command's environment sets all four of
`DEMI_DEV_PROVIDER_BASE_URL`, `DEMI_DEV_PROVIDER_API_KEY`,
`DEMI_DEV_PROVIDER_MODEL` and `DEMI_DEV_PROVIDER_CONTEXT_WINDOW`, it also seeds
an `openai` entry labeled **Development** that speaks Chat Completions to that
endpoint, with that one model and context window, and prints the entry.
`DEMI_DEV_PROVIDER_THINKING_EFFORTS`, optional, lists the model's
thinking efforts, comma-separated, as the endpoint names them; without it the
model has none and the page offers no effort. Setting only some of them stops the command before it builds
anything, naming the missing variables. The key reaches the backend only
through the web API and is never printed. A developer keeps the
four in the repository's ignored `.env` (`.env.example` lists them) and runs
`bun run dev`, which loads `.env` and starts `cargo xtask dev` with it. For
example, with Command Code's gateway:

```sh
DEMI_DEV_PROVIDER_BASE_URL=https://api.commandcode.ai/provider/v1
DEMI_DEV_PROVIDER_API_KEY=<key>
DEMI_DEV_PROVIDER_MODEL=deepseek/deepseek-v4.1-flash
DEMI_DEV_PROVIDER_CONTEXT_WINDOW=1000000
DEMI_DEV_PROVIDER_THINKING_EFFORTS=low,medium,high
```

A turn with that model is a real request and costs what the vendor charges;
automated tests never use it.

`DEMI_DEV_ECHO=1` adds the echo model: an entry of the `anthropic` family
labeled **Echo** that points at the echo endpoint, with the one configured
model `echo`, which answers `hello` with `Echo: hello` without calling any
vendor; any other value stops the command before it builds anything. It is
off by default. With neither a real model nor Echo the run seeds no model,
and its printout says how to add one.

It does not cover what needs the real services: the Cloud isolates nothing
and has no image, so a Cloud reset or a guest's network rules do not behave
as on a real machine, and a paired device of another target finds no program
for it. The steps above give the full development backend for those.

The command releases make the operations of `demi.file`, `demi.browser` and
`demi.claude-code` available on the Cloud and on a paired device of this
machine's target, the conversation browser included. Packaging
`demi-browser` takes the Chrome for Testing archive of this machine's target
from `.cache/resources/`, downloading it the first time
([Packaging](../delivery/builds-and-releases.md#packaging)). The manager
gives its Cloud's runners the artifact cache `.cache/dev-artifacts/` in the
repository, which outlives every run, so a Cloud installs each program and
Chrome once rather than at every wake or run
([Install artifacts](../execution/native-runtime.md#install-artifacts)).

## Deployment and user ownership

The single-backend deployment runs one backend process that owns every user.
Its control service runs in process, its conversation databases are local, its
object store is the data directory or an S3 bucket, and the machine manager
supplies [Cloud](../cloud/managed-hosts.md), which every deployment has. It
can serve the built web app directory alongside the API, and with
`DEMI_EXPOSE_DOMAIN` configured it answers expose hostnames with the public
relay. [Cloud setup](../cloud/setup.md) describes installing the machine
manager.

A user is the unit of placement at both levels. Inside a process, all of a
user's work runs on one shard thread ([Runtime model](#runtime-model)). The
multi-worker deployment pins each user to one worker process, a complete
backend for its assigned users, and adds one internal control service:

```text
Web app / runner -> reverse proxy -> worker for that user
                                      +-- the user's shard: conversations and live
                                      |   sessions, runner connections, the Cloud machine
                                      +-- control service client -> control service
```

A deployment route map pins each user's HTTP, conversation sockets, runner
sockets, and managed machines to one worker. The selected routing design uses
a route cookie in the user's browser and a runner reconnect header containing
the owner's routing key. Routing hints select placement; authentication still
establishes identity. Login requests can reach any worker because account lookup
uses shared control records. Pairing must reach the worker that holds the
unclaimed runner's connection, so its routing must preserve that
connection-to-code relationship. An off-the-shelf reverse proxy applies the map,
passing `Origin` and `Host` unchanged
([Authentication and ownership](#authentication-and-ownership)); the product
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
