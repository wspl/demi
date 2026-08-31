# Runner Daemon

| | |
|---|---|
| Date | 2026-08-31 |
| Status | Proposed (M1 scope of `docs/multi-user-web-gateway.md`) |
| Scope | `@demicodes/runner` — responsibilities, device claiming, gateway connection model, multiplex protocol, control RPC surface |

The runner daemon turns a user's machine into an execution target for the
multi-user web product. It is a headless sibling of the `@demicodes/web`
server: the same assembly (`LocalHost` + coding harness +
`createLocalAgentServer`, command bridge on by default), minus any HTTP
listener — it only dials out.

Design principle: no speculative constraints. The daemon imposes no workspace
restrictions, no local policy layer, and no configuration beyond what the
connection itself needs.

## Responsibilities

1. **Device identity and connection.** On first start, connect to the gateway,
   receive a claim token, and print it; the user enters that token in the web
   UI to attach the device to their account permanently. The gateway then
   pushes a device token over the same socket, which the daemon persists and
   uses for all subsequent connects. One outbound WebSocket, exponential
   backoff on reconnect; no inbound ports, ever. Device online status in the
   web UI is simply this socket's state.
2. **Per-workspace AgentServer hosting.** One `LocalHost` + harness +
   `createLocalAgentServer` per workspace directory, created lazily on first
   attach — the AgentHub pattern from `packages/web/src/server/agent-hub.ts`,
   owned by the runner package (assembly-level parallel, not shared helper
   code; the hub composes `coding-agent` + `host-local`, which no lower
   package may do). Any existing directory is a valid workspace; a
   `channel_open` for a nonexistent path fails with an error, nothing more.
3. **Credential-free provider assembly.** The daemon holds zero provider
   credentials (design invariant 3). HTTP providers are assembled in gateway
   mode: `baseUrl` → gateway inference endpoint, headers carrying only the
   gateway-issued runner token; the gateway injects real credentials at
   forward time. The Claude Code CLI runs in token mode: the env overlay sets
   `ANTHROPIC_BASE_URL` → gateway and a placeholder `CLAUDE_CODE_OAUTH_TOKEN`;
   the gateway swaps the resulting `Authorization` header for the vault token
   (CLI adoption of the env token is verified in the main design doc).
4. **Control RPC answering.** Provider/model listing, session preparation,
   runner status (see RPC surface below).

Non-responsibilities: user authentication (gateway-only), credential custody
of any kind (gateway vault), session authority (checkpoints write through to
the gateway store from M2), any bespoke shell/tool runtime (everything comes
from the existing local assembly), and serving browsers directly.

## Process shape and local state

Single binary, one command for M1: `demi-runner run [--gateway <url>]`.
First start prints the claim token and waits for the claim; later starts
authenticate with the persisted device token. Service installation is M7.

State lives under `$DEMI_HOME`/`~/.demi` (the layout `host-local` already
owns):

```
~/.demi/
  runner.json          # gateway URL, device id, provider options (claudePath, …)
  runner-token         # device token (0600)
  bridges/, bridge-bin/  # existing createLocalAgentServer state
```

## Connection model: one multiplexed WebSocket

One outbound connection carries the control channel and every relayed agent
connection. Chosen over "control socket + one socket per session" for its
liveness semantics: daemon presence equals the state of this single socket, so
the gateway's device-online flag, session leases, and client interruption
notices all follow it atomically — there is no "control dead but session
sockets alive" ambiguity. Multiplexing is cheap because the agent transport
contract is the 4-method `JsonWebSocket` interface
(`packages/agent/src/websocket-transport.ts:5`): each logical channel gets a
tiny adapter and feeds `AgentServer.attachTransport` unchanged.

### Envelope protocol (lives in the relay protocol package)

Channel 0 is control; channels ≥ 1 each carry one agent transport binding.
Agent frames travel as pre-encoded portable-JSON strings inside the envelope
so the agent-layer codec (`Uint8Array`/`bigint`) is applied exactly once.

| Direction | Message | Purpose |
|---|---|---|
| d → g | `hello { deviceToken?, runner: { name, platform, version, capabilities: { processHost: true } } }` | authenticate (token absent on an unclaimed first start) |
| g → d | `hello_ok { deviceId }` | accepted (claimed device) |
| g → d | `claim_pending { claimToken }` | unclaimed: daemon prints the token, keeps the socket open |
| g → d | `claimed { deviceToken }` | user entered the token in the web UI; daemon persists and is live |
| g → d | `hello_error { reason }` | rejected (revoked device, bad token) |
| both | `ping` / `pong` | liveness (gateway-driven interval) |
| g → d | `channel_open { ch, kind: 'agent', cwd }` | attach a relayed agent connection to workspace `cwd` |
| both | `channel_close { ch, reason? }` | either side detaches one binding |
| both | `frame { ch, payload }` | one agent frame (payload: portable-JSON string) |
| g → d | `control_call { id, method, params }` | control RPC request |
| d → g | `control_result { id, ok, result \| error }` | control RPC response |

Session-open flow: browser `AgentClient` connects to the gateway → gateway
resolves the session's owning runner and workspace → `channel_open` to the
daemon → daemon lazily builds the workspace server, wraps the channel as a
`JsonWebSocket`, and calls `attachTransport`. From there the agent protocol is
end-to-end pass-through: session ids stay client-owned on the `open` frame,
and `SessionOwnershipRegistry` takeover works exactly as in `@demicodes/web`.

Disconnect semantics: when the multiplexed socket drops, every channel closes
with it; the daemon closes all bindings (in-flight turns keep running locally
and keep persisting — the binding-close-must-not-abort rule applies here too),
the gateway marks the device offline and notifies attached clients. On
reconnect the daemon sends `hello` again and clients re-`open`, landing on the
checkpoint-restore path.

## Control RPC surface (daemon-side)

Superset of the existing `/control` shape
(`packages/web-ui/src/transport/protocol.ts:40`), same
`{id, method, params}` wire, tunneled through `control_call`:

| Method | Notes |
|---|---|
| `listProviders()` | existing shape; availability from `provider.state()` (auth is a gateway concern, so daemon providers report ready) |
| `listModels({providerId})` | existing shape |
| `prepareSession({providerId, modelId, thinkingEffort?, serviceTierId?})` | existing shape |
| `listWorkspaces()` | recently used workspace directories |
| `runnerStatus()` | version, platform, capability flags |

Credential and quota state do not cross this surface — both live at the
gateway vault.

## Trust model

- The device token authorizes "this device executes for this user" and
  carries no provider credentials.
- The daemon trusts the gateway for user identity: sessions arriving on its
  channels belong to the claiming user, and the daemon runs them in whatever
  directory the session names. Multi-user sharing of one daemon is out of
  scope.

## Open questions

- Backpressure: whether channel `frame` messages need per-channel flow control
  or the single socket's backpressure suffices (expected: suffices; agent
  frames are small and bounded by view caps).
- Whether the web UI should offer directory creation on the device (e.g. new
  project folder) or directory creation stays a local-machine action.
