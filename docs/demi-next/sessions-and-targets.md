# Demi Next: Sessions and Execution Targets

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Delivered (switching and offline semantics in M6; hostless in M8; the session upgrade and managed hosts in M11; attached hosts 2026-09) |
| Scope | A conversation's execution target: the three states, hostless execution, switching, attached hosts, offline behaviour, what persists where |

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

Silence is possible because **the hostless environment is a subset of the
managed host's environment, and every script that would touch anything
outside the subset is moved before it runs.** Three things make that
precise.

**The namespace.** The hostless filesystem consists of exactly two
subtrees, `/home/demi` and `/tmp`, plus `/dev/null`. Nothing else exists,
and nothing else is pretended: `/etc`, `/usr`, `/proc` are not empty
directories, they are outside. The store-backed Host keeps what the
managed host would show — mode, mtime, case-sensitive names, owner fixed
to `demi` — so `ls -l` reads the same on both sides. It holds no symbolic
links: nothing hostless can create one, and a drop or upload that carries
one is itself an upgrade (`tinybash.md` § Semantics).

**The upgrade condition**, decided entirely at parse time, before any
statement runs (`tinybash.md`). A script is outside the subset when any
of these holds:

| Condition | Example |
|---|---|
| a construct outside the grammar | `$(…)`, `for`, `&` |
| a command word that is neither a builtin nor a root | `python3`, `git`, `date`, `which` |
| a builtin flag outside its whitelist | `grep -P`, `sed -i` |
| a path outside the namespace, anywhere a path can appear, under any state the script may be in | `cat /etc/os-release`, `> /var/log/x`, `cd /`, `ls /usr/*`, `cd missing; cat ../x` |
| a root-command argument declared as a path that resolves outside | `demi file read /etc/hosts` |

Relative paths resolve against the cwd, which is always inside the
namespace because `cd` outside it is itself a condition; a `cd` whose
success cannot be decided before the script runs is checked both ways
(`tinybash.md` § Semantics). Root-command arguments are checkable because
the manifest marks path-typed arguments (`commands.md`) and the loader
resolves them before dispatch. The one
condition that can only be seen at run time — the hostless storage quota
— is not an upgrade trigger: the script has already run in part, so it
fails the command with an `ENOSPC`-class error, the same thing a full
disk does on a machine; the quota is sized so that this is a genuine
fault, not a normal event.

**What moves.** The new host's home image is built from the hostless
tree — both subtrees, with mode and mtime intact — by `mke2fs
-d` before the VM boots (`storage.md`), so the first command already runs
on a complete home; the backend hands tinybash's session state — cwd and
variables — to the real bash job. The environment table is shared: the hostless `env`
(`HOME`, `USER`, `PATH`, `PWD`, `SHELL`, `LANG`) is the table the managed
host's login environment is generated from, so `echo $PATH` prints the
same string on both sides. Output formats are GNU's on both sides
(`tinybash.md`). Hostless has no processes, no background jobs and no
output files, so nothing else exists to move.

**Verification: split equivalence.** A sequence of tool calls is run
twice — once entirely on a machine, once split at an arbitrary point with
the first part hostless, the upgrade, and the rest on the machine. Every
tool result and the final state of both subtrees must match byte for
byte, at every split point. This test is the design's definition of
"silent".

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

At a turn boundary the backend re-resolves the Host, **attaches the
departed host to the conversation** (below) with its directory as the
attachment's working directory, and injects a context block stating the
previous and new target and directory. Files are never moved by code
except in the hostless → managed case, where the store holds them. The
context block also states that full outputs of earlier commands live on
the previous target: output paths the model saw before the switch are
stale on the new Host, and `demi host shell --host` reaches them. When
the new target is on the same device, the block notes the old directory
is still directly accessible. A switch requested mid-turn is refused;
concurrent switches have one winner.

## Attached hosts

