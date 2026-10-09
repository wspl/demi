# Native command execution

This document defines native command execution: binding a command to a release,
installing its executable, and running it in a shared service. The execution
contract comes first; protocol, publication, and build requirements follow.

The runner is a Rust process with an embedded brush shell. Native command
algorithms run in separate executables that stay running to serve multiple calls.
These *resident services* can be released independently of the runner. The runner
embeds no JavaScript engine.

Demi's native commands belong to two packages, one per capability:
`demi.file`, the file commands, and `demi.browser`, the conversation browser.
Each is its own resident program and its own release, so a job that edits a
file downloads and starts only the small file program, and a browser release
does not re-release the file commands. The package that installs the Claude
Code CLI, `demi.claude-code`, uses the same contract
([The package](../providers/claude-code.md#the-package)). Shell utilities and
`rpc` handlers have their own owners.

## One command, from declaration to result

The following sections trace `demi file patch` on a laptop. The backend selects
the command's release, the runner installs the laptop executable, and a
resident service applies the patch to the local file.

The process diagram shows the native execution path. Each box is a process;
arrows show requests, labeled with their contents and transport. Response paths
are omitted. The runner and command service execute on the laptop.

```text
Native command execution: process communication

+----------------------------+
| Backend process            |
| Defines commands and jobs  |
+----------------------------+
              |
              | Job + pinned manifest
              | MessagePack / WebSocket
              v
+----------------------------+
| Runner process             |
| Runs shell and dispatches  |
+----------------------------+
              |
              | Native invocation
              | HTTP/2 over stdio
              v
+----------------------------+
| Command service process    |
| Executes native operations |
+----------------------------+
```

The patch handler works beside the file. The caller supplies the patch and
receives output and an exit status; the backend does not need the original file.
Keeping algorithms in an independent service allows command releases without
rebuilding the runner.

For file mutations the runner also supplies the job-owned `EditContext`. The
service uses the shared recorder when publishing or restoring file contents;
the runner reads its journal at job completion. The recording contract, limits
and lifetime are defined in [Edit tracking](edit-tracking.md).

The responsibility boundaries are:

| Owner | Responsibility |
| --- | --- |
| Command set | Declare names, help, argument types, and logical bindings; bind `rpc` handlers. |
| Execution adapter in the backend | Validate bindings against the startup catalog, build manifests, and answer artifact location requests. |
| Backend artifact module | Publish complete releases and sign artifact downloads. No command algorithms. |
| Runner | Validate dispatch, install artifacts, own service processes and their retention, and route invocations. |
| Shared command-sdk | Handle framing, HTTP/2, byte IO, and cancellation over a supplied transport. |
| Command package | Implement operations, validate their arguments, and release operation resources. |

The shared SDK owns no artifact or process management. Brush builtins and external
command clients use the same dispatcher, which supplies validated operation
metadata to the native service. `rpc` calls go to the backend instead. The
native service never receives raw CLI requests.

Related contracts define the surrounding behavior:

- [Command declarations](commands.md): CLI parsing and manifest semantics.
- [Local forwarding](commands.md#external-command-clients): external clients and endpoint access.
- [Runner jobs](runner.md#shell-jobs): brush, profiles, and whole-job cleanup.
- [Crates and packages](../architecture/crates-and-packages.md#module-layout): source module layout and ownership.

## Bind an exact package

The command declaration identifies an operation; the startup catalog selects
its release. These terms have distinct meanings:

| Term | Meaning in the patch example |
| --- | --- |
| Declaration | Names package `demi.file` and operation `file.patch`. |
| Catalog | Maps `demi.file` to one selected release descriptor. |
| Descriptor | Identifies that release, its operations, and each platform executable. |
| Manifest | Pins the descriptor hash and operation for the job. |
| Artifact | The executable bytes selected for the laptop's platform. |

Pinning the manifest prevents a later release or download URL from changing what
an existing job executes. The backend composes the catalog at startup from its
[deployment configuration](#backend-deployment-configuration) and gives it, with
its artifact resolver, to the execution adapter that builds every conversation's
shell environments. The command tree declares the binding: the `patch` leaf of
the `demi file` group names package `demi.file` and operation `file.patch`.
Its help and argument type follow the
[command declaration contract](commands.md).

Each package ID resolves to exactly one immutable descriptor within the
catalog. Duplicate IDs, incomplete packages, and unknown operations fail
initialization. Declarations do not import compiled-in release descriptors.

Each shell environment serves one Host for one agent node, and carries that
node's command set, the conversation's identity, and the catalog. Each
invocation retains its authorized context even when several environments share
a catalog. The agent itself has no object-store dependency; package selection
does not belong in model settings.

A release consists of its descriptor and one executable for each target it
carries; a published release carries all six of the
[release target matrix](#publish-a-complete-release). The descriptor contains:

| Descriptor field | Meaning |
| --- | --- |
| `id` | Stable namespaced identity, such as `demi.file`. |
| `version` | Human-readable release version, immutable within its publisher. |
| `protocolVersion` | Command-service wire major version. |
| `operations` | Unique operation IDs supplied by the package, written from the package's Rust operation enum when the release is packaged. |
| `targets` | The target triples the release carries, each with executable SHA-256 and byte size. |

Software a program needs beside itself, released by someone else, is the
program's own concern, not the release's. For example, `demi-browser` needs
Chrome for Testing: its package's record pins the version and, per platform,
the official archive's URL, size and SHA-256, and the program asks the runner
to install it from that URL when the agent runs `demi browser install`
([Browser distribution](../browser/browser.md#browser-distribution)). The
release carries only the program.

A package declares its operations once, as the operation enum its service
routes by, so the descriptor, the service's own answer to `GET /v1/info`, and
the code that handles each operation cannot list different sets.

The descriptor hash is the SHA-256 of its canonical JSON (RFC 8785). Paths,
object keys, and URLs locate artifacts but do not participate in package
identity.

The runner independently validates descriptors and references. Publication
rejects reused id/version pairs with different content, unsupported wire versions,
and missing targets. An input-schema change affecting native behavior requires a
matching release. The runtime does not repair arguments or fall back to another
version.

Command definitions and catalogs remain fixed for the backend's lifetime. New
definitions take effect when the backend restarts. The catalog does not support
live updates. Existing contexts retain their pinned binding. Connection loss and
context disposal follow the [runner command lifetime](runner.md#command-lifetime).

## Install artifacts

An **artifact** is a file the runner installs on its Host for a command
package: the package's executable, or anything else the package's program
needs there, such as `demi.browser`'s Chrome for Testing or
`demi.claude-code`'s Claude Code CLI. Every artifact is installed the same
way, through one cache, and reported the same way while it installs. What
differs is only who asks for it and why:

| Artifact | Who asks, and when | Where its digest comes from |
| --- | --- | --- |
| A package's executable | The runner, before it starts the program | The pinned descriptor |
| Software the program installs from its official source, such as Chrome for Testing | The program, through the [artifacts stream](#the-artifacts-stream), when its user asks, as `demi browser install` does | The program's own record, which also names the source |
| A file the backend chose at run time, such as a Claude Code CLI version | The program, through the artifacts stream, during the call that named it | The record the backend gave the call |

An artifact has a **form**: `file`, one executable file, or `archive`, a zip
or `.tar.zst` archive, told apart by its first bytes, that is unpacked and that names its **entry**, the file its user
starts, as a relative path with `/` between the components. It has the
package it belongs to, a **name** and a **version** for the user, such as
`Chrome for Testing` and `153.0.8010.36` (an executable's name is `program`
and its version the package's), and the SHA-256 and size of its bytes. The
package and the name together are the artifact's **line**: the versions of
one thing, of which the Host needs only the newest.

For example, the first `demi browser install` on an arm64 Mac:

```text
runner   descriptor of demi.browser 0.1.3, target aarch64-apple-darwin
           install demi.browser / program 0.1.3, file          -> starts the program
           cache? image? no -> asks the backend where, downloads
program  install: Chrome for Testing 153.0.8010.36 is needed
           artifacts stream: install Chrome for Testing 153.0.8010.36,
           archive with entry chrome-mac-arm64/.../Google Chrome for Testing,
           from https://storage.googleapis.com/chrome-for-testing-public/...
runner     cache? no -> downloads from that URL, verifies, unpacks
           -> answers the entry's path
```

### Where an artifact comes from

The runner takes the first of these that it has for an artifact's digest:

1. A verified entry of its own cache.
2. A verified copy that the Host's image preinstalled
   ([Preinstalled artifacts](#preinstalled-artifacts)); a Mac has none.
3. A download from the URL the program's request names, when it names one.
4. A download from where the backend says, for a package's executable or an
   artifact the backend attached to a stream.

The download request names the artifact's exact digest and target and the
live work it serves: the job and the hash of the manifest it runs with, or a
[service stream](runner.md#service-streams). The runner knows which: it
installs an executable for the job or stream that needs the service, and a
program's request names the invocation it serves, which the runner started
for one job or one stream.

The backend answers only for live work on that connection that the artifact
belongs to:

- the executable for the target of a package the work binds: for a job, a
  package its pinned manifest with that hash names; for a stream, the
  stream's package ([Bind an exact package](#bind-an-exact-package));
- an artifact the backend attached to the stream when it opened it, with its
  location. For example, the backend opens `claude-code.ensure` with a
  release record and attaches each of the record's downloads, located at the
  vendor's official URL ([The package](../providers/claude-code.md#the-package)).

At most 32 requests are answered at a time on a connection; one beyond that, or
one that repeats an id still in flight, is refused. When the job exits or the
stream ends, its outstanding requests are cancelled and get no answer. The
backend validates the location before sending it.

The location is an HTTP or HTTPS URL without credentials. A package's
artifact lies in the deployment's object store
([The object store](../backend/storage.md#the-object-store)). With an S3
store, the backend signs an HTTPS GET URL valid for five minutes, so private
storage needs no change to its bucket policy; the runner downloads directly
from storage, and credentials remain in the backend. With a local store, the
location is the backend's own `/native-artifacts/<sha256>` on its public URL
([Backend deployment configuration](#backend-deployment-configuration)). An
attached artifact's location is the one attached. The scheme cannot change
what runs: the runner checks the download against the size and SHA-256 it
was given, which reached it over its authenticated connection to the
backend, in the pinned descriptor or in the call's own input.

The runner obtains URLs on demand. URLs never become package identity or permanent
manifest fields. An expired URL can be refreshed for the same digest. Other
download failures do not count as expiration.

To download, the runner completes these steps:

1. Download to a temporary file, decoding the response's content coding, and
   enforce the declared size on the decoded bytes. The runner accepts `zstd`
   and no coding; a response with any other coding fails the download. It
   follows up to ten redirects, as GitHub's release downloads need; the size
   and SHA-256 it checks are the record's, whatever host answers.
2. Verify the size and SHA-256 of the decoded bytes.
3. For a `file`, apply executable permissions where required and publish the
   verified file atomically into the cache as `<sha256>`. For an `archive`,
   unpack it into a temporary directory, check that its entry is there,
   write a receipt naming the archive's SHA-256 and the entry's, and publish
   the directory atomically into the cache as `<sha256>/`; the archive itself
   is not kept. This is the artifact library's archive installation, the one
   the Cloud image build uses.

A download has no overall deadline, since an artifact of hundreds of megabytes
takes minutes on a slow link; it fails when the
connection makes no progress for 60 seconds, or when its work is cancelled. A
command that installs on purpose, such as `demi browser install`, reports its
download in its own output, a line at each tenth of the size and one as it
unpacks, which the agent reads as it reads any command's output; an artifact
a command needs on its first use installs silently, before the command
starts.

Concurrent callers share one download per digest. Cancelling one caller preserves
a download still needed by another. If the download fails or is cancelled, the
runner releases the response and removes the temporary file or directory. The
runner never executes partial or mismatched files.

### The cache

The cache is private to the runner's user and only publication writes it, so
an entry is verified once, as it is published. A later use takes the entry
without reading it: for a `file` the runner checks only that it is a regular
file of the declared size, for an `archive` only that the directory holds a
receipt naming the archive's SHA-256, and a mismatch fails the install
rather than being repaired. Hashing the entry again would make every service
start read the whole executable or archive.

Beside each entry the cache records its line and version, and it records an
[image copy](#preinstalled-artifacts) it used the same way, without copying
it, so `installed` can name both. Once an artifact is
installed, the runner removes the other artifacts of its line, except those a
running service of this runner holds: the service's own executable and what
it was given through its artifacts stream. Those go at a later install of the
line. So a Host keeps one Claude Code CLI, one Chrome and one executable of
each package, however often each is updated, beside what still runs.

The cache is `artifacts/` in a paired device's installation state. A Cloud's
is `.demi/artifacts` in the home of its `demi` user, on the home image, which
stops, wakes and resets keep ([Images](../cloud/managed-hosts.md#images)): a
Claude Code CLI downloaded once stays until a newer one replaces it.
`DEMI_ARTIFACTS` names another directory. Runners of one user may share one:
an artifact is published under its digest, an archive under a lock file
beside it, so two runners that install one artifact at once download it once
or twice but publish one copy, and neither fails; each removes only what its
own services do not hold, so runners of different releases that share a
cache can remove each other's older versions, which they then download again.
The [development backend](../backend/backend.md#one-command-development-backend)
gives its Cloud's runners one directory that outlives them for this reason.

### The artifacts stream

A program asks the runner for artifacts over a stream the runner opens, as
it opens the [numbers stream](#conversation-numbers):

```text
Service                        Runner                          Backend
  |<-- POST /v1/artifacts --------|  opened after GET /v1/info     |
  |--- {id 4, install: {...}} --->|  cache? image? no              |
  |                               |--- artifact_resolve ---------->|
  |                               |<-- artifact_location ----------|
  |                               |    download, verify, unpack    |
  |<-- {id 4, path} --------------|                                |
```

- Once it has checked a new service's catalog, and before it admits a call,
  the runner opens one `POST /v1/artifacts` request with the metadata `{}`.
  It stays open for the service's life. The SDK gives a handler its artifacts
  source with each invocation, bound to that invocation, and a request made
  before the stream opens waits for it.
- The service writes each request as one standard output record, JSON with
  `id`, its own number for the request, unique among its requests in flight,
  and one of:
  - `install`: `invocation`, the invocation it serves, which must be one the
    runner started in this service and that has not ended; `name`,
    `version`, `sha256`, `size`; `form`, `{ kind: "file" }` or
    `{ kind: "archive", entry }`; and, for software the program installs
    from its official source, `url`, an HTTP or HTTPS URL without
    credentials, which the runner downloads from instead of asking the
    backend; the size and SHA-256 the program pinned decide what it keeps,
    whatever the transport. Before the outcome, the runner answers
    `{id, progress}` as the download passes each tenth of the size, with
    `phase` `download`, `done` and `total` in bytes, and once more with
    `phase` `unpack` as an archive unpacks; a program that installs on
    purpose, such as `demi browser install`, prints them, and one that
    installs for its first use ignores them. The runner answers
    `{id, path}`, the absolute path of the file or of the archive's entry,
    once the artifact is installed.
  - `installed`: `name`. The runner answers `{id, installed}`: the artifacts
    of that line its cache and the image hold, each with its `version`,
    `sha256` and `path`, the cache's newest install first. It reads only the
    Host and asks the backend nothing.
- A request that fails is answered `{id, error}`, with the reason. The runner
  keeps at most 32 requests of a service in flight and refuses one beyond
  that at once. An install's request ends with its invocation: when the
  invocation ends, the runner cancels the download it waits for, unless
  another request needs the same artifact.
- The stream ends with the service; a request still waiting fails.

A program uses only the paths the runner answers: it never downloads or
looks for an artifact itself. It knows what it asks for, and from where,
from its own record, as `demi-browser` knows the pinned Chrome for Testing
and its official URL, or from the call's input, as `demi-claude-code`
receives the release the backend chose. A URL the program names cannot
change what runs either: the runner checks the download against the size
and SHA-256 the same request gives, as it does every download.

### Installed artifacts

The runner also reports what its cache holds, so a page can tell what a Host
has without asking it. For example, after `demi browser install` on a
laptop, the laptop's device lists `demi.browser: Chrome for Testing
153.0.8010.36`, and the conversation browser's panel offers new tabs on that
laptop ([Live browser view](../browser/live-view.md#a-browser-tab-in-the-panel)).

The runner sends `installed { artifacts }`, each with its package, name and
version, once when it connects and again whenever an install or a retirement
changes its cache ([The cache](#the-cache)); an executable is listed as the
others are. The backend keeps each device's last list with the device's
record, so it outlives the connection: a stopped Cloud keeps showing what
its cache held, since the cache lies on its home image. The product state
carries each device's list ([Page synchronization](../product/web-api.md#page-synchronization)).
A plugin reads it for its own package's artifacts; the runner and the
backend know no artifact's meaning.

### Preinstalled artifacts

A Cloud image holds the executable of each command package it was built
with ([Cloud images](../cloud/images.md#root-filesystem-contents)), so a new
or reset Cloud need not download them. The runner looks there before it asks
the backend: the directory `/opt/demi/artifacts/<sha256>`, named by the
executable's SHA-256, holds it as its one file, under the name its release
gives it. For example, the first `demi file patch` on a
Cloud after a reset finds `/opt/demi/artifacts/<sha256>/demi-file`, checks
it, and starts the service from there, without asking the backend for a
location or downloading anything.

The copy lies outside the runner's private cache, so the runner checks it as
it checks a download instead of trusting it as it trusts a cache entry:

- The directory must hold exactly one regular file, with the size and
  SHA-256 the runner was given. A request does not name the file, so the
  runner takes the directory's one file.
- The runner checks a copy once per runner process, the first time it needs
  it. A copy that matched is used in place, like a cache hit, and is not read
  again while the process runs.
- A copy that does not match is not used. The runner writes why to the
  [Host log](runner.md#host-log) and downloads the artifact into its cache;
  since the cache comes first, later uses take that download. The runner
  never depends on the copy: the backend locates every artifact it grants,
  so a damaged image costs a download, not a command.
- When the directory does not exist, the runner downloads the artifact and
  logs nothing. That is the case on every paired device and on a Cloud whose
  image holds other releases: the runner does not ask which kind of Host it
  runs on. A Windows runner does not look, since Cloud images are Linux only.

## Invoke and retire a service

The runner launches the verified executable with `--command-service`. The first
patch call checks the service before invoking `file.patch`. Later calls reuse
the process and connection, with one HTTP/2 stream per invocation.

The sequence below starts with an installed executable. Time runs downward;
arrows show interactions between the runner and service. Each invocation uses
its own stream on the reused connection.

```text
Native command execution: startup and successful calls

Runner                                  Command service
  |                                           |
  |--- Start process ------------------------>|
  |--- GET /v1/info ------------------------->|
  |<-- Protocol version + operations ---------|
  |                                           |
  |    Validate against pinned descriptor     |
  |                                           |
  |--- POST /v1/numbers (stays open) -------->|
  |                                           |
  |--- POST /v1/invoke ---------------------->|
  |                                           | Execute
  |<-- stdout / stderr records ---------------|
  |<-- completion record ---------------------|
  |                                           |
  |--- Next invocation ---------------------->|
  |                                           | Reuse process
  :                                           :
```

Calls share a resident service only when all three values match:

- Runner registration
- Execution user and security context
- Artifact digest

Concurrent startup requests within that scope share one launch. Different
immutable artifacts can run side by side. No service crosses a privilege boundary.
Reuse spreads process and connection startup costs across calls. It does not
establish a performance claim for individual commands.

The runner owns the child process and all three pipes. It establishes HTTP/2
directly over stdin/stdout, with the runner as client and service as server.
The transport needs no port listener, TLS, HTTP/1 upgrade, or gRPC.

Before admitting calls, the runner checks `GET /v1/info` against the pinned wire
version and operation set, then opens the service's
[numbers stream](#conversation-numbers). The runner rejects startup on timeout, missing
operations, extra operations, or version mismatch. The verified file establishes
executable identity, so the handshake does not require the service to report its
own hash.

Each invocation owns its cwd, environment, identity, input, output, and cancellation.
Handlers must not change process-global cwd or environment. Concurrent operations,
including blocking or CPU-bound work, must allow connection processing and
cancellation to continue. File algorithms belong in the command package, not in
the runner or shared SDK.

| Event | Required result |
| --- | --- |
| Normal completion | Deliver output and completion; release invocation resources. Keep the service available. |
| Invocation cancelled | Cancel its handler and release its resources. Preserve unrelated calls and the connection. |
| Handler exceeds cancellation grace | Report a service fault and retire the process. |
| Service exits or corrupts the protocol | Fail affected invocations with the service's exit status and the tail of its standard error, which the Host log also records. Do not automatically replay potentially completed side effects. |
| Execution context is disposed | Release its bindings and its lease on the service. |
| No lease remains and the service holds no conversation | Request shutdown, drain the service, close its transports, and reap the child. |

The runner allows **6 seconds** for the shutdown request and process exit.
If shutdown fails or exceeds that deadline, the runner terminates and reaps the
child. Startup failures also release the process and its transports.

Once the shutdown is answered, either side may close its transport while the
other still has closing HTTP/2 frames queued. For example, the service exits as
soon as its connection has drained, while the caller's connection may still be
sending its own GOAWAY; that write then fails with a broken pipe. On either
side, a write into the closed pipe or socket, a reset or an end of input after
an answered shutdown ends the connection and is not a failure.

EOF ends input, not execution. Cancellation requires the handler to stop and
release its resources; resetting a stream or aborting a task is insufficient.
The [protocol limits](#validation-and-flow-control) set the cancellation grace.

For example, two jobs using the same artifact can share one service. Cancelling
one call normally leaves the other running. If its handler refuses to stop within
the grace period, retiring the faulty service also fails the other affected call.
The runner must report that failure rather than claim isolated cancellation.

Changing the startup catalog creates new pinned bindings; it never swaps the
executable that serves an existing context.

### Keep a service resident

The runner's service registry decides when a service ends, and the connection's
message handling never waits for that decision. A service stays resident while
anything holds a lease on it:

- a live execution context whose pinned manifest names the service's artifact;
- an open [user stream](#user-streams) running in it;
- a start in progress, for a call waiting until the service is ready;
- the manifest installed on the connection, so consecutive jobs reuse the
  service instead of starting it again;
- the package release that the connection's user streams bound last, one per
  package, so consecutive user calls reuse the service as consecutive jobs do.
  It lasts until the connection ends; a stream that binds another release of
  the package moves the lease to that release.

For example, the page lists the conversation browser's tabs with a one-shot
`browser.tabs` call each time it is shown again. The first listing on a
connection starts the `demi.browser` service; the next ones find it running,
whether or not a job has run.

When the last lease ends, and after each conversation release ends, the
registry asks a service without leases for its
[conversation status](#conversation-scoped-state). A service that still holds a
conversation stays resident; one that holds none is shut down.

An answer counts only when no lease came or went and no release ended while it
was on its way; otherwise the registry asks again. The service answers from
what it holds at the moment it is asked, so an answer can still name a
conversation whose release ended before the answer arrived. For example, the
backend releases two conversations at once. The check that follows the first
release is answered while the second release is still running, and names the
second conversation. Trusted, that answer would keep a service that holds
nothing; instead the registry asks again once the second release has ended,
and stops the service.

A status check that fails, or does not answer within 5 seconds, keeps the service and writes
the failure to the [Host log](runner.md#host-log): a service that cannot say
what it holds is not a service that holds nothing, and shutting it down would
end every conversation it serves. For example, when the connection switches to
a new `demi.browser` release, the service that ran the earlier release loses its
last lease while one conversation's browser in it is still retiring; the
service stays, and so does every other conversation's browser.

A service is retired against its leases only as faulty: when it exits, breaks
the protocol, fails a conversation release, or keeps a handler past its
cancellation grace. Losing the backend connection shuts every service down
([Command lifetime](runner.md#command-lifetime)).

## Command context

A command often needs to know more than its arguments: which conversation it
serves, who started it, and how to present results to the user. For example,
`demi browser open` keeps its tabs in the invoking conversation's browser, and
that browser starts Chrome in the user's time zone. These facts form the
command context, one value of one type on the command-service wire, which the
backend, the runner, and every native service link:

| Field | Meaning |
| --- | --- |
| `conversation` | The conversation the work belongs to. Work that belongs to no conversation, such as installing the Claude Code CLI after an account is added, names the provider entry it serves instead. Either is a name of ASCII letters, digits, `-` and `_`, at most 64 characters, which the wire checks where it is decoded, as it checks the conversation a release names: the runner names a job's directory after it ([Pipes and output](runner.md#pipes-and-output)). |
| `caller` | Who started the work: `agent`, with the agent's `number` as the model knows it ([Identifiers the model sees](../agent/runtime.md#identifiers-the-model-sees)), or `user`, for a [user stream](#user-streams). |
| `locale` | The time zone, an IANA name, and the languages, BCP 47 tags in preference order, that the web app last reported for the conversation's user ([User preferences](../product/web-api.md#user-preferences)), or `UTC` and `en-US` until it reports them. |
| `colorScheme` | `light` or `dark`, the scheme the web app last reported for the user, `light` until it reports one; the conversation browser starts with it ([Native driver](../browser/browser.md#native-driver)). |

The backend is the context's only source, and nothing reads it from
environment variables: a script can change those, and every program the job
runs inherits them.

```text
Backend: builds the context and keeps it in the job's or stream's record
   | job_start { context }, service_open { context }
   v
Runner: keeps it in the job's live execution context
   |-- native invocation { context, edits, cwd, env, args } --> command service
   `-- rpc_call { jobId } --> backend: the handler receives the record's context
```

- The backend builds the context when it starts a job or opens a user stream,
  and keeps it in that job's or stream's record. A job that `demi host shell`
  starts on another Host carries its invoking job's context.
- The runner keeps the context in the job's live execution context and writes
  it into every native invocation record. The invocation's `edits`, `cwd` and
  `env` stay separate: they describe the runner's resources and the process,
  not the origin of the work.
- An `rpc` call names only its job. The backend gives the handler the context
  from its own record of that job
  ([Bind jobs to their caller](sessions-and-targets.md#bind-jobs-to-their-caller)).
- Only declared commands receive the context. Other programs the job runs,
  such as scripts, neither receive nor need it. The job environment carries
  only what a command alias needs to reach the runner: the local endpoint, the
  opaque context handle, `DEMI_HOME` and the aliases on `PATH`. The runner
  finds the job's context through the handle
  ([External command clients](commands.md#external-command-clients)).

A new context field changes the type and the backend's construction of the
context; its consumers read it where they already receive the context.

## Conversation-scoped state

A native operation can keep state that must survive the shell job that created
it. The conversation browser keeps its tabs between shell jobs and agent turns.
That state belongs to the conversation, and the command package keeps it itself,
keyed by the conversation identity the runner supplies; there is no separate
resource handle, acquisition, or grant.

This section is the port through which every such capability attaches, and it
is deliberately the only one. The runtime provides mechanism, identity and
release, and no policy: it does not know what a tool holds, why, or for how
long. The backend provides one policy, the conversation idle rule in
[Conversation idle and Host resource release](resource-lifecycle.md), and no
mechanism specific to any tool. A tool that needs more than this port is
evidence that the port is missing a generic capability, not a reason to give
that tool a path of its own through the runner or the backend.

Native handlers find the conversation in the invocation's
[command context](#command-context). A script cannot change the context, so it
cannot select another conversation's state.

Calls from every conversation on a device share one resident service per
artifact and security context. The service separates state by conversation. A
service that holds conversation state stays resident after its last lease ends,
and so does one that cannot say whether it holds any
([Keep a service resident](#keep-a-service-resident)).

The service exposes `POST /v1/conversation` with the invocation framing and
cancellation contract below. Its operation is `release` or `status`, and its
trusted `conversation` field names the conversation; the local command client
cannot invoke it. `status` returns the conversations the service holds, and
answers at once from what it holds, without waiting for a release or any other
work in progress. `release` ends everything the service holds for that
conversation, awaits the domain cleanup, and acknowledges; an unknown
conversation is harmless to release. The runner sends `release` when the
backend sends the generic `conversation_release` message for that conversation.
Service shutdown releases every held conversation, all of them at once rather
than one after another, so the whole release fits the shutdown deadline. Failed
cleanup retires the faulty service and reports it, rather than claiming a
successful release. Which events lead the backend to send a release is defined
in [Conversation idle and Host resource release](resource-lifecycle.md).

### Conversation numbers

Some state a service keeps is named with a number the model reads, and the
conversation must never give that number twice
([Identifiers the model sees](../agent/runtime.md#identifiers-the-model-sees)).
For example, the conversation browser names a tab `t7`. After the browser
restarts, the conversation is released or the Cloud stops, the next tab is
`t8`, never a second `t7`. The service cannot keep that count itself, since
its state ends with each of those events, so the backend keeps it: the
conversation's `tab` sequence
([Conversation state and transactions](../backend/storage.md#conversation-state-and-transactions)).
The service asks for numbers through the runner, which forwards each request
as it forwards an artifact's location request.

```text
Service                        Runner                          Backend
  |<-- POST /v1/numbers ----------|  opened after GET /v1/info     |
  |--- {id 3, c1, tab, count 4} ->|--- numbers_reserve ----------->|
  |--- input pull --------------->|                                | 9 to 12 reserved
  |<-- {id 3, first 9} -----------|<-- numbers_reserved -----------|
```

- Once it has checked a new service's catalog, and before it admits a call,
  the runner opens one `POST /v1/numbers` request with the metadata `{}`. The
  request stays open for the service's life. The SDK gives the handler its
  numbers source when the service starts, and a request made before the
  stream opens waits for it.
- The service writes each request as one standard output record, JSON with
  `id`, its own number for the request, unique among its requests in flight;
  `conversation`, a conversation name it received in a
  [command context](#command-context); `sequence`, which is `tab`, the only
  sequence a service draws from; and `count`, from 1 to 16. It reads the
  answers as input, one chunk per pull
  ([Request body and input demand](#request-body-and-input-demand)).
- The runner sends each request to the backend as `numbers_reserve` on its
  connection and gives each answer back as one input chunk, in the order the
  answers arrive: `{id, first}`, the first of `count` consecutive numbers that
  now belong to the service, or `{id, error}`. It keeps at most 32 requests of
  a service in flight and refuses one beyond that at once.
- The backend answers only for a conversation of the device's user that
  reaches the device, as its primary Host or an attached one
  ([Attached hosts](sessions-and-targets.md#attached-hosts)). It advances the
  sequence by `count` in its own transaction before it answers, so a number it
  gave out is never given again, whether or not the service uses it. A
  service may therefore reserve a few ahead: a number it never uses is a gap,
  never a repeat. At most 32 requests are answered at a time on a connection;
  one beyond that, or one that repeats an id still in flight, is refused.
- The stream ends with the service. Service shutdown ends it, and a request
  still waiting fails. A lost backend connection stops the runner's services
  in any case ([Command lifetime](runner.md#command-lifetime)).

### User streams

An operation can also serve the conversation's user directly, for as long as
the user's page stays open. For example, the
[live browser view](../browser/live-view.md) streams pictures of the
conversation's browser tabs to the work panel and the user's input back to
them. It is the same conversation state the agent's `demi browser` commands
use, reached through the same port.

Each user stream is declared by name with a native binding, beside the command
tree and the way a command leaf binds an operation, by the
[plugin](../architecture/plugins.md#calling-its-command-package) whose commands
bind the package: `plugin-browser`'s `browser` stream binds `demi.browser`
operation `browser.live`. The declarations are fixed with the command tree for
the backend's lifetime, and a page can open only a declared name.

When the user opens a stream, the backend asks the conversation's Host runner
for a [service stream](runner.md#service-streams). The runner invokes the
operation with `POST /v1/invoke`, the contract every command uses, in the
resident service that holds the conversation's state: the invocation uses the
same package binding as the conversation's jobs, so it reaches the same
service, and the open stream holds a lease on that service. The connection
keeps the service for its next streams
([Keep a service resident](#keep-a-service-resident)). Its
[command context](#command-context) names the conversation and a `user`
caller, with the user's locale. Its `cwd` is the conversation's directory and
its environment is empty.

The same mechanism serves a one-shot user call: the backend opens a service
stream on an operation that finishes by itself, passes the operation's `args`
as a command's invocation does, asks for its JSON result, sends no input, and
reads the output to its end. A call whose invocation exits nonzero fails with
what the operation wrote to its standard error, which a JSON invocation writes
as `{ error: { code, message, details } }`. The failure also retains the bounded
standard output, so a package whose contract puts its error document there can
validate and report it without losing its cause
([Service streams](runner.md#service-streams)). The
[conversation browser's tab methods](../browser/live-view.md#the-tab-methods), a
plugin's package calls, call `browser.tabs`, `browser.open` and `browser.close`
this way, so a tab the user
opens is the tab the agent's `open` would have made, created by `user`.

The invocation's input is the bytes the page sends and its output is the bytes
the page receives. The operation frames its own messages; the runner and the
backend forward bytes without reading them. The stream ends when either side
ends it: the runner cancels the invocation when a pipe fails, and the
invocation's completion ends the page's connection.

## Invocation protocol

Protocol version 1 uses direct HTTP/2 requests. The shared SDK owns process IO.
Handlers use logical output writers. Executable stdout contains only protocol
bytes. Process stderr carries service diagnostics, which the runner drains
independently into the [Host's log](runner.md#host-log) line by line. The runner
also keeps the last part of that output, so when a service exits or breaks the
protocol, every call that fails with it reports the service's exit status and
that tail.

| Request | Purpose |
| --- | --- |
| `GET /v1/info` | Return wire version and operation IDs before invocations. |
| `POST /v1/invoke` | Run one invocation on one stream with concurrent request/response bodies. |
| `POST /v1/conversation` | Release a conversation's state or report which conversations hold state ([Conversation-scoped state](#conversation-scoped-state)). |
| `POST /v1/numbers` | Carry the service's requests for conversation numbers and their answers for the service's life ([Conversation numbers](#conversation-numbers)). |
| `POST /v1/shutdown` | Stop admission and drain before process exit. |

### Request body and input demand

The invocation request begins with a four-byte big-endian JSON byte length,
followed by JSON containing `operation`, `invocationId`, the command's path,
such as `demi file read`, which a handler that takes several values puts
before each failure it reports
([Handle an rpc call](commands.md#handle-an-rpc-call)), parsed `args`, whether
the caller asked for `json`, `cwd`, `env`, the
[command context](#command-context), for a job that records edits, its
`edits` context, and, for an invocation a job's command makes, where its
stdout goes, `stdout`: `job` or `elsewhere`
([Where a command's stdout goes](runner.md#where-a-commands-stdout-goes)). Each subsequent stdin chunk has a four-byte big-endian length
and at most 64 KiB of binary payload.

The service sends response headers before waiting for input. Input then follows
this exchange:

1. The handler requests its next input chunk.
2. The SDK sends an input-pull record.
3. The caller sends exactly one chunk or signals EOF with request END_STREAM.

The caller preserves the chunk boundary across HTTP/2 DATA frames. A short
live-stdin read does not authorize another read. Receive-window capacity does
not authorize reading stdin either.

A handler may finish without reading all of its input, and a
[conversation request](#conversation-scoped-state) is answered from its
metadata alone. Once its response is complete, the service ends such a
request with `RST_STREAM(NO_ERROR)`, and whatever the caller still sends after
that, an input chunk or its EOF, is dropped, not an error. A conversation
request's metadata is its whole body, so the caller sends it together with
END_STREAM and has nothing left to send when the answer comes. Otherwise a
loaded caller that ends a release in a second step can find the request
already reset, and a release the service answered would look failed, which
retires the service ([Keep a service resident](#keep-a-service-resident)).

### Response records and completion

Responses contain records with a one-byte kind, four-byte big-endian payload
length, and payload. DATA frame boundaries are unrelated to record boundaries.

| Kind | Payload |
| --- | --- |
| 1: stdout | Raw bytes. |
| 2: stderr | Raw bytes. |
| 3: completion | JSON: `exitCode` from 0 to 255, optionally `error: { code, message }`. |
| 4: input pull | Empty; requests one input chunk or EOF. |
| 5: medium | JSON: `size`, at most 16 MiB. A [returned medium](commands.md#return-media) begins, and its bytes follow. |
| 6: medium bytes | Raw bytes of the medium that began last. |

A handler returns a medium through its output writer, which writes one
medium record and then the medium's bytes in medium-bytes records, with no
other record of the invocation between them, so a medium is never
interleaved with output or with another medium, and its place among the
stdout records is where the handler returned it. Bytes that do not add up to
the medium's size, medium bytes without a medium, and a size over 16 MiB
break the protocol. The service neither routes nor keeps media: the runner
does, by the invocation's `stdout`
([Return media](commands.md#return-media)). An invocation without `stdout`, a
[user stream](#user-streams) or a package call, cannot return media: its
writer refuses them, and the handler learns so from the write.

A normally handled invocation ends with exactly one completion record. Missing
completion is failure even after HTTP 200. Bytes after completion are invalid.

HTTP status reports rejection before execution: invalid metadata, an unknown
operation, or a service that is shutting down. It never reflects how many calls
are in flight. After execution starts, command failure uses completion.
Command output carries any partial-operation result.

### Validation and flow control

The SDK rejects unknown metadata fields and malformed or oversized records. The
wire's own messages, descriptors, and the command context are fixed contracts,
decoded into their types and validated where they enter
([Validation at entry](../architecture/contracts.md#validation-at-entry)).
Command arguments are different: their schemas come from the declarations at
runtime. The dispatcher validates them against the leaf's JSON Schema, and the
native handler decodes them into the operation's argument type and rejects
invalid values before work.

The SDK checks message sizes while decoding, before allocating unbounded memory.
These limits apply:

| Resource | Version 1 limit |
| --- | --- |
| Invocation metadata JSON | 256 KiB |
| Response record payload | 64 KiB |
| HTTP/2 header list | 16 KiB |
| Queued output records per invocation | 4 |
| A returned medium | 16 MiB |
| HTTP/2 handshake, service info, and invocation metadata timeout | 10 seconds per phase |
| Cooperative cancellation grace | 5 seconds |

These are fixed SDK limits. The number of invocations is not limited
([Load](runner.md#load)). Each stream has its own flow-control window and
output queue, and the connection's window is the largest HTTP/2 allows, so the
connection never becomes the constraint: a slow consumer holds back only its
own invocation, and memory grows with the invocations running. The service's
only peer is the runner that started it, so HTTP/2's guard against a flood of
cancelled requests is off; cancelling many just-sent invocations at once is
ordinary.

The SDK returns input receive capacity while assembling one requested, bounded
chunk. It reserves output capacity before sending DATA. Connection processing
continues while output is blocked so that resets and disconnects can be received.

`RST_STREAM(CANCEL)` triggers the invocation's cancellation token. The
[service lifetime rules](#invoke-and-retire-a-service) determine whether cleanup
succeeds or the process is faulty.

Local forwarding reuses framing, demand, and completion rules but has a distinct
raw CLI metadata type. Its authentication and disconnect behavior remain in
[the local client contract](commands.md#external-command-clients).

## Publish a complete release

A published version supplies the same operations on all supported targets.
Requiring a complete release lets jobs select their execution host without
encountering a platform-specific gap in that version.

Command packages and runner releases cover the six targets of
[Executables and targets](../delivery/builds-and-releases.md#executables-and-targets),
which the [release workflow](../delivery/builds-and-releases.md#release-workflow)
builds. Building for a target does not show that the
build runs there: release validation runs the same protocol and command
conformance cases on a native artifact of every target, never under emulation,
and the build guide's [Validation](../delivery/builds-and-releases.md#validation)
lists the other checks, among them that a Linux executable is self-contained.

Completeness is a rule of the published release, not of the descriptor or the
store. A descriptor and a runner manifest name the targets they carry. Release
packaging writes all six unless told otherwise, and the release workflow
publishes only that. A developer's release carries only the targets of the
Hosts in use: on an x86_64 Linux machine that also hosts its own Cloud, that
is `x86_64-unknown-linux-musl` alone. The backend publishes and serves a
release of any nonempty set of targets in the same way, whatever its object
store ([Backend deployment configuration](#backend-deployment-configuration)).
A Host whose target a release lacks fails the command with a catalog
mismatch, and its runner cannot be installed from that release; nothing falls
back to another target.

The [build guide](../delivery/builds-and-releases.md) defines commands and
toolchain setup. Managed guest images consume the Linux runner. Their image
lifecycle and execution-surface verification follow the
[managed host design](../cloud/managed-hosts.md#images).

### Publish packages, then source artifacts on demand

The backend's artifact module publishes the command packages into the
deployment's one object store, local or S3, the store that holds the users'
blobs ([The object store](../backend/storage.md#the-object-store)), and it
publishes in the same way into either. The store's library provides the
conditional writes and SHA-256 checksums publication needs, and, for S3, the
presigned URLs. Before the backend accepts requests, the module completes
these steps:

1. Validate every release's descriptor.
2. Publish each descriptor and its immutable package/version mapping.
3. Enable the catalog.

No artifact is stored at startup. An artifact enters the store the first
time something needs it: a runner asks where to download a package's
executable, or an installer asks for a runner executable
([Runner releases](#runner-releases)). The backend then takes it from the
first source that holds it:

1. The store itself: an artifact needed before is there already.
2. The release's files, where the server release's `release.json` says
   ([Backend deployment configuration](#backend-deployment-configuration)):
   the executable's file read from a directory on the backend's machine, as
   a developer's release keeps them, or downloaded over HTTPS, following
   redirects, as GitHub serves a published release's assets.

It checks the bytes, a package executable's compressed copy by what it
decodes to and anything else by itself, against the size and SHA-256 the
descriptor or the runner manifest gives, stores them, and answers the request.
Needs of one artifact that arrive together share one fetch. When no source
holds the artifact, or a download fails, the request fails with the reason,
and the next need tries again. For example, the first `demi file read` on a
Windows laptop paired with a server that runs a published release: the
runner asks where `demi-file` for `x86_64-pc-windows-msvc` downloads from; the
store does not hold it yet, so the backend downloads
`demi-file-x86_64-pc-windows-msvc.exe.zst` from the release's location, checks that it decodes to the executable the descriptor
names, stores it, and answers with its location; the laptop downloads it from
there, as every later laptop does.

Conditional writes reject conflicting content. Repeated startup reuses existing
objects. Interrupted publication can leave unreferenced blobs, but it cannot
expose a partial release or overwrite an existing version's meaning. Multipart
ETags must not be treated as SHA-256 checksums.

Under the store's `native/` keys
([The object store](../backend/storage.md#the-object-store)), an executable
is `native/blobs/<sha256>`, a descriptor's canonical JSON is
`native/descriptors/<digest>.json`, and the package/version mapping is
`native/packages/<id>/<version>.json`, with the version percent-encoded as a
URI component.

A package executable's object holds the executable compressed with zstd and
carries `Content-Encoding: zstd`; descriptors, mappings and runner
executables are stored as they are. HTTP's own content coding is the
mechanism: the store serves the stored bytes with the coding they were
stored with, and the runner's HTTP client decodes them
([Install artifacts](#install-artifacts)), so no descriptor, manifest or cache
entry knows about compression. zstd rather than gzip because it matters
here: the release runner compresses to about 28% of its size with zstd and to
41% with gzip. Packaging compresses each executable, at zstd's slowest
ordinary level, and the release's files hold the compressed copy
([Packaging](../delivery/builds-and-releases.md#packaging)); the backend
stores that copy once it decodes to the executable's size and SHA-256, so no
backend compresses anything.

Every object carries `sha256` and `size` metadata, which describe what the
object stands for: for a package executable, the SHA-256 and byte size of the
executable, not of the compressed bytes; for any other object, of its stored
bytes. An object already in place counts as the one being stored when both
values match, and as a conflict otherwise. The compressed bytes of one
executable may differ between zstd versions without changing what the object
is.

### Backend deployment configuration

The backend publishes every command package release in the `commands/`
directory of its [server release](../delivery/builds-and-releases.md#server-release),
one release per directory, named as its program, such as `demi-file`, which
holds the release's `descriptor.json`. The programs are not in the server
release: they are the release's files, one per program and target, at the
location its `release.json` names, an HTTPS URL or a directory on the
backend's machine:

```json
{ "files": "https://github.com/wspl/demi/releases/download/v0.1.3/" }
```

A file is named by the executable and the target, with `.exe` on Windows: a
command program's compressed copy as `demi-file-x86_64-unknown-linux-musl.zst`
or `demi-file-x86_64-pc-windows-msvc.exe.zst`, and a runner executable as it
is, `demi-runner-aarch64-apple-darwin`
([Server release](../delivery/builds-and-releases.md#server-release)). An empty
`commands/` means no command packages: the backend publishes nothing and
starts with an empty catalog, so conversations offer no `demi file` or
`demi browser` commands. A root without `commands/` or without
`release.json` is an error, since only the empty directory means none.

The deployment's object store says where runners download the artifacts
from ([The object store](../backend/storage.md#the-object-store)). A runner
receives the location when it asks for an artifact, once the backend has
sourced it
([Where an artifact comes from](#where-an-artifact-comes-from)):

- With an S3 store, a GET URL signed for five minutes.
- With a local store, `GET /native-artifacts/<sha256>` on the backend's
  public URL. For example, a backend at `https://demi.example.com` answers a
  Cloud guest's request for the `x86_64-unknown-linux-musl` executable of
  `demi.file` with `https://demi.example.com/native-artifacts/<sha256>`. The
  backend serves the object from its data directory with the content coding
  it was stored with, without credentials and without expiry, like the runner
  installers' downloads; any digest the store does not hold answers 404
  `not_found`.

Downloads from a local store need no credentials because they reveal nothing
a user owns and cannot change what runs. The objects are the released
programs, which the release publishes anyway, and each URL names a digest,
which the runner checks the download against; the digest reached it over its
authenticated connection to the backend.

The backend builds its package catalog and artifact resolver from
`commands/` at startup. A job that `demi host shell` starts on another Host
receives the calling session's catalog. The backend's scenarios load the
programs the workspace built as a release of their Host's target, which the
scenario backend publishes into the local store of its temporary data
directory; tests without a backend resolve an artifact to a local file
instead.

A command program a developer rebuilds without a new workspace version is
still a release of its own: packaging outside the release workflow names it
with a version that carries a digest of its artifacts
([Rust executables](../delivery/package-versioning.md#rust-executables)), so
a backend whose data directory outlives the rebuild publishes it beside the
earlier one instead of conflicting with it.

### Runner releases

The server release's `runners/` holds the runner release the installers
install: `manifest.json`, which names the current release, and the release's
directory with its own `manifest.json`, which lists each target's executable
by size and SHA-256
([Packaging](../delivery/builds-and-releases.md#packaging)). The executables
themselves are the release's files, published compressed with zstd as
`<file>.zst`, as a package's artifacts are, and sourced like a package's:
compressed, a runner executable is about a quarter of its size, 10.6 MB
instead of 38.8 MB for macOS's. The size and SHA-256 are the decompressed
executable's.

`GET /runner-artifacts/<release>/<target>/<file>.zst` answers the compressed
file from the object store, sourcing it first when the store lacks it; a
paired device's runner downloads its updates from it and decompresses them
as it does any artifact ([Runner updates](runner.md#runner-updates)).
`GET /runner-artifacts/<release>/<target>/<file>` answers the same file
decompressed as it streams, for the installers, which know only the backend
and have no zstd. Both stream from S3 as from the data directory.

A backend sources the runner executables of its release for the systems of
the paired devices it serves, both architectures of each, as soon as it
starts, in the background, so an
update never waits on the release's origin: a runner that reconnects after
the server's upgrade finds its executable already in the store.

## Acceptance

The [build guide](../delivery/builds-and-releases.md#validation) identifies
release checks. Acceptance of this design requires the same observable outcomes
for successful calls, blocked IO, malformed input, cancellation, and service
failure on all six targets, as well as verified publication and installation.
Synthetic transport benchmarks do not establish file-command performance.

These outcomes are observed on a paired device and on the Cloud:

| Situation | Required result |
| --- | --- |
| A service with no lease left answers its status check slowly or not at all | The service stays resident with every conversation it holds, and the connection keeps answering other requests meanwhile |
| A handler returns a medium whose bytes do not add up to its size, or one over 16 MiB | The call fails as a protocol break does ([Invoke and retire a service](#invoke-and-retire-a-service)); the runner keeps nothing of the medium |
| A handler of a user stream or a package call returns a medium | Its writer refuses it, and the handler goes on |
| A resident service exits while calls are running | Each call fails with the service's exit status and the tail of its standard error; the Host log shows the same |
| A service holding several conversations' browsers shuts down | Every conversation is released within the shutdown deadline |
| A request for an artifact location names a job that has exited | The request is refused or gets no answer; no location is sent |
| A Cloud's first native command after a wake or a reset, with an image that embeds the selected release | The service starts from the image's executable: the runner requests no artifact location and downloads nothing |
