# Backend scenario acceptance

Scenario tests verify complete backend paths with scripted providers and real
native runners. They complement crate tests, which cover schemas, state
machines, primitive conformance, provider adapters, and command parsing. No
test calls a real model: every suite scripts the model, so a run costs nothing
and answers the same way every time. A check against a real vendor is done by
hand, never as part of a suite.

Three suites drive the whole backend:

| Suite | Where it lives | How it reaches the backend | Model | Hosts |
| --- | --- | --- | --- | --- |
| Backend scenarios | Rust integration tests of the backend crate (`crates/backend/tests`) | HTTP and the conversation WebSocket, with the agent protocol's typed frames | A scripted provider family | Real runner processes; a scripted machine manager for the Cloud |
| Web app contract suite | Tests of `packages/web` | The web application's API client and `ConversationClient`, against the backend executable | A scripted Anthropic-compatible endpoint | A real runner; the backend scenarios' scripted machine manager, which no path asks for the Cloud |
| Real-machine suites | Rust tests that run only when environment variables supply their resources; of them, the Cloud and Claude Code suites and part of the browser suite exist ([Real machine acceptance](#real-machine-acceptance)) | As the backend scenarios | Scripted | A real machine manager, gVisor sandbox, and shipped image; real Chrome; the real Claude Code CLI |

## System under test

```text
Driver: HTTP with a session cookie; conversation WebSocket, typed agent frames
    -> backend in the test process: the edge and the user's shard
    -> the conversation's host access and remote shell environment
    -> runner process with embedded brush
         +-- native command service beside the files
         +-- rpc relay to the invoking node's backend handler

Observe: the next provider request, target files, the cold transcript,
and the usage ledger
```

The scenario harness, the *world*, starts the backend inside the test process,
with its production shard threads and a temporary data directory. In place of
external services it uses a scripted model, serves a scripted machine manager
on a Unix socket, and serves local fixtures in place of models.dev and the
Claude Code release distribution. It loads the
command packages the workspace built as development releases, which the
backend's development store serves to the runners
([Backend deployment configuration](../execution/native-runtime.md#backend-deployment-configuration)).
It pairs real runner processes and owns cleanup. The runners are the real
runner executable, which the test starts from the target directory it runs from
([Validation](builds-and-releases.md#validation)). The fakes come from the
test-support features of the crates that own what they fake
([Crates and packages](../architecture/crates-and-packages.md)); the scripted
machine manager is the backend's own test code, since the manager's crate runs
only on Linux.

The model is scripted in one of two ways: a scripted vendor (`MockVendor`, from
`demi-provider-common`'s `testing` feature) that a built-in family sends to, which
answers each request with the next scripted response and records every request;
or a family the scenario registers, whose runtime it scripts. Title requests
are off unless a scenario turns them on, since one beside the first turn would
take a scripted answer. A *driver* opens a conversation through the HTTP API,
connects to its WebSocket, and sends and reads the typed agent frames.
Assertions inspect what the model receives, not just what a client displays.

A driver reads a transcript as the backend serves it cold, through the
transcript route, and follows events and phases in the frames. It does not
rebuild the live transcript from patches: the web app's patch applier is the
only one, so the [web app contract suite](#web-app-contract-suite) compares
live with cold.

The scripted machine manager speaks `machine-manager-protocol` as the real one does:
operations of one device run in arrival order, requests on a connection are
answered as they finish, and a death reaches every connection. It starts the
same runner as a local process with the boot record's backend URL and token,
over a home that survives stop, wake and reset, and it records wake,
hibernate, checkpoint, reset, and growth calls. A test can hold or fail a
reset, keep a wake's runner from connecting, fail a save, or kill a runner as
a crash would. It does not start a sandbox, mount disk images, or implement
filesystem isolation. A stop clears the runner's state but its log and its
job root, as a Cloud's `/run/demi` goes with a stop while its system image,
which holds those two, stays; a reset clears them too; both keep home.
A successful scripted reset does not demonstrate replacement of system
packages.

Restart tests reuse the data directory. A backend restart whose runners come
back listens at the address it had, since a runner keeps its backend's URL;
one that needs only the same public URL configures that URL and listens on a
port of its own, because another test may take a released port. Runner
restarts reuse device identity and persistent directories. Multi-user scenarios
must use separate authenticated sessions; the world's default helper uses one
session and must not be mistaken for an isolation test by itself.

Scenarios run real runner processes, so they use real time with short
configured windows: a short idle window instead of the product's hour, for
example. A paused clock would advance whenever the runtime waits on a
runner and fire every timer early
([Tests and time](../architecture/concurrency.md#tests-and-time)). In-process
tests on a paused clock pin the exact boundaries of timer rules. A scenario
that needs wall time to pass, such as an expose's expiry, sets an injected
clock instead of waiting.

A scenario never fits a step into such a window. One that looks at a running
Cloud holds its conversation's file gate while it looks (`file_gate` of the
backend's `testing` feature): a lease of that gate is the conversation's work,
so the Cloud idles only after the lease ends, and a lower bound on the stop
counts from there. One that shows an operation a transition holds waits until
the operation waits at that gate (`ActivityGate::waiting`, from `demi-shared-gates`'
`testing` feature), and fails if the operation finishes first.

## Required scenario coverage

The matrix defines the observations each path requires.
[Shared checks and their limits](#shared-checks-and-their-limits) says what the
common checks do not establish.

| Path | Required observation |
|---|---|
| File workflow | Create, read, edit, and list agree with bytes on the selected target |
| Output view | Long/binary output and nonzero exits preserve full target files while bounding model-visible previews |
| Long jobs | Status, stdin, abort, and background jobs retain attribution and do not leave orphan processes; observation timeout does not cancel work |
| Command storage | Todo RPC reaches the invoking node; state survives turns and remains separate across nodes and conversations |
| Subagents | Inherited execution target, independent command state, completion events, and cross-host callbacks retain child identity |
| Target exchange | Cloud/device/project changes update each node's context; files stay on their original device; same-device changes do not duplicate bindings |
| Transitions | While a reset, target change, or archive holds a conversation, opening it, a message, a metadata change, and a Host operation wait and then proceed; a conversation that only has the Cloud attached keeps running through a Cloud reset ([How a conversation uses a device](../execution/sessions-and-targets.md#how-a-conversation-uses-a-device)) |
| Concurrent conversations | cwd and job handles stay scoped while filesystem data is shared on the same machine |
| Attachments | Provider requests receive authorized bytes; the upload answer carries the detected media type and, for a text file, its snippet; persisted references and blob reads enforce ownership |
| Client disconnect | Backend work continues and the client can reattach to persisted results |
| Editing and Fork | History and command-state boundaries are durable and retries are idempotent; external file effects and source children are not replayed or cloned |
| First Cloud use | History-only access does not boot a machine; concurrent execution joins one allocation and wake |
| Multiple Cloud projects | Project directories share one user's managed device and can read each other's files |
| User isolation | Another account cannot resolve, reset, read, or invoke commands on the first account's resources |
| Lifecycle admission | Active trees and admitted file/process operations prevent idle retirement; wake/reset/checkpoint races do not create duplicate writers |
| Archive and project removal | Metadata operations preserve machine identity and files; conversation deletion is not a product operation |
| Reset | Affected jobs end, retained home survives, and the selected system generation becomes authoritative without changing device/project identity |
| Cross-host RPC and pipes | Valid callbacks use the invoking Host; unauthorized or mismatched job/node/device contexts fail; bulk bytes stream separately from bounded control views |
| Host expose | A public URL relays HTTP, streaming, and WebSocket to the device service; another user cannot manage it; expiry, removal, and Cloud stop destroy it and end its connections ([Host expose](../execution/expose.md#acceptance)) |

Use [Commands](../execution/commands.md),
[Sessions and targets](../execution/sessions-and-targets.md),
[Storage](../backend/storage.md), and
[Managed hosts](../cloud/managed-hosts.md) as the authorities for behavior. A
fixture must not define an alternative execution contract.

## Failure and recovery

| Failure | Required result |
|---|---|
| Idle backend restart | History and command state restore, devices reconnect, and new work runs |
| Backend restart after dispatch but before a result | The call has an explicit unknown outcome and is never silently replayed |
| Backend shutdown | Every shutdown step runs even when one fails; open transfers and user streams end before the user's Cloud hibernates, so the Cloud still saves ([Startup and shutdown](../backend/backend.md#startup-and-shutdown)) |
| Runner death | Its running jobs fail; reconnect permits new work; persistent files remain |
| Runner connection lost during a conversation release | The archive, target switch or detach that sent the release succeeds, and the runner hears the release again after it connects ([A release that fails](../execution/resource-lifecycle.md#a-release-that-fails)) |
| Cloud hibernate/wake | System and home changes survive; processes and temporary output do not |
| Checkpoint publication failure | Previous committed generation remains usable; working files remain available for retry |
| Reset failure before publication | Home and the previous generation remain recoverable; new work cannot use an indeterminate system |
| Boot failure after reset publication | The new committed generation stays authoritative; retry boots it without resetting again |
| Broken or disconnected guest | Backend-controlled reset does not depend on guest commands |
| Worker ownership loss | The stale worker is fenced before the new owner gains writable disk access |

The last row belongs to the multi-worker deployment
([Deployment and user ownership](../backend/backend.md#deployment-and-user-ownership));
a suite that runs one backend cannot demonstrate it.

## Shared checks and their limits

No check runs by itself when a scenario closes: each scenario asserts what it
protects, such as the requests its scripted model received. The world does not
compare the live transcript with the cold one; tests for content, media,
queue, and command-state correctness assert those values explicitly on the
cold transcript.

The world does not bound the `job_output` bytes a job sends. The runner's wire
test pins them where the runner sends them, in the job table's tests of
`runner-jobs`: the first 32 KiB of each stream, and
beyond them, while the backend follows the job, at most 16 KiB of the newest
bytes per stream and interval, the last before `job_exit`
([Pipes and output](../execution/runner.md#pipes-and-output)).

The world does not record the runner's wire frames either. `job_exit` and
`pipe_done` are pinned where the runner sends them, by the runner tests of
`backend-remote-host` (`crates/backend-remote-host/tests/host_remote/runner.rs`), which read
every frame their runner sends: each job's end there comes from its
`job_exit`, and they check `pipe_done` for a job's pipes, whole and refused, a
file read whose reader left, and service and network streams. A scenario sees
a missing `job_exit` as a command that never ends. The backend uses
`pipe_done` only to fail a pipe early; the end of the pipe's HTTP exchange is
what settles it.

The usage ledger is checked by the scenarios that read it through
`GET /api/usage`, not after every scenario
([Usage ledger](../providers/usage-and-quota.md#usage-ledger)).

Cleanup belongs to the world and its fake machine manager, including after
assertion failures: drivers detach, runners stop, and the backend closes. Tests
introducing new streams, processes, or failure injection must also verify their
cleanup.

## Web app contract suite

The web app contract suite checks the backend the way the page uses it, and it
is the regression suite for that contract: a change on either side that breaks
the other fails here. It is part of the tests of `packages/web`. It starts the
backend executable with a temporary data directory, calls it through the web
application's API client, and drives conversations with `ConversationClient` from
`@demicodes/conversation-client`, the client the page runs. Its requests
carry what the user's browser adds to the product page's own: the session
cookie, and the page's `Origin` wherever the user's browser sends one
([Authentication and ownership](../backend/backend.md#authentication-and-ownership)).
The generated schemas validate every response and frame on arrival, as they do
in the page
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
The model is an Anthropic-compatible HTTP endpoint that the suite scripts and
the backend's Anthropic API provider calls; tools run on a real runner.

The backend reconciles with its machine manager when it starts, before it
serves, and again when it closes
([Control and ownership](../cloud/managed-hosts.md#control-and-ownership)).
The suite therefore starts the backend with the backend scenarios'
[scripted machine manager](#system-under-test), run as the backend crate's
example program `scripted_machines`, and stops the backend before the
manager. The program prints the manager's socket path as its first line,
which the suite passes as `DEMI_MACHINE_MANAGER_SOCKET`, and serves until its
standard input closes or it is terminated; the runners it started end with
it.

| Path | Required observation |
|---|---|
| Sign-in | A user signs in through the API client, and the session admits the synchronization channel, whose snapshot is the account's, and the conversation WebSocket |
| Create and chat | A conversation created through the API runs a turn, and the client receives the scripted reply and the end of the turn |
| Tools | A scripted tool call runs on the real runner, and its result appears in the transcript |
| Reload | After a reload, the transcript the client assembled from live patches equals the cold transcript the backend serves |

The patch protocol has one more check. The Rust agent tests write patch
sequences together with the snapshot each must produce, and the
`@demicodes/conversation-client` tests apply them with the web app's patch applier
([Frame protocol](../agent/runtime.md#frame-protocol)). With the reload check,
this verifies the patches without a second applier.

## Real machine acceptance

The Cloud's real-machine suite runs the backend against a real machine
manager, its gVisor/systrap sandbox, and a shipped image, with scripted
providers. It reaches the Cloud through normal Host access, as a conversation
does; no test calls a real model.
[Managed hosts — Verification](../cloud/managed-hosts.md#verification) owns the
full persistence, isolation, failure, and platform matrix that a run must
show. Linux amd64 and Linux arm64 are separate acceptance environments.
Record startup phases, memory, and checkpoint I/O separately from functional
assertions. A fake machine manager does not certify
filesystem durability, sandbox isolation, or distributed writer fencing.

Each real-machine suite runs only when environment variables supply what it
needs, and is skipped otherwise, so an ordinary test run needs no Linux host,
Chrome, or Claude Code CLI:

| Suite | What is real | What its environment supplies |
|---|---|---|
| Cloud | The machine manager, its gVisor sandbox, and the shipped image | The manager's socket, a backend URL the manager allows, the manager's state directory, and the native configuration of the command packages the image embeds ([Cloud suite](#cloud-suite)) |
| Browser | Chrome for Testing on a paired device or on the Cloud | The pinned Chrome for Testing executable (`DEMI_TEST_CHROME`), and on the Cloud what the Cloud suite needs |
| Claude Code | The vendor's CLI on the Cloud's runner, calling a local mock of the vendor's endpoint | The CLI executable (`DEMI_TEST_CLAUDE_CODE`) |

Of the browser suite, only the Chrome tests of `demi-browser` exist: they
drive a real Chrome for Testing through the command program on the machine
that runs them, not through the backend or on a Cloud
([Validation](builds-and-releases.md#validation) gives the command). The
Cloud suite opens Chrome on a Cloud but does not watch its live view. A test
of the conversation browser's live view on a Cloud is not written, and whether
to write it is open. Until it is, release acceptance checks what it would
observe by hand on a real Cloud.

Deployment prerequisites are in [Cloud setup](../cloud/setup.md).

### Cloud suite

The Cloud suite is `real_cloud` in the backend's scenario binary; an ordinary
run ignores its tests. Its world is the backend scenarios'
([System under test](#system-under-test)) with the real manager in place of
the scripted one, configured by four variables:

- `DEMI_TEST_MACHINES_SOCKET`: the manager's socket, which the backend
  connects to.
- `DEMI_TEST_CLOUD_URL`: the backend URL the manager allows, its
  `DEMI_MANAGED_BACKEND_URL`. The backend listens on that address and port and
  gives the URL to the Clouds' runners as its public URL.
- `DEMI_TEST_CLOUD_NATIVE`: a native configuration with a development store
  ([Backend deployment configuration](../execution/native-runtime.md#backend-deployment-configuration))
  that names the command package releases the image embeds, so a Cloud's
  runner starts them from the image instead of downloading them.
- `DEMI_TEST_MACHINES_DATA`: the manager's state directory. The suite only
  reads it: it looks inside the generation a checkpoint saved, and finds a
  boot's host processes to measure their memory.

The suite runs on the manager's host, as root. Its tests share the manager,
and a backend that starts reconciles the manager, which stops every Cloud, so
the tests run one at a time. The suite asks the manager directly only for a
checkpoint, whose time the backend's schedule would otherwise choose, and for
a device's committed generation, which it looks inside. It
observes what [Verification](../cloud/managed-hosts.md#verification) lists
with the resource limits off, except the rows that section leaves to release
acceptance. Each test prints what it measures, apart from its assertions: the
first boot until the Cloud runs, the first command, Chrome's first and later
tab, the checkpoint, the stop, the reset, and the peak memory of the Cloud's
processes on the host.

Against a manager installed on the host:

```sh
DEMI_TEST_MACHINES_SOCKET=/run/demi-cloud/machines.sock \
DEMI_TEST_CLOUD_URL=http://<address>:<port> \
DEMI_TEST_MACHINES_DATA=/var/lib/demi-machine-manager \
DEMI_TEST_CLOUD_NATIVE=<native configuration> \
  cargo test --workspace --features demi-runner/test-fixtures --test backend \
  -- --include-ignored real_cloud --test-threads=1 --nocapture
```

On a Linux machine without an installed manager,
`crates/machine-manager/scripts/cloud-suite.sh` runs the same command against a
manager of its own, as root:

```sh
sudo bash crates/machine-manager/scripts/cloud-suite.sh --image <release> \
  --native <native configuration> --work <directory>
```

The script starts the manager the workspace built (`target/debug`) with its
resource limits off, in a stand-in execution host: the init of a throwaway PID
and mount namespace with its own `/run` and an empty, read-only cgroup root,
sharing the machine's network namespace so that the Clouds reach the backend.
Like a host's init, it reaps the processes it adopts, such as the Sentry of a
sandbox that died, which `runsc` would otherwise take for a running one.
Nothing in the stand-in can create a cgroup, so a boot that asked for one
would fail, and the machine's cgroup hierarchies stay untouched. The backend
URL is the machine's address toward the Clouds with a free port; the state
directory, socket, and logs are beneath the work directory. After the run,
also when it fails or is interrupted, the script stops the manager, which
saves every Cloud, ends the stand-in, deletes the manager's nftables table,
restores IP forwarding, and removes the state directory. It then compares the
processes, mounts, loop devices, network interfaces and namespaces, nftables
tables, cgroups, and listeners with their state before the run, and fails when
anything the run made remains.

A Cloud's disks cannot grow where the manager lacks `CAP_SYS_RESOURCE`
([Linux requirements](../cloud/setup.md#linux-requirements)), as in a container
that drops it even for root. There the growth test cannot pass, and the suite
runs without it: the script takes `-- --skip grows_its_home_online` after its
own arguments, and the `cargo test` command takes `--skip grows_its_home_online`
after its `--`.

### Claude Code suite

The Claude Code suite is part of the backend scenarios' test binary and runs
their world with the vendor's CLI in place of a scripted provider
([Validation](builds-and-releases.md#validation) gives the command). The
world's Cloud installs Demi's copy of the CLI through `claude-code.ensure`, from a
local distribution that serves the supplied executable with a manifest the
suite computes, and the provider starts it as in the product
([Claude Code](../providers/claude-code.md#requests-over-stream-json)). The
CLI's inference goes to the scripted Anthropic-compatible endpoint the other
suites use, which the Cloud's runner names in `ANTHROPIC_BASE_URL`. That
runner's environment is the suite's alone, so no proxy of the machine's
reaches the CLI; the CLI's non-essential traffic, telemetry and error
reporting are off, and the account's token is made up, so nothing reaches the
vendor. The scenarios cover what the product relies on from the CLI:

- the install and its verification;
- the `initialize` request with the SDK MCP server, which offers the model
  Demi's tools;
- reasoning and text as they stream;
- a batch of tool calls through Demi's tools, and their results;
- usage;
- Stop in the middle of a stream;
- a new process that replays the transcript, and a change of model and
  effort, which needs one;
- a vendor error as the request's failure.

Each scenario asserts what the product observes: the transcript, the frames,
the usage ledger and what the scripted endpoint received. What the suite cannot
show, a real account against the real vendor, stays a check by hand
([Claude Code](../providers/claude-code.md#acceptance)).
