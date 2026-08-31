# Runner Daemon

| | |
|---|---|
| Date | 2026-08-31 |
| Status | Proposed (M1 scope of `docs/multi-user-web-gateway.md`) |
| Scope | `@demicodes/runner` — responsibilities, local state, gateway connection model, multiplex protocol, control RPC surface |

The runner daemon turns a user's machine into an execution target for the
multi-user web product. It is a headless sibling of the `@demicodes/web`
server: the same assembly (`LocalHost` + coding harness +
`createLocalAgentServer`, command bridge on by default), minus any HTTP
listener — it only dials out.

## Responsibilities

1. **Device identity and connection.** Pair once with the gateway (device-code
   flow) to obtain a device token; maintain exactly one outbound WebSocket to
   the gateway with exponential-backoff reconnect. No inbound ports, ever.
2. **Per-workspace AgentServer hosting.** One `LocalHost` + harness +
   `createLocalAgentServer` per allowed workspace directory, created lazily on
   first attach — the AgentHub pattern from `packages/web/src/server/agent-hub.ts`,
   owned by the runner package (assembly-level parallel, not shared helper
   code; the hub composes `coding-agent` + `host-local`, which no lower
   package may do).
3. **Local provider assembly.** All five providers built from the machine's
   existing credentials (`~/.claude` keychain/pool, `~/.codex/auth.json`,
   `~/.grok/auth.json`, env API keys). From M3 on, assembled in gateway mode:
   `baseUrl` → gateway inference endpoint + runner-token header; claude-code
   via the CLI env overlay option. Credentials never leave the device.
4. **Control RPC answering.** Provider/model listing, session preparation,
   workspace validation, health and quota snapshots (see RPC surface below).

Non-responsibilities: user authentication (gateway-only), session authority
(checkpoints write through to the gateway store from M2), any bespoke
shell/tool runtime (everything comes from the existing local assembly), and
serving browsers directly.

## Process shape and local state

Single binary, `demi-runner`, two commands for M1:

- `demi-runner pair <gateway-url>` — device-code pairing; prints the code /
  verification URL, polls, persists the device token.
- `demi-runner run` — foreground daemon (service installation is M7).

State lives under `$DEMI_HOME`/`~/.demi` (the layout `host-local` already
owns):

```
~/.demi/
  runner.json          # gateway URL, device id, allowed workspace roots,
                       # provider options (claudePath, …)
  runner-token         # device token (0600)
  bridges/, bridge-bin/  # existing createLocalAgentServer state
```

## Connection model: one multiplexed WebSocket

One outbound connection carries the control channel and every relayed agent
connection. Chosen over "control socket + one socket per session" for its
liveness semantics: daemon presence equals the state of this single socket, so
the gateway's runner-online flag, session leases, and client interruption
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
| d → g | `hello { deviceId, token, runner: { name, platform, version, capabilities: { processHost: true } } }` | authenticate the connection |
| g → d | `hello_ok { runnerId }` / `hello_error { reason }` | accept / reject |
| both | `ping` / `pong` | liveness (gateway-driven interval) |
| g → d | `channel_open { ch, kind: 'agent', cwd }` | attach a relayed agent connection to workspace `cwd` |
| both | `channel_close { ch, reason? }` | either side detaches one binding |
| both | `frame { ch, payload }` | one agent frame (payload: portable-JSON string) |
| g → d | `control_call { id, method, params }` | control RPC request |
| d → g | `control_result { id, ok, result \| error }` | control RPC response |

Session-open flow: browser `AgentClient` connects to the gateway → gateway
resolves the session's owning runner and workspace → `channel_open` to the
daemon → daemon validates `cwd` against its allowed roots, lazily builds the
workspace server, wraps the channel as a `JsonWebSocket`, and calls
`attachTransport`. From there the agent protocol is end-to-end pass-through:
session ids stay client-owned on the `open` frame, and
`SessionOwnershipRegistry` takeover works exactly as in `@demicodes/web`.

Disconnect semantics: when the multiplexed socket drops, every channel closes
with it; the daemon closes all bindings (in-flight turns keep running
locally and keep persisting — the M4 binding-close rule applies here too), the
gateway marks the runner offline and notifies attached clients. On reconnect
the daemon sends `hello` again and clients re-`open`, landing on the
checkpoint-restore path.

## Control RPC surface (daemon-side)

Superset of the existing `/control` shape
(`packages/web-ui/src/transport/protocol.ts:40`), same
`{id, method, params}` wire, tunneled through `control_call`:

| Method | Notes |
|---|---|
| `listProviders()` | existing shape; availability from `provider.state()` |
| `listModels({providerId})` | existing shape |
| `prepareSession({providerId, modelId, thinkingEffort?, serviceTierId?})` | existing shape |
| `listWorkspaces()` | allowed roots + recently used dirs (replaces `defaultWorkspace`) |
| `validateWorkspace({path})` | daemon-enforced allowlist check |
| `runnerStatus()` | version, platform, capability flags, per-provider auth state + latest quota snapshots |

Provider auth/quota data crossing this surface is public metadata only
(`ProviderAuthState`, quota snapshots) — never tokens or raw auth files,
matching the existing provider secret boundaries.

## Security model

- **Workspace allowlist is enforced by the daemon**, not the gateway: a
  compromised gateway cannot direct the daemon to run an agent in an arbitrary
  path. `channel_open` outside the allowlist is refused.
- The device token authorizes "this device executes for this user" and
  nothing else; it carries no provider credentials.
- The daemon trusts the gateway for user identity (sessions arriving on its
  channels belong to the paired user); multi-user sharing of one daemon is out
  of scope.

## Open questions

- Backpressure: whether channel `frame` messages need per-channel flow control
  or the single socket's backpressure suffices (expected: suffices; agent
  frames are small and bounded by view caps).
- Whether `listWorkspaces` should support creating new directories under an
  allowed root from the web UI, or directory creation stays a local-machine
  action.
