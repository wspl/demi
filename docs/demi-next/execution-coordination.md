# Demi Next: Execution Identity and Coordination

Status: final-state contract; acceptance tracked in `progress.md`.

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
user, and actual in-flight execution when deciding whether a machine is idle.
Guest transitions remain owned and serialized by the provisioner.

Every node receives product execution context independently of its profile prompt.
The context carries the latest target switch, attached hosts and any Cloud reset. The backend
uses `context_version` and `last_switch_json`; each node observes the revision
from its own persisted context block before inference. Context
observation is node-scoped and checkpointed with the transcript; one node cannot
consume another node's pending change. The agent's generic context contracts carry
root and node identity without depending on backend database types.

## Cloud admission and recovery

One user owns one Cloud machine. The lifecycle reserves device-wide admission
before shutdown or reset, including file operations, provider processes and
cross-host jobs from every conversation. Tree reservations alone do not protect
a shared device. New work waits for a normal wake; work during reset is rejected
with an explicit resetting status and is never silently replayed.

A reset interrupts all jobs on that device and coordinates the durable disk
operation described in `managed-hosts.md`. A first-use race joins one allocation
and one boot. Device uniqueness is enforced in the control database; process
serialization alone is insufficient. Project and conversation archive operations
do not destroy the machine. A callback remains bound to the live job that
originated it, including when another conversation uses the same device.

## Package responsibilities

- `agent`: root/node identities, tree-wide turn admission and per-node context
  persistence, using its existing node assembly and tree directory.
- `backend/conversation`: target admission and product
  execution context. HTTP file drops use the same target admission.
- `backend/runner`: authenticated RPC routing using live jobs on the sending
  device; invocation-specific Host and node scope.
- `host-remote`: live job facts and their lifecycle; no user or conversation
  authorization policy.
- `runner-protocol` and `runner`: carry the job reference end to end; no agent
  runtime or conversation store on a runner.
- `command-loader` and `shell`: the shared command contract, input schema and
  invocation context.

## Verification

Backend regression coverage must assert cross-user RPC refusal, mismatched and
expired jobs, child storage through cross-host calls, callbacks from an older
still-authorized target, tree-busy switches, concurrent turn admission, uploads
during target changes, shared-device shutdown admission and interrupted reset
recovery. Cloud scenarios cover one device across concurrent projects and
conversations, failed wake, preserved system/home and cross-user refusal. Agent tests cover all
turn entrances and context visibility for root, child and custom-profile nodes.
Runner/remote-host/protocol tests cover job-reference lifecycle and cancellation.
Use scripted providers and scoped package suites; no real-model tests.

## Deployment ownership

Single-instance execution has one owning backend process. Multi-worker automatic
failover requires fencing of an old worker's execution and storage authority;
a user routing key alone is insufficient. This belongs to the scaled-deployment
contract, not the single-instance runtime.
