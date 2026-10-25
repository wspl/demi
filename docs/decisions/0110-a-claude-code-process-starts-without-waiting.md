---
status: proposed
---

# A Claude Code process starts without waiting on the network, and an idle one closes

## Problem

A conversation's first message, a compaction, a title, or any request that
finds no kept process starts a new CLI process. On a Cloud with two cores,
the first word of such a request comes after 6.6 s at the median and 14 s at
the 90th percentile; a request that takes the process kept from the last turn
gets its first word after 1.5 s. The process start itself, writing the
session and spawning, takes about 0.2 s. The rest is the CLI waiting:

* As it starts, it fetches its feature flags (GrowthBook) and holds its
  startup until they answer.
* Before it sends the turn's request, it waits for two more requests of its
  own to the vendor and for its event logging to start.

Each wait is a TLS connection across an ocean from inside a gVisor sandbox.
Measured on a laptop with the CLI's startup profiler, the flag fetch holds the
startup for 0.5 s and the second wait for 0.4 s, of a 1.2 s start.

Speeding the start up exposes a race that today's slowness hides. The CLI
does not hold a turn for an SDK MCP server that is still connecting: it
lists a pending server's tools later, and offers the model a built-in tool
to wait for them. The provider starts the CLI with no built-in tools, so the
model cannot wait, and a turn that starts before the handshake with Demi's
MCP server ends offers no tools at all. Today the CLI's own network waits
are longer than the handshake, so the first turn has its tools; a CLI that
starts fast, or whose waits fail fast, answers its first turn without them.

Memory is the other cost. An idle CLI process holds about 100 MB of its own,
and the first on a machine about 120 MB more for the executable, which the
others share. A runtime keeps its process until the conversation's tree is
evicted, which waits for every page to leave it and ten more minutes, and a
page keeps up to eight recent conversations attached. So a process outlives
its last turn by hours: on a Cloud used all day, 3.8 processes live on
average, 10 at the peak, most of them idle.

## Decision

A new process answers as soon as its machine can run it, with Demi's tools
from its first turn, and a process nobody uses for five minutes closes. The
user sees a conversation's first word, a compaction and a title sooner, and
nothing else changes.

### The CLI's environment

Every process of a request starts with the CLI's nonessential traffic off and
its feature flags read from the account's disk cache:

```text
CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1
CLAUDE_CODE_GB_DISK_CACHE_WHEN_TELEMETRY_OFF=1
```

* The first turns off the CLI's telemetry, its error reporting, its update
  and announcement checks, and its flag fetch: everything the start waited
  for.
* The second keeps the flags the CLI would use. With traffic off the CLI
  takes its built-in defaults, which may behave differently; with this
  variable it reads the values the account's last flag fetch cached in
  `.claude.json` (`cachedGrowthBookFeatures`), which the CLI's debug log
  reports as `source: growthbook` instead of `source: default`.
* The CLI still prints `rate_limit_event`, from the vendor's response
  headers, so the quota that a turn observes keeps coming.
* The usage probe keeps the traffic on, as it does today: it is the one
  process of the account that fetches the flags.

### Keeping the flags fresh

The CLI refreshes a flag cache older than six hours when it starts with
traffic on, its own refresh interval, and leaves a younger one alone. Two
processes of an account run with traffic on: its usage probe, and its
sign-in, whose CLI fetches the flags as the sign-in succeeds, so a new
account's first process finds them cached.

So when a process of the account starts, and no probe or sign-in of the
account began in the last three hours, or none since the backend started, the
provider runs the account's usage probe in the background. The backend keeps
that time per account with the Claude Code machine, for every provider of the
account, since each session's runtime builds a provider of its own.

A probe every three hours keeps the cache at most nine hours old. The start
that triggers a probe does not wait for it. A probe that fails is logged and
tried again at the next start after three hours, as any probe is.

### Tools from the first turn

The provider hands the CLI the result of the MCP handshake with Demi's server
in the `initialize` request, as the Agent SDK does, and the CLI reads that
request during its startup:

* The CLI starts with `--await-initialize`, so it reads `initialize` from its
  input before it resumes the session's pending turn. Every process writes
  `initialize` first: one whose request offers no tools, such as a title's,
  declares no server.
* The request declares the server, and `--mcp-config` no longer names it.
* Before it writes `initialize`, the provider runs the handshake with its own
  rmcp server in process: `initialize`, `notifications/initialized` and
  `tools/list`. The request carries their results in
  `sdkMcpServerManifests`:

