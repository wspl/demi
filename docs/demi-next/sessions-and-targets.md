# Demi Next: Sessions and Execution Targets

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (switching and offline semantics delivered in M6; hostless in M8; grants and auto-provision in M10) |
| Scope | A conversation's execution target: the three states, hostless execution, switching, grants, offline behaviour, what persists where |

## The session

Every conversation is one AgentSession in the backend plus one metadata row
(`storage.md`). Its **execution target** is a mutable property; the harness
resolves the Host per action from it (`AgentHarness.host`, per-Host
environment reuse, cross-Host handle ownership checks — an existing, tested
mechanism). Action metadata is not checkpointed: the target comes from the
backend's conversation record per action.

## The three states

| State | Record | Where commands run |
|---|---|---|
| **Workspace** | `conversations.workspaceId` | the workspace's host — a user host at a path, or a Cloud managed host |
| **Session-bound managed host** | `conversations.hostDeviceId` | the managed host provisioned for this conversation |
| **Hostless** | neither | `demi` commands in the backend against the conversation's store-backed filesystem; nothing else |

Resolution order: workspace if set, else the session-bound managed host,
else hostless. `workspaceId` and `hostDeviceId` are mutually exclusive.

A **workspace** is a lightweight named entity `(device, path, name)`. It is
only an attribute of conversations: conversations move freely between
workspaces and to hostless, and nothing else hangs off it — no
per-workspace settings or permissions. A **user host** is a device the user
paired through the claim flow (`runner.md`); a **managed host** is a VM the
backend provisions (`managed-hosts.md`).

New conversations start hostless. That is the chat-first default: zero
setup, and most conversations never leave it.

## Hostless execution

In the hostless state the tool call runs in tinybash (`tinybash.md`), the
backend's small shell: a GNU-faithful subset of bash and coreutils over the
store-backed filesystem, plus the root commands dispatched through the
in-process loader (`commands.md`): `demi file`, `demi todo`, `demi agent`
and the rest work against `@demicodes/host-virtual`, a `Host` whose files
live in the conversation's store.

**The upgrade is silent, always.** The `bash` tool is described to the
model as bash and nothing else; the model is never told about tinybash,
its subset or the existence of a machine boundary. The first script that
falls outside tinybash's subset — an unsupported construct or a program
that is neither a builtin nor a root — is run, whole and unchanged, on a
machine the backend provisions and binds to the conversation (session
upgrade, `managed-hosts.md`). From that command on, every tool call runs
there. Nothing is injected into the transcript; the web UI tells the user
the conversation now has a machine.

Silence is possible because **the hostless environment is the managed
host's environment, minus the programs**:

- The hostless filesystem is the managed host's home: same user, same
  `/home/<user>` path, same default cwd; tinybash's `~` is that home. Paths
  the model saw before the upgrade are the same paths after it.
- The backend writes the hostless files into the new host's home at their
  own paths before the first command runs, and hands tinybash's session
  state — cwd and variables — to the real bash job.
- Output formats are GNU's on both sides (`tinybash.md`).

The agent never initiates a target switch of any kind, and no model-driven
migration exists. Managed hosts are a deployment requirement
(`managed-hosts.md`), so there is always a machine to upgrade to; a
provisioning failure surfaces as an ordinary tool error and is retried on
the next tool call.

## Switching

Switching is one generic mechanism with named entrances, all
user-initiated from the web target picker (second confirmation):
hostless → user host, user host → user host, workspace ↔ another
workspace, and the hostless → managed host entrance above. There is no
managed → hostless entrance.

At a turn boundary the backend re-resolves the Host, **grants the departed
host to the conversation** (below), and injects a context block stating the
previous and new target and directory. Files are never moved by code except
in the hostless → managed case, where the store holds them. The context
block also states that full outputs of earlier commands live on the
previous target: artifact paths the model saw before the switch are stale
on the new Host, and `demi host shell --id` reaches them. When the new
target is on the same device, the block notes the old directory is still
directly accessible. A switch requested mid-turn is refused; concurrent
switches have one winner.

## Host grants

A conversation has a **grant set** (`conversation_host_grants`) of hosts
it may reach besides its current target. Grants are created only by the
user: switching the target in the picker automatically grants the departed
host; the user can grant and revoke hosts explicitly on the devices page.
The agent can never grant. The backend checks the grant set before
dispatching a cross-host command (`demi host shell --id`); a host outside
it is refused. Being a grant target does not pin a managed host against
idle reclamation; `shell --id` on a hibernated one wakes it.

The grant set is the trust asymmetry's first answer inside the product
(`overview.md`): the datacenter side reaches only the hosts a user has
named for a conversation.

## Offline targets

When a runner is offline mid-turn, in-flight jobs die and fs calls fail;
these surface as ordinary tool errors and the turn continues or ends — the
session itself is never lost. An offline target leaves the conversation
fully readable and chattable, just unable to touch that machine; the user
may switch it elsewhere.

## What persists where

- **Conversation state** — checkpoints, subagent records, `host_store` —
  goes through `Host.store` and is backend-local (`storage.md`). The journal
  (block rows appended during streaming, never a whole-transcript rewrite)
  is required design.
- **Hostless files** live in the conversation's store and are the source
  for placement on a managed host.
- **Command artifacts** — full command output — are real files on the
  execution target under `Host.commandArtifactsDir`, written by the runner's
  tee (`runner.md`). They follow the target: reachable while its runner is
  online, fetched by reference over HTTP when a user opens a past command's
  full output; a hibernated managed host is woken on demand to serve them;
  an offline user host shows "full output is on an offline host". They are
  not uploaded at hibernation — the value of full output decays fast, and
  uploading it would move the very bytes the wire rule exists to keep off
  the wire. The transcript's bounded views remain always readable.
- **Browser refresh / disconnect** is inherently safe: turns run
  server-side; the client reattaches with `open` + `sync_transcript`. A
  binding close never aborts an in-flight turn.
