# Demi Next: Sessions and Execution Targets

Status: target architecture contract. Implementation acceptance is recorded in `progress.md`.

## Session and target

Every conversation owns one backend agent tree and one execution-target selection.
The root and every subagent resolve that selection per action through
`AgentHarness.host`. Transcripts and command storage belong to the conversation,
independently of its execution device. The backend returns a stable `RemoteHost`
for each device and uses `RemoteShellEnvironment` for shell operations.

| Selection | Resolution | Initial working directory |
|---|---|---|
| Cloud (default) | the user's unique managed device | `/home/demi/sessions/<conversationId>` |
| Device | an explicitly selected paired device | the selected directory, or its reported home |
| Workspace | the workspace's device and path | the workspace path |

A workspace is a named `(user, device, path)` record. It groups conversations;
it does not own a machine or establish a filesystem permission boundary.
New Cloud projects use `/home/demi/projects/<workspaceId>` by default. A user
can select another existing directory. All of that user's Cloud projects and
session directories are directly accessible from the same machine. Different
users never share a managed device or its writable disks.

The control record stores one explicit target selection. A Cloud selection can
exist before the user's managed device is allocated; resolution obtains the
unique device when an operation needs it. Opening a conversation, reading history
and inference that requires no machine do not start a VM. A file operation,
shell job or provider that requires a process obtains the target before execution.
Cloud creation and wake are idempotent per user. Failure returns an operation
error; the backend never silently selects another device.

## Shell semantics

Every script runs in real bash on the selected device. The runner's job table
owns execution, streams, cancellation and output files (`runner.md`). A shell
retains its working directory between jobs; shell variables and background
processes follow the runner's documented job semantics. Cloud jobs run as `demi`
with home `/home/demi` and passwordless sudo. Paired-device jobs use the device
user's environment. Backend-owned execution identifiers are injected per job.
System installs on Cloud survive ordinary shutdown and wake (`managed-hosts.md`).

## Switching

A user can switch between Cloud, paired devices and workspaces while the entire
conversation tree is idle and unarchived. The backend reserves tree and file
admission, validates the destination, commits the selection and advances the
execution-context revision. Concurrent switches have one winner. The next
inference of each node observes its own persisted context block describing the
old and new device and directory; one node cannot consume another's update.

Switching never copies files. A departed device becomes attached with its last
cwd; the incoming device leaves the attachment set. A directory change on the
same device does not add a duplicate attachment. The context explains that old
output paths belong to their original device. When both directories are on
Cloud, the old directory remains directly accessible without cross-host routing.
Already dispatched jobs remain attributed to their original device and node.

## Attached hosts

A conversation has one main device and zero or more attached devices. Device
identity is unique across these bindings. `conversation_hosts` records
`conversation_id`, `device_id`, `name`, `cwd` and `attached_at`, with unique keys
on `(conversation_id, device_id)` and `(conversation_id, name)`.

The user attaches and detaches devices; the agent cannot grant itself access.
Aliases start from device names with a numeric suffix for collisions. The API
can rename an alias; the host menu presents identity, status, promotion and
detach (`host-menu.md`). Attached cwd is updated from each completed cross-host
job and is a starting directory, never a filesystem restriction.

`demi host list` reports main and attached devices. `demi host shell --host
<name|id> <script>` checks the authenticated user's ownership and the conversation
binding before dispatch. A sleeping Cloud device is woken. An attachment alone
does not keep it running. The product offers paired devices in its attachment
picker; Cloud can be attached as a departed main device.

Transfers use ordinary shell pipelines, for example:

```sh
tar c . | demi host shell --host ci 'tar x -C /work'
demi host shell --host ci 'tar c -C /work .' | tar x
```

The backend brokers byte streams between runners (`runner.md` § Pipes).
Attachment changes appear in each node's next execution-context update. Revoking
a paired device terminates its connection and removes its conversation grants.

## Offline targets and recovery

A sleeping Cloud device wakes on demand. An unavailable paired device or failed
Cloud wake yields an ordinary tool error; conversation history remains readable.
No operation is redirected to another machine to hide a failure.

Before dispatch, the agent persists the tool call as executing. On backend
recovery, a recorded result stands; a dispatched call without a recorded result
is completed with an unknown-outcome error and is never automatically replayed.
It may have had partial external effects. An undispatched operation can proceed
through the normal resume path. Runner disconnect terminates its jobs according
to `runner.md`; a reconnect permits new jobs, not resurrection of old processes.

## What persists where

- Conversation trees, transcript blocks and per-node command state live in the
  backend's conversation database. Target switching, VM shutdown and system
  reset do not relocate or erase them.
- Files live on the selected device. Cloud has persistent system and home
  volumes. Project deletion removes its metadata only after no conversations
  reference it; it never implicitly deletes its directory or the user's VM.
  Archiving a conversation does not destroy Cloud or its session directory.
- Full command output lives on the target, with only the model's bounded view
  recorded in the transcript. On Cloud, runner output is under `/run/demi` and
  is temporary; shutdown or reset removes it. Durable results must be written
  to a persistent directory. Output is not separately copied to the backend.
- Browser disconnect does not abort a turn. A new client reconnects to the
  backend and synchronizes the transcript.
