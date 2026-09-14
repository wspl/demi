# Resource lifecycle coordination

Status: design only. The shared coordinator and browser idle reclamation are not
implemented. Existing Cloud lifecycle code and `ActivityGate` provide parts of
the mechanism; their presence does not establish the contract below.

This document owns the common rules for resource use, idle deadlines, transition
admission, dependency retirement, and coordinator cleanup. Browser policy belongs
to [Conversation browser](browser.md#idle-reclamation). Cloud policy and durable
machine operations belong to [Managed hosts](managed-hosts.md#lifecycle-and-capacity).
Authorization and every conversation Host access belong to
[Sessions and targets](sessions-and-targets.md#host-operations).

## Purpose and scope

A conversation's browser and its Cloud device both become idle. If separate
cleanup loops run independently, one can shut down Cloud while the other opens a
Host connection to close the browser, waking the device again. A shared
coordinator orders those operations: retire dependent browsers while Cloud is
still available, then save and stop Cloud. Cleanup never counts as new user work.

The backend has one lifecycle coordinator per owning backend instance. It
coordinates resource records independently; slow cleanup on one device must not
block unrelated devices. It is a backend module, not a new service, agent loop,
or replacement for the native runtime or machine provisioner.

The coordinator owns the mechanism. Resource adapters supply policy, readiness
and generation facts, activity sources, and start/retire/recovery operations.
They keep their domain state and errors. The common layer does not acquire a
second copy of tabs, machine state, job tables, agent-tree phases, or reset
journals. Existing admission gates remain the gates of record; their counters
must not be mirrored in another resource manager.

## Ownership and dependencies

Ownership answers who may retain or release a resource. A dependency answers
what must remain available while an operation uses it. They are different:

```text
Ownership                            Execution dependency

User owns Cloud device               Cloud device generation
Conversation A owns browser A          +-- browser A generation
Conversation B owns browser B          +-- browser B generation
                                       +-- shell jobs
```

A browser has a conversation owner and is bound to that conversation's main Host
and directory selection. Cloud has a user owner and can serve several
conversations. Dependency identities derive from those authoritative bindings;
there is no separately editable dependency configuration. Target changes release
the old binding before registering a replacement.

A paired device supplies the same Host dependency and connection-generation
boundary. Its adapter can report available/lost but cannot power down or restart
the user's computer. Browser retirement is identical on paired and Cloud Hosts;
Cloud adds managed wake, disk preservation, and VM retirement at its own boundary.

Each live resource registration names a kind, logical owner, and current
incarnation. An incarnation binds the actual native grant or device generation.
A returning logical device ID does not revive its previous browser grants.
Late completions, notifications, and timers must match the current incarnation
and transition before changing state. Do not build a second generation counter
for a domain that already provides one; use its authoritative identity.

The current resource dependency is Host to browser. Active work is represented
by its existing operation or job lifetime. Do not build a general workflow graph
or migrate every application object into this coordinator.

## Adapter contract

Registration binds an authoritative resource owner to these injected interfaces.
This is an in-process contract; exact TypeScript names are implementation details.

| Adapter contribution | Required meaning |
| --- | --- |
| Identity and availability | Logical owner, actual current incarnation, and domain readiness; no request can substitute another owner or generation |
| Dependencies | Resolve the current Host binding and its admission scope; register changes under the same transition boundary as target changes |
| Activity observation | Current policy facts plus ordered changes and an unsubscribe operation; use existing tree/job/stream owners as the source |
| Policy and guards | Idle duration, eligible activity predicate, and atomic reservations of the authoritative guards used to admit that work |
| Other scheduled work | Optional checkpoint or hard-cap candidates derived from domain timestamps; the coordinator schedules them, the domain validates their policy |
| Start | Start or obtain the resource under admitted demand; return only when its generation is ready, or report partial-start cleanup and failure |
| Retire | Release the specified incarnation under a transition reason and cleanup capability; report confirmed release, established loss, or failure |
| Recover | Establish a safe domain state after failure; reuse the existing durable operation when the domain has one |

The coordinator keeps its registration, lease release handles, idle start time,
and current transition. It reads domain facts instead of copying their
independently writable states. External resource notifications use the existing
schema-validated native/runner boundary. A failed or stale activity observation
cannot authorize retirement; suspend idle eligibility and report the problem
until authoritative observation is restored.

Cloud's checkpoint and hard-cap timestamps stay in its adapter. The coordinator
chooses the next applicable policy deadline alongside idle deadlines; adapters
do not create parallel sweeping timers. Completing maintenance advances the
appropriate domain schedule but does not change the idle start time. If retirement
and a checkpoint are both due, retirement subsumes the checkpoint's disk save.

## Use and retention

A use lease is an admitted operation whose completion releases admission. A
retained resource reference only keeps an object available between uses. The
[native retained grant](native-runtime.md#retained-resources) is the second kind:
it keeps browser state and its service owned, but does not keep Cloud active.

| Kind of participation | Prevents conflicting retirement | Resets idle waiting |
| --- | --- | --- |
| Demand: user/agent operation, browser viewing, or relevant active work | Yes | Yes |
| Retention: idle tabs, a resource grant, or a saved target binding | No | No |
| Maintenance: admitted cleanup or checkpoint work | Yes, while it runs | No |

The admission owner records each lease's purpose. Derive demand and maintenance
activity from those same records; an aggregate busy flag cannot determine whether
to restart an idle interval. Extend the shared admission primitive if needed
instead of mirroring its counts in the coordinator.

A browser operation holds browser admission and the Host admission obtained by
its containing `withHost` operation or shell job. Reuse that existing Host lease;
do not enter the parent again just to represent the dependency. A viewer's frame
subscription is demand for both resources. Releasing one viewer affects only its
own lease. Holding a browser resource without an operation creates no parent
lease.

Agent-tree and job owners provide their current activity and change notifications.
They remain authoritative. A relevant active tree can prohibit idle retirement
without launching a browser or VM that does not yet exist. A stored tree, future
scheduled wakeup, completed job handle, or sidebar entry does not by itself count
as running work. Actual turn/wakeup admission and reservation use the existing
agent-tree lifecycle contract.

Product metadata observers consume published backend state. They do not keep
resources alive merely by receiving tab, control, or lifecycle changes. Acquiring
a fresh Host snapshot is a short demand operation. An open frame subscription is
viewing even if the page itself is not changing. Clients must unsubscribe when
the browser surface is no longer displayed; the server counts the actual live
subscription, not an assumed browser-window visibility state.

## Idle deadlines

Each policy supplies its own idle duration and exact eligibility predicate. Equal
Cloud and browser defaults do not make them one shared countdown. The shared
mechanism works as follows:

1. An available resource becomes idle when all demand has ended and its policy's
   activity predicate permits retirement. Record that instant as `idleSince`.
2. New demand clears `idleSince` immediately, including demand admitted while
   startup is in progress. When demand ends, evaluate eligibility again and begin
   a fresh full interval. A resource never starts just to run an idle timer.
3. Derive the deadline from `idleSince` and the policy duration. At that deadline,
   try to reserve the resource, its required dependency admission, and its policy
   guards. Recheck the current incarnation, elapsed time, and activity under those
   reservations. Observing an idle snapshot is not authorization to retire.
4. If demand won admission, release the attempt and recompute from current facts.
   If maintenance prevents the reservation, defer without changing `idleSince`;
   reevaluate when that maintenance ends.

Use an injected monotonic clock and one coordinator-owned timer for the next
eligible deadline. Subscribe to activity and transition changes to reschedule it;
a domain module must not run a second idle sweep for the same resource. Initial
activity snapshots and subscription revisions must cover changes during
registration, so work cannot be missed between the snapshot and the listener.
A stale timer is harmless after an incarnation or idle interval changes.

Replacing the timer cancels its previous callback. Resource activity listeners
remain installed across timer changes and are released when that resource is
unregistered or the coordinator is disposed. Timers and transient leases are
not persisted. After restart, adapters reconcile their actual resources before
registration and a new idle interval. Durable VM/reset recovery stays in the
managed-host contract.

## Admission and transitions

### Starting and using a resource

For a browser, resource startup acquires the retained native controller. Its
first `open` lazily starts Chrome for Testing inside that controller. The native
owner serializes its process startup without a second backend copy of browser
process state or a native idle timer. For Cloud, resource startup asks the
managed adapter to obtain a running VM and runner generation.

Normal demand enters through the existing authorized operation path. The
coordinator admits demand before startup so retirement cannot overtake it.
Concurrent callers for the same registration share one startup result; each
caller keeps its own cancellation and use lease. Cancelling one caller does not
cancel startup needed by another. If all demand disappears during startup,
request cancellation; if startup still completes, retire the unused result and
release its dependencies instead of leaving an unowned process.

A startup failure reaches every joined caller, releases partial resources, and
retains the adapter's failure information. Automatic crash-loop restrictions and
explicit recovery remain resource policies. There is no automatic replay of a
page action or shell job after startup or connection failure.

For already available resources, startup is skipped. Acquisition of a dependency
is part of the same admitted operation, not a second route around Host access.
Operation success, failure, and cancellation share lease cleanup. Abort signals
remain attached until actual work and its input/stream cleanup have stopped.

### Reserving retirement

A retirement has one owner and a cancellation context. Concurrent requests for
the same resource join or are subsumed by that transition; they never execute
its cleanup twice. Joining a cleanup does not complete a different domain
operation: archive still commits its archive record, and reset still performs its
durable disk transition after any shared retirement finishes. Loss fences the
old incarnation immediately; a late cleanup result cannot restore it.
Requests name a reason such as idle, last-tab closure, owner release, parent
retirement, reset, connection loss, or backend shutdown. Resource
policy determines whether active work may be interrupted for that reason.

Idle retirement never interrupts active work. Explicit archive/target changes,
reset, and shutdown keep their existing admission and interruption policies.
Closing an individual tab still cancels work on that tab. Do not implement a
universal force flag that silently overrides every resource's policy.

Before cleanup begins, cancellation can abandon a retirement attempt and release
its reservations. Once cleanup starts, cancelling a caller only stops that
caller's wait; the coordinator still owns cleanup until the adapter confirms a
safe outcome or records failure. It cannot roll back submitted page effects or
partially saved disks. Other joiners keep their own cancellation. Disposal joins
these owned tasks rather than detaching them.

The coordinator closes new admission before draining work that a transition is
allowed to cancel. A use request during ordinary idle retirement waits outside
held admission gates for that transition to finish, then re-enters the authorized
acquisition path. Reset, archive, owner release, and loss return their specific
status instead. Waiting to acquire does not mean replaying an already dispatched
operation. Retirements do not reverse midway because a new request arrived.
A tab-targeted browser request still fails if its old tab expired during the
wait; only an explicit open creates a replacement page.

A transition involving several gates first attempts the required reservations
without waiting while holding a partial set. If an attempt conflicts, release
what it acquired before waiting or rescheduling. Once reserved, revalidate the
owner/binding and policy guards, then await only the admitted drain/cleanup work.
A forced transition first closes demand admission and cancels affected work,
then obtains the exclusive reservations after their leases drain. New dependency
bindings cannot appear inside a closed scope.

No operation may hold a conversation gate while waiting for a device transition
that itself needs that gate to clean up a browser. The Host admission contract
specifies how a losing acquisition releases its partial gates and waits at the
outer boundary. Idle-tree guards use the same lifecycle admission as new turns;
a provider waiting for its next response is still an active turn.

### Retiring dependencies together

When a browser and its Host are both due, choose the Host retirement and include
its dependent browser resources. After admission is reserved:

1. Prevent new Host demand and browser bindings; capture the affected live
   resource incarnations from the authoritative bindings.
2. Retire browser resources while the Host generation is still available. Use
   lifecycle Host access with the transition's reservation capability. This IO
   is maintenance: it neither wakes a replacement Host nor refreshes idle time.
3. Acknowledge or establish loss of the browser resources, then let the Cloud
   adapter save disks and stop the VM. Finish only when the adapter establishes
   its final outcome.
4. Release reservations and notify dependents. Ordinary future demand may wake
   Cloud through normal Host access and create a fresh browser.

If browser cleanup began first, it holds maintenance admission on the Host.
Host retirement waits for that cleanup to finish and then rechecks its already
elapsed idle deadline. It does not start another ten-minute wait. If Host
retirement won first, an independent browser timer joins that transition rather
than trying to acquire or wake the Host. A maintenance callback cannot recursively
start another retirement of the same resource or acquire ordinary demand inside
its parent retirement.

Retiring a browser does not retire its Host or sibling browsers. The Host's own
policy decides whether it is idle. Conversely, a Cloud retirement may include
idle browsers whose own idle deadlines have not expired; retaining a browser is
not a claim that its parent must stay running. An active viewer or browser
operation prevents ordinary parent idle retirement through its demand lease.

A periodic Cloud checkpoint is maintenance, serialized with shutdown/reset by
the same transition owner. Its maintenance lease can coexist with admitted
demand; it does not require all jobs or viewers to become idle. It preserves
running processes and does not retire browsers merely because execution pauses
briefly. Reset or shutdown waits for this owned maintenance to establish its
outcome before operating on the same VM/disks. The checkpoint schedule and
VM/disk semantics remain Cloud policy. Unattended-job caps are also Cloud policy;
they are not browser deadlines or a reason to drop an active viewer's lease.

## Failure, loss, and disposal

Cleanup completion means the adapter confirmed its required resource release.
A deadline or a closed socket alone is not proof of successful cleanup. The
native resource and managed-host contracts define their cancellation/shutdown
deadlines and escalation; the coordinator does not invent another timeout ladder.
If a native service must be retired, all resources carried by it receive loss.

An unconfirmed child cleanup failure prevents a routine parent retirement from
being reported successful. Release temporary reservations but keep affected
admission blocked until adapter recovery establishes a safe state. Report the
resource, reason, operation, and error. Do not immediately rerun a due timer into
a tight failure loop. If a forced Cloud reset or shutdown can establish resource
loss by terminating the parent, its adapter may do so under that existing policy;
it must still preserve the required disks and report cleanup failure separately.

Unexpected runner/device loss fences dependent browser grants, fails pending
calls with their actual known or unknown outcome, clears timers, and cancels
streams. This path does not call `withHost` to wake the lost device for cleanup.
Native connection teardown and provisioner recovery establish resource release;
unconfirmed termination remains an error. A later connection never revives old
handles. Re-register only after the adapter establishes the new incarnation.

Coordinator disposal closes new demand, clears its timer and activity listeners,
cancels pending acquisitions, and completes or joins transitions under backend
shutdown policy. Retire dependents before parents, then dispose adapters. A
callback arriving afterward cannot register a new resource or reset a timer.
Backend shutdown completion must await owned cleanup and report failures; it
must not leave detached retirement tasks.

The coordinator's admission/transition record is transient. It owns the current
transition and its scheduling state, not a duplicate `running`/`off` flag.
Domain availability comes from the adapter's authoritative state. Durable Cloud
operations remain in the existing store, and tab/control state stays native.
Single-backend authority does not provide distributed worker fencing; that
remains the [backend deployment contract](backend.md#deployment-and-user-ownership).

## Integration and acceptance

[Package boundaries](../package-boundaries.md) owns placement. `backend/lifecycle`
implements the common coordinator using `ActivityGate` and existing cancellation
utilities. `managed` supplies the Cloud adapter and policy; `conversation` supplies
browser ownership, policy, and authorized Host access; `runner` reports connection
and job lifetimes. Native services and the provisioner perform actual cleanup.
There is no new package or protocol for these in-process scheduling decisions.
Extend the authoritative native resource/runner schemas for the required
generation and loss events rather than introducing another transport.

The completed implementation replaces duplicated Cloud/browser idle bookkeeping
and scheduling with this single owner. Generic code consumes injected adapters;
it does not import the product modules that register them. Delivery includes
migrating Cloud to the coordinator and adding the browser adapter, with no second
Cloud sweep retained as a compatibility path.

Use an injected clock, scripted activity, and controlled adapter completions to
verify these races without real models:

| Situation | Required result |
| --- | --- |
| Demand arrives just before or during deadline reservation | Exactly one of use or retirement wins admission; no live action is shut down as idle |
| Two callers start one resource; one cancels | One startup; remaining caller succeeds; all cancelled callers release their leases |
| Every startup caller cancels | Partial or late-created resources are released |
| A retirement caller cancels after cleanup starts | Other joiners remain valid; the coordinator completes or records owned cleanup before disposal |
| Browser and Cloud deadlines expire together | One ordered parent transition; child cleanup does not wake Cloud or extend its idle interval |
| Browser A becomes idle while browser B or a job uses Cloud | Only A retires; B and the job continue |
| Paired-device browser reaches its deadline | Browser resources end; the user's computer and runner remain available |
| New demand encounters parent cleanup needing a conversation gate | Partial admissions are released; no gate cycle or hidden action replay |
| A stream disconnects or becomes hidden | Its subscription releases demand; metadata observation alone keeps no browser or VM alive |
| Checkpoint overlaps an idle deadline | Maintenance completes, then the elapsed idle deadline is rechecked |
| Target switch, archive, or reset overlaps a timer | One admitted transition with the current binding; a stale timer cannot close its replacement |
| Child release or Cloud save fails | Error and blocked/recoverable state remain visible; no successful-release claim or timer retry loop |
| Connection loss, backend disposal, or delayed callback | Dependent handles expire; no orphan leases, timers, listeners, or new work after disposal |

Then verify browser idle reclamation on paired devices and Cloud, and verify real
Cloud save/wake/reset behavior under the same coordinator. Checks above are
acceptance requirements, not completed implementation results.