A conversation has one **main host** — its execution target, where the
`bash` tool runs — and a set of **attached hosts** it may reach besides
it. The main host is what the three states above describe; attached hosts
are rows of `conversation_hosts`:

```
conversation_hosts
  conversation_id   device_id   name    cwd             attached_at
  c1                dev-9f2a    ci      /srv/ci/app     …             ← attached by the user
  c1                dev-01c7    ci-2    /home/a/app     …             ← the departed target of a switch
PRIMARY KEY (conversation_id, device_id)      UNIQUE (conversation_id, name)
```

- **Identity** is the device. A device is attached to a conversation at
  most once.
- **`name`** is what the model and the user call the host. It is seeded
  from the device's hostname when the host is attached, made unique within
  the conversation with a numeric suffix when two devices share one, and
  renamable by the user. `demi host shell --host` resolves it, and also
  accepts the device id; `demi host list` prints both.
- **`cwd`** is where work on that host last stood: the directory the last
  `demi host shell --host` there ended in, written back from the job's
  exit (`runner.md` § Jobs and the tee), and where the next one starts.
  An explicitly attached host starts in its home; the departed target of a
  switch starts in the directory it was left at. The column carries no
  permission and draws no boundary — it is the same thing the main host
  carries between jobs, kept once per attached host.

The attachment set is independent of the main host's state. A hostless,
user-host, workspace or managed main host reaches its attached hosts the
same way, attaching is offered in every state, and nothing in the product
or the backend narrows the combination — a hostless conversation with
attached hosts is not the expected shape, but it is a supported one.

Attaching is the user's act alone: switching the target automatically
attaches the departed host; the user attaches and detaches hosts on the
conversation's host list (`POST /api/conversations/:id/hosts { deviceId
}`, `PATCH …/hosts/:deviceId { name }`, `DELETE …/hosts/:deviceId`,
`backend.md`). The backend accepts any device the user owns; which
devices the product offers for attaching is the product's choice
(`product.md` — managed hosts never appear there, so they enter the set
only as the departed target of a switch). The agent can never attach.
Switching the main host to an attached host removes its row — a host is
main or attached, never both — and the departed target takes a row in
its place; the removed row's `cwd` goes with it, the new main host's
directory is the one the picker named. A change to the set is
announced to the model at the next turn boundary by a context block
listing the attached hosts with their directories, the same mechanism as
the switch announcement. Revoking a device detaches it from every
conversation — an attachment is a permission, gone with the machine.

The backend checks the attachment set before dispatching a cross-host
command (`demi host shell --host`); a host outside it is refused. Being
attached does not pin a managed host against idle reclamation; `shell
--host` on a hibernated one wakes it. Copying between hosts is the shell
idiom over a pipe (`runner.md` § Pipes): `tar c . | demi host shell
--host ci "tar x"` pushes, `demi host shell --host ci "tar c ." | tar x`
pulls; there is no separate copy verb, `tar` already defines the
semantics and the pipe carries it byte for byte. `tar` is a tinybash
builtin (`tinybash.md` § Builtins), so a hostless conversation copies to
and from its attached hosts without acquiring a machine.

The attachment set is the trust asymmetry's first answer inside the
product (`overview.md`): the datacenter side reaches only the hosts a
user has named for a conversation.

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
- **Hostless files** live as a tree in the conversation's database with
  their bytes in the blob store, and become the home image at upgrade
  (`storage.md`).
- **Command outputs** — full command output — are real files on the
  execution target under `Host.commandOutputDir`, written by the runner's
  tee (`runner.md`). They stay there: the model reads them with ordinary
  commands while the target is its Host, and the transcript keeps exactly
  the view the model saw, which is also all the browser shows. Nothing
  fetches them back to the backend and nothing uploads them at hibernation
  — their value decays fast, and the wire rule exists to keep those bytes
  off the wire.
- **Browser refresh / disconnect** is inherently safe: turns run
  server-side; the client reattaches with `open` + `sync_transcript`. A
  binding close never aborts an in-flight turn.
