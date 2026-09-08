# Demi Next: Scenario Suite

Status: target acceptance contract; completion requires evidence in `progress.md`.

## Composition under test

```text
Web API → backend → agent → RemoteShellEnvironment → packed tinyjs runner
                                                      → real bash → JS commands
        ← model-visible result, persisted transcript, ledger and blob references
```

The suite uses scripted providers, temporary backend data and real packed tinyjs
runners over sockets. A fake managed provisioner drives lifecycle transitions
using those runners; disk/VM guarantees require a separate Linux/KVM smoke.
No test calls a real model. Package-level suites separately cover schemas, native
primitives, provider adapters, authentication and command parsing.

## World and driver

One world owns the backend, test users, claimed runners, fake provisioner, wire
trace and teardown. It can create multiple conversations/projects per user and
at least two users. Device selection is Cloud, a paired device or a workspace.
Each driver queues scripted provider events, sends a turn and observes the exact
input of the next inference request. File assertions read the target's disk;
conversation state assertions read the cold transcript and scoped backend store.

Restart tests reopen the same data directory and fixed backend port. Runner
restarts use the same device identity and persistent directory. Fake provisioner
results do not certify overlayfs, ext4 or Firecracker behavior.

## Scenario matrix

| Scenario | Required observation |
|---|---|
| File workflow | heredoc, read, edit and list agree with files on the selected machine |
| Output view | long/binary output, nonzero exit and timeout produce bounded model views; full bytes stay in target files |
| Long jobs | status, live stdin, abort and background jobs preserve identity and leave no orphan processes |
| Todo | RPC traverses the local relay; per-node storage survives turns and differs between conversations |
| Subagents | parent/child use the same selected target, independent command state and the correct completion frames |
| Target switch | Cloud → paired device → Cloud changes per-node context; files stay on each original device |
| Same-device project switch | cwd changes while both project trees remain accessible; no duplicate attachment |
| Concurrent conversations | cwd/job handles remain scoped; files are shared on the same user machine |
| Attachments | provider receives inline media while transcript/browser use authorized blob references |
| Client disconnect | turn completes in backend; cold and reattached transcripts agree |
| First Cloud use | no VM for history-only access; concurrent machine requests allocate one logical device and one VM |
| Multiple Cloud projects | distinct project directories reference the same device; a file in one is readable from another |
| User isolation | another user cannot resolve, reset or invoke RPC on the first user's Cloud |
| Managed lifecycle | active turns in any relevant tree and admitted file/process operations prevent idle retirement; wake preserves disks |
| Project/conversation archive | archiving or deleting metadata does not destroy Cloud or delete its directories |
| Reset | all affected jobs end; home remains; system returns to selected base; device and project identities remain |
| Admission races | first use, wake, upload, shutdown, checkpoint and reset serialize without duplicate writers or silent replay |
| Cross-host RPC | unknown/exited jobs, mismatched node/shell and unauthorized devices are refused; valid callbacks use the invoking Host |
| Cross-host pipes | binary stdin/stdout stream end-to-end; cancellation reaches the remote job; control sockets carry no bulk payload |

## Recovery matrix

| Failure | Required observation |
|---|---|
| Backend restart while idle | transcript and command state restore; devices reconnect; new work runs |
| Backend restart during a job | dispatched call without result becomes an unknown-outcome error, never automatic replay |
| Runner death | running jobs fail; reconnect enables new jobs; existing disk files remain |
| Cloud shutdown/wake | system and home changes survive; old processes and temporary output do not |
| Failed checkpoint publication | previous committed generation remains valid; newer working files remain for retry |
| Reset before publication | retained home and previous generation remain recoverable; no new work uses an indeterminate system |
| Reset after publication, boot failure | committed new generation remains authoritative; retry boots it without another reset |
| Broken guest | backend reset works without a runner connection or functioning guest commands |
| Worker ownership loss | old writer is fenced before a replacement VM can access writable disks |

## Teardown invariants

Every world verifies cold/live transcript block equality, bounded runner frames,
correct job attribution, completed or explicitly disconnected jobs, drained pipes
and one usage row per completed scripted provider request. Failed assertions name
the user, conversation and device. No scenario assumes shared machine files imply
shared conversation command storage or authorization across users.

## Real machine verification

Environment-gated Linux/KVM tests exercise `direct` and `jailer` launch modes with
the shipped kernel/base. Install a system package, write a home file, hibernate,
wake and verify both. Reset and verify the system change is gone while the home
file survives. Exercise guest boot failure, independent disk growth, interrupted
saves and reset commit boundaries. Record actual resident memory, peak workload
memory, cold command-ready latency, wake latency and checkpoint I/O separately.

The scenario fixtures remain internal to `backend`; runner primitives and Host
conformance belong to their own package tests. Test doubles for agent contracts
are test-only and do not define an alternative production execution environment.
