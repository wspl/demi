# Demi Next: Backend Architecture

`@demicodes/backend` is the product server. It authenticates browser requests,
hosts conversation agent trees, assembles providers and commands, and connects
those sessions to execution machines. Runners execute device work; the backend
owns conversation state and product policy.

## Request paths and responsibilities

```text
Browser
  +-- HTTP product data ----> HTTP routes -> domain modules -> ControlService
  +-- conversation WS -----> scoped transport -> AgentServer -> conversation DB
                                                   |
                             +---------------------+-------------------+
                             v                                         v
                     provider runtime                         Host / shell context
                       |         |                                     |
                 vendor API      +-- process spawn --------------------+
                                                                       v
                                                              runner connection
                                                                       |
                                                              execution machine
```

For example, opening history reads stored blocks without starting a runner.
Sending a message validates its frame and provider selection. Inference can use
a backend HTTP provider; a shell tool or process provider obtains the selected
execution Host when needed. The same conversation keeps its history if its target
changes.

| Module | Responsibility | Design contract |
|---|---|---|
| `http` | Public routes, validation, authentication gates, WebSocket upgrade, browser assets | [Web API](web-api.md) |
| `auth` | Accounts, password verification, sessions, login lockout, email-change delivery | [Product](product.md) |
| `conversation` | Agent-tree hosting, history/Fork, summaries, target resolution, the one host entry for operations outside the agent, frame and file admission | [Sessions and targets](sessions-and-targets.md) |
| Command assembly | Coding-agent roots plus the product's `host` group, manifests, incoming RPC dispatch | [Commands](commands.md) |
| `llm`, `vault`, `usage` | Provider assembly, credential scope, model discovery, inference admission and accounting | [Providers](providers-and-vault.md) |
| `runner` | Device pairing, live connection registry, remote Host handles, command relay, pipe broker | [Runner](runner.md) |
| `lifecycle` (planned) | Shared use admission, startup joining, idle scheduling, dependent-resource retirement and disposal | [Resource lifecycle coordination](resource-lifecycle.md) |
| `managed` | Cloud policy, allocation, machine transitions, image generations and reset through the shared coordinator | [Managed hosts](managed-hosts.md) |
| `storage` | Control records, per-conversation persistence, user blob namespaces | [Storage](storage.md) |
| `sync` | Reconstructible application snapshots for browser polling | [Web API state synchronization](web-api.md#sidebar-mutations-read-state-and-page-synchronization) |

These are modules of one backend, not independently deployed services.
`backend` is a product leaf: framework packages do not import it.
[Package boundaries](../package-boundaries.md) owns the dependency contract.

## Authentication and ownership

Browser API routes use the `demi_session` cookie. It contains a random 256-bit
token whose SHA-256 identifies the stored session. The cookie is `HttpOnly`,
`SameSite=Lax`, and `Path=/`; HTTPS requests, including forwarded HTTPS, set
`Secure`. A missing or expired session returns 401 `unauthenticated`; an invalid
supplied cookie is cleared.

Sessions expire 30 days after their last renewal. A request with less than
15 days remaining renews the session and cookie. Login failures are tracked by
email; five recent failures lock it for one minute. These are configurable auth
defaults, separate from inference rate limiting.

Setup and login are public entrances. Runner and pipe routes use device
credentials instead of browser cookies; an unclaimed runner uses the pairing
protocol. Public installer downloads contain no credential. All other `/api`
resources pass through the browser session gate. Inaccessible user-owned objects
return 404, insufficient role returns 403, and missing authentication returns 401.

[Instance mode](product.md#instance-mode-shared-vs-isolated) controls provider
ownership. Every provider lookup, catalog, configuration route, and inference
resolution uses the same scope. Starting with provider rows whose ownership
conflicts with the configured mode is refused.

## Browser synchronization

Ordinary product state uses REST with stable error codes and `{ code, message }`
errors. Each open conversation uses one WebSocket at
`/api/conversations/:id/stream`, carrying agent `ClientFrame` and `ServerFrame`
messages. Execution context comes from the conversation's server-side target;
the browser cannot override it with an arbitrary frame cwd.

`GET /api/state` returns a conditional application snapshot: account, preferences,
projects, conversation summaries, devices, providers, and Cloud status. It does
not start Cloud or infer. The product browser polls and revalidates this snapshot;
chat output remains on the conversation stream. A snapshot combines multiple
stores and live state, so it is reconstructible rather than one global atomic
read. A later poll catches changes made during assembly.

`web-ui` implements reusable behavior through data and handlers. `web` supplies
fetch, WebSocket, and state adapters; `web-gallery` supplies specimens using the
same components. Model discovery has its own account-wide cache and loading
state, so it does not block readable history. See
[Model catalog caching](../model-catalog-cache.md).

## Media by reference

Transcript media sent to the browser uses blob references instead of inline bulk
bytes. The conversation transport externalizes inline media in root and subagent
reset/patch frames, preserving frame order. The browser retrieves the referenced
bytes through the cookie-authenticated blob route in its user's namespace.

Only the route's supported image, video, audio, and PDF media types are served
inline. Other content downloads as `application/octet-stream`; responses include
`X-Content-Type-Options: nosniff`, private immutable caching for one year, and
`Vary: Cookie`. A hash is not an authorization token across users.

Inbound send/steer validation accepts the agent content schema plus product upload
and remote-file references. The scoped transport validates the entire frame
before changing metadata or granting attachments. It resolves uploaded attachment
IDs to caller-owned blobs and writes them under the selected Host's
`~/.demi/attachments/<conversation>/`, outside the workspace. The agent receives
an attachment record and any native media block it can consume. Text attachments
can include a short preview. Missing or inaccessible uploads become an explicit
attachment-unavailable text block.

A remote-file reference retains its device and absolute path instead of copying
bytes. The backend checks ownership and connectivity before adding attached-host
grants; the model reads the file later through the host command. Its contents can
change before that read. [Web API](web-api.md#device-files-and-remote-references)
defines the wire shape.

Inbound resolution and outbound externalization each serialize their work. A
failed frame reports an error without poisoning later queued frames. Closing or
unsubscribing prevents delayed work from reaching the session. The underlying
persistence and blob ownership are defined in [Storage](storage.md).

## Deployment and user ownership

The local deployment runs one backend with in-process `ControlService`, local
conversation databases, blob storage, and optional managed machines. It can serve
the built browser directory alongside the API. Cloud setup is described in
[Managed-host setup](../managed-hosts-setup.md).

The intended multi-worker deployment retains a complete backend per assigned
user and introduces one internal control service:

```text
Browser / runner -> reverse proxy -> worker for that user
                                      +-- conversations and live sessions
                                      +-- runner connections and managed VMs
                                      +-- remote ControlService -> demi-controld
```

A deployment route map pins each user's HTTP, conversation sockets, runner
sockets, and managed machines to one worker. The selected routing design uses a
browser route cookie and a runner reconnect header containing the owner's routing
key. Routing hints select placement; authentication still establishes identity.
Login requests can reach any worker because account lookup uses shared control
records. Pairing also needs to reach the worker holding the unclaimed runner
connection; its routing must preserve that connection-to-code relationship. An off-the-shelf reverse proxy applies the map; the product
supplies deployment configuration rather than a custom router.

Existing WebSockets remain on their worker. Moving a user requires draining or
stopping their sessions, publishing managed-machine state, restoring replicated
conversation state on the destination, fencing the old owner, and updating the
route map. Runners reconnect to the destination. The experience is a backend
restart; it is not transparent live migration. Storage publication and control
RPC requirements are defined in [Storage](storage.md#multi-worker-storage-placement).

## Implementation status

The executable implements the single-backend deployment. The browser is connected
to REST, conditional state polling, preferences, and conversation WebSockets.
The old frontend milestone labels are not API boundaries; the current route
contract is [Web API](web-api.md).

Multi-worker routing, ownership fencing, the remote control service, and S3
recovery are not implemented. Shared-mode subscription credentials also need an
explicit multi-worker distribution and refresh policy before that deployment can
be implemented. Routing an authenticated claim to an unclaimed runner on another
worker is also an open deployment decision. User affinity alone does not solve instance-shared credentials.