```rust
pub(crate) enum ControlRequest<'a> {
    Initialize {
        #[serde(rename = "sdkMcpServers")]
        sdk_mcp_servers: &'a [&'a str],
        /// The handshake's results by server, which the CLI answers its MCP
        /// client from, so the turn finds the tools connected.
        #[serde(rename = "sdkMcpServerManifests", skip_serializing_if = "BTreeMap::is_empty")]
        sdk_mcp_server_manifests: BTreeMap<&'a str, &'a Manifest>,
        #[serde(rename = "systemPrompt")]
        system_prompt: &'a str,
    },
    // …
}

pub(crate) struct Manifest {
    /// The server's `initialize` result, as rmcp produced it.
    #[serde(rename = "initializeResult")]
    initialize_result: serde_json::Value,
    /// The server's `tools/list` result.
    #[serde(rename = "toolsListResult")]
    tools_list_result: serde_json::Value,
}
```

* The handshake asks for the protocol version the CLI's MCP client asks for,
  which must match or the CLI handshakes over the control channel as before.
  The provider asks for rmcp's newest, and when the CLI's answer
  (`sdk_mcp_manifests_parked`) reports a mismatch, it takes the version from
  the `initialize` the CLI then sends and keeps it with the machine, for every
  provider, so only the first process after the machine's CLI asks for
  another version races its handshake. It logs each mismatch.
* `tools/call` and everything after the handshake still flow as
  `mcp_message` control requests; the server keeps answering them, and a
  handshake the CLI repeats is answered as today.

### An idle process closes

A process is idle when no request uses it: a kept process once the action
that ran it ended, and a prepared one before its request takes it. The
machine's idle processes, of every user, are seated in one registry, which
`PreparedProcesses`, until now one per user, becomes. It lives with the
backend's Claude Code machine:

```rust
/// The idle CLI processes of one machine, of every runtime and user whose
/// processes start there.
pub struct IdleProcesses { /* seats, oldest first */ }

impl IdleProcesses {
    /// Seats a process, idle from now; the seat closes it after five idle
    /// minutes, or when the machine needs its memory.
    fn seat(self: &Arc<Self>, entry: &str, idle: Idle) -> Seat;
}

enum Idle {
    /// Kept by a runtime after its action ended.
    Kept,
    /// Prepared ahead of the request it waits for.
    Prepared,
}
```

* A process idle for five minutes closes, SIGTERM first, whatever pages are
  attached to its conversation. Its runtime then holds none, and the next
  request starts a new process that resumes the session.
* A process waiting on its tool calls belongs to an action and is never idle:
  the action's lease holds it however long its tools run.
* Closing loses nothing a restart cannot rebuild. The request a new process
  sends begins with the same bytes as the kept one's, so the vendor's prompt
  cache still serves it, and with the waits above gone a new process costs
  about 1.3 s on a two-core Cloud.
* A change of an entry's active account closes the entry's prepared
  processes that run in another account's directory. Every user's shard
  follows the change, so one prepared again in the new account's directory,
  by a shard that followed it first, stays.
* Prepared processes now close after five idle minutes too, instead of ten,
  and the limit of two per user goes: the memory rule below covers every idle
  process.

Five minutes is where the replayed history of a Cloud used all day bends:
99% of the turns that took a kept process came within 5.5 minutes of the
last, so closing after five idle minutes starts a new process for 1.1% of
them, and lowers the average of live processes from 3.8 to 2.5.

### Memory

The runner reports its Host's physical memory in its hello, the backend
tells the registry at each start on the machine, and a machine keeps at
most as many idle processes as a quarter of that memory holds at
128 MB each: an idle process measured 100 to 110 MB, and three quarters stay
for the machine's own work and its processes in use. A Cloud with 4 GB keeps
eight. A seat beyond them closes the oldest idle process.

```rust
pub struct RunnerInfo {
    // …
    /// The Host's physical memory in bytes.
    pub memory: u64,
}
```

A process in use is never closed for memory.

### What changes for what exists

* Demi supports only the CLI it installs now, which knows
  `--await-initialize` (since 2.1.261) and the manifest (since 2.1.265). A
  start passes both, and no start runs another way.
* A runner reports its memory in its hello, whose wire version rises from
  63 to 64; the backend refuses a hello of another version, as it does for
  every wire change.
* A session written before this change resumes as before: nothing stored
  changes.

## Spike

