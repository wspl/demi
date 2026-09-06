# Demi Next: Execution Identity and Coordination

Status: final-state contract; implementation tracked in `progress.md`.

## Identity

A user owns a conversation. A conversation owns one agent tree and one execution
target. Each node owns its transcript and command storage. Each dispatched job
is bound to its node, shell and actual execution Host for its entire lifetime.

```
Zan / conversation c1 / node a1 / shell s3
                    |
                    +-- job j7 on alpha -- runner callback --> a1's command tree
                    |                                        a1's command storage
                    +-- job j8 on beta  -- runner callback --> beta as invoking Host
```

The backend registers a job before sending it. A runner RPC names the job;
the authenticated device connection and the backend's live job record establish
its authority. Node and shell ids supplied by a process must match that record.
The command's arguments are validated against the leaf's existing schema at the
backend boundary. Unknown, exited and disconnected jobs cannot invoke commands.
A callback never selects a Host from the most recently created shell environment.

Cross-host execution preserves the originating node and shell identity. The root
conversation id selects the target and tree store; the node id selects command
storage and the command tree. Device tokens do not authorize other users' jobs.
Pipe authorization remains bound to its device endpoints. Device revocation
terminates the connection and its calls. An ordinary main-target switch leaves
already dispatched jobs pinned to the departed Host, which becomes attached.

## Tree activity and target changes

The agent owns tree-wide turn admission: client sends, queued messages, child
starts, restore, automatic completion wakeups and yield wakeups all enter through
one node-assembly lifecycle boundary. The backend can reserve an idle tree while
committing a user target change. Reservation and turn admission are mutually
exclusive; checking a root session's phase is not a synchronization mechanism.
A live child's turn counts even while it is waiting on a provider with no job.

The backend's conversation target module owns target resolution and transitions.
It coordinates file operations with target changes, including HTTP file drops.
The managed-host lifecycle uses tree activity for every conversation of its
owner, and actual in-flight execution when deciding whether a machine is idle.
Guest transitions remain owned and serialized by the provisioner.

Every node receives product execution context independently of its profile prompt.
The context carries the latest target switch and attached hosts. The backend
uses `context_version` and `last_switch_json`; each node observes the revision
from its own persisted context block before inference. Context
observation is node-scoped and checkpointed with the transcript; one node cannot
consume another node's pending change. The agent's generic context contracts carry
root and node identity without depending on backend database types.

## Hostless eligibility

Hostless scripts execute local work: supported shell constructs and
builtins, file commands, todo commands and observational command leaves. Scripts
that start or resume agent work, or execute on another Host, require a machine
before any statement runs. The backend declares this execution policy alongside
the command manifest; the parser checks all invocations, including pipelines and
expanded command arguments. Help and invalid invocations do not provision a VM.
With an admission policy, root invocations containing unexpanded glob arguments
require a machine: file creation could change their command or option tokens
after preflight. Builtin globbing retains its normal hostless behavior.

A hostless conversation may retain attached Hosts, but executing on one first
upgrades its main target. Spawning a child first upgrades the root conversation;
all children start on its machine. This keeps parent/child waits out of the
hostless-to-machine cutover. No running parent interpreter migrates while waiting
for its child.

```
c1: printf before > note; demi agent spawn ...; cat child.txt
    |
    +-- preflight: requires a machine; no statement has run
    +-- reserve c1's hostless execution and file-operation admission
    +-- finish admitted local work; reject a blocked cutover without replay
    +-- prepare a home image from the quiescent files tree
    +-- provision and commit c1's machine target
    +-- run the entire original script with real bash
```

## Cutover and recovery

One conversation owns one cutover. Admission covers full hostless execution
(including delayed completion after a tool observation window), as well as file
uploads. A cutover blocks new old-target work, waits for admitted operations and
then materializes the files. Work admitted after the cutover resolves its Host
again and executes on the committed target. A wait must be cancellable; a failed
or cancelled attempt never replays a partially executed command. A hostless
command awaiting live stdin may keep its admission lease; the cutover drain
times out after 30 seconds, leaving that script and its files on the current
target. No hostless script is replayed to force a cutover.

The control database records `conversation_upgrades`: `prepared` before
materialization/provisioning, and `committed` in the same transaction as target
binding. The recorded conversation owns the prepared image/device. The target binding is the commit point.
Before it, the hostless tree is authoritative and an uncommitted machine cannot
accept conversation jobs. After it, the home image is authoritative and the
hostless tree cannot accept writes. Recovery completes or discards the recorded
transition before admitting work; it never guesses from arbitrary historical
files. Retiring the source tree is idempotent after the target commit. These
operations span stores; no cross-database transaction is implied.

## Package responsibilities

- `agent`: root/node identities, tree-wide turn admission and per-node context
  persistence, using its existing node assembly and tree directory.
- `backend/conversation`: target admission, Hostless policy, cutover and product
  execution context. HTTP file drops use the same target admission.
- `backend/runner`: authenticated RPC routing using live jobs on the sending
  device; invocation-specific Host and node scope.
- `host-remote`: live job facts and their lifecycle; no user or conversation
  authorization policy.
- `runner-protocol` and `runner`: carry the job reference end to end; no agent
  runtime or conversation store on a runner.
- `command-loader` and `shell`: the shared command contract, input schema and
  invocation context. Generic preflight decisions remain declarative.
- `tinybash`: accepts an embedder's invocation eligibility predicate during
  parse-time checking; knows no agent, VM or backend.

## Verification

Backend regression coverage must assert cross-user RPC refusal, mismatched and
expired jobs, child storage through cross-host calls, callbacks from an older
still-authorized target, tree-busy switches, concurrent turn admission, uploads
on both sides of cutover, and interrupted cutover recovery. Hostless scenarios
cover machine admission before spawn/resume/cross-host scripts, nested parent
scripts, help/error exclusions and shell-state handover. Agent tests cover all
turn entrances and context visibility for root, child and custom-profile nodes.
Runner/remote-host/protocol tests cover job-reference lifecycle and cancellation.
Use scripted providers and scoped package suites; no real-model tests.

## Deployment ownership

Single-instance execution has one owning backend process. Multi-worker automatic
failover requires fencing of an old worker's execution and storage authority;
a user routing key alone is insufficient. This belongs to the scaled-deployment
contract, not the single-instance runtime.