**What the start waits for.** A harness started CLI 2.1.296 with the
provider's arguments against a local stand-in for the Messages API and
answered its MCP handshake as the provider does. Signed in with a
subscription's token, the first word came after:

| Configuration                          | Laptop | Two-core Cloud (gVisor) |
| -------------------------------------- | -----: | ----------------------: |
| Today                                  | 1.74 s |                1.72 s\* |
| Traffic off, flags from the disk cache | 0.35 s |                  1.31 s |
| The above, with the manifest           | 0.30 s |                  1.34 s |

\* The stand-in refuses the vendor's other requests at once, so today's
start skips the network waits a real one makes; on the Cloud those cost
about 5 s in production.

The size of the resumed session, 0.1 to 9.5 MB, moved the first word by less
than the noise. In gVisor the remaining 1.3 s is CPU: loading the CLI's code
(0.3 s), its setup (0.3 s), reading the session (0.1 s) and building the
request (0.5 s).

**The tools race.** The request's tools were checked in the stand-in:

| Configuration                                       | Tools in the first request |
| --------------------------------------------------- | -------------------------: |
| Today, laptop                                       |                       5/5 |
| Today, gVisor (the waits fail fast)                 |                       0/5 |
| Traffic off                                         |                       0/5 |
| Traffic off, with `CLAUDE_CODE_MCP_STARTUP_WAIT_MS` |                       0/5 |
| Traffic off, with `--await-initialize`              |                       0/5 |
| Traffic off, with the manifest                      |                       0/5 |
| Traffic off, the manifest and `--await-initialize`  |       10/10 and 5/5 gVisor |

With the manifest, a `tools/call` reached the server as `mcp_message` and
its result reached the next request, and the CLI sent no `initialize`,
`notifications/initialized` or `tools/list` over the control channel. A
manifest whose protocol version differed was reported as
`protocol_version_mismatch` and ignored.

**The flags.** With traffic off and no cache variable, the CLI's debug log
said `mcp runtime arm: v2 (source: default)`; with the variable, `(source:
growthbook)`. A start with traffic on refreshed a cache stamped seven hours
ago, and left one stamped minutes ago unchanged, even after 15 s alive.

**Memory.** On Linux an idle process held 222 MB alone; four held 103 MB of
their own each and shared 120 MB, and the sandbox's memory rose by 108 MB for
each process after the first. `BUN_JSC_forceRAMSize`, `MIMALLOC_PURGE_DELAY=0`
and `BUN_OPTIONS=--smol` changed nothing.

## Prior art

* The [Claude Agent SDK](https://www.npmjs.com/package/@anthropic-ai/claude-agent-sdk)
  captures its in-process MCP servers' handshakes and sends them as
  `sdkMcpServerManifests`, and offers `prewarm()`, a spare process started
  with `--await-claim` that a `claim_session` binds to a new session. Demi
  takes the first. It does not take spares: one holds 230 to 260 MB parked,
  and a claim starts a new session, where most of Demi's starts resume one.
* [tars](https://github.com/devlikebear/tars/pull/888) sets
  `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`, `DISABLE_TELEMETRY`,
  `DISABLE_ERROR_REPORTING` and `DISABLE_AUTOUPDATER` and measured each turn's
  start fall from 1.4 s to 0.38 s. It runs on the default flags; Demi keeps
  the cached ones.
* Zed's [claude-agent-acp](https://github.com/agentclientprotocol/claude-agent-acp)
  starts the CLI through the Agent SDK with no setting of its own for this.

## Alternatives

* **Modify the CLI.** The vendor's terms for running Claude Code in a product
  or a hosted sandbox require the binary as published, and the CLI ships
  several releases a week whose minified names change with each. Nothing
  left to gain needs it: the waits go with documented switches, and the CPU
  and memory that remain are the CLI's runtime itself.
* **Turn telemetry off alone** (`DISABLE_TELEMETRY`). Measured, the first word
  still waits for the CLI's other requests, about 0.7 s on a laptop, and the
  flags fall back to their defaults.
* **Spare processes.** They would hide the remaining 1.3 s for a new
  conversation's first turn only, at 230 to 260 MB each, on a machine whose
  memory the idle rule is there to save.
* **Keep processes longer, or until the tree is evicted.** Each idle process
  costs about 100 MB to save about 1.3 s on a turn that may never come.
* **A shorter idle time.** Two minutes would leave 2.1 processes on average
  instead of 2.5, and start a new process for 2.4% of turns instead of 1.1%.

## Open questions

None.
