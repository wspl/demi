# Runner connections and shell jobs

`demi-runner` turns a device into an execution target. It connects to the backend,
performs Host filesystem and process operations, and owns shell jobs. The backend
retains agent sessions, provider selection, and conversation history.

This document owns registration and job lifetime. [Commands](commands.md) defines
declared command dispatch, and [native execution](native-runtime.md) defines
artifact installation and resident command services.

The runner keeps control work apart from the work it controls. Its control
thread reads the backend connection, routes each message to the work it
belongs to, and drives the byte paths of pipes and streams, which only wait on
the network; it never blocks. File and git requests, hashing, and other
blocking work run on blocking threads a few at a time ([Load](#load)), and
shell jobs run on a shell runtime of their own ([Shell jobs](#shell-jobs)). A
burst of requests, a slow disk, or a busy job therefore never stops the runner
from reading its connection. [Concurrency](../architecture/concurrency.md#runner)
gives the threads and the owner of each piece of state.

## Connection and identity

Each backend registration has its own credentials, artifact cache, local
endpoint, and selected runner release; `DEMI_ARTIFACTS` can name a cache that
registrations share
([Install artifacts](native-runtime.md#install-artifacts)). The runner keys installation state by normalized backend
URL and holds an OS lock while that installation is active. Separate
registrations do not share their authorization context.

A paired device stores its device token in private installation state. A managed
guest receives a token at boot and keeps temporary state. The backend owns device
claiming and user ownership. A paired device reaches the backend at its URL; a
Cloud's runner reaches it through the backend's runner socket on the server
([Backend socket](../cloud/managed-hosts.md#backend-socket)), with the same
routes. The runner opens an outbound WebSocket and sends its
`hello` first; the backend closes a connection that has sent nothing within 30
seconds of opening. Besides its token, version and platform, the hello names
the Host's operating system with its release, such as `Ubuntu 26.04` or
`macOS 26.5`, on Linux its distribution's os-release `ID`, such as `ubuntu`
(the logo of the device's mark, [Conversations and
projects](../product/conversations.md#conversations-and-projects)), its
architecture, such as `x86_64`, and, read once at the
runner's start without a shell, its kernel's name and release (`uname -s`,
`uname -r`), `autostartUnavailable`, why the device will not start the
runner by itself after a restart, an object tagged by its `cause`
(`noSystemd`, `noUserManager`, `notWritten`, `lingerOff`, `refused`,
`unusableName`, `unknownAccount`) with the path, the user or the system's
error the cause names, null when it will
([Starting with the computer](#starting-with-the-computer)), and the
`SHELL` of its environment, which the Claude Code
provider's session snapshot takes ([The session a process
resumes](../providers/claude-code.md#the-session-a-process-resumes)). The backend keeps them
and the runner's version on the device record, which Settings shows, and the
agent reads the system and architecture in its context block
([Switch the primary target](sessions-and-targets.md#switch-the-primary-target)). The backend looks a hello's token up while it watches the
connection, and lets a runner that goes away meanwhile go without adopting it.
A hello for a device that already has a connection is the same runner
coming back over a new one more often than a second runner: a network that
drops a connection without closing it leaves the backend holding a socket
nobody answers. So the backend pings the held connection and waits up to 5
seconds: one that answers keeps the device, and the new hello is refused
with `already_connected`, as a second runner sharing a token is; one that
does not is closed, and the new hello is answered. A connection whose
liveness checks are paused, as a Cloud's are while it is checkpointed, counts
as answering. For example, a laptop
whose Wi-Fi blinked reconnects within about 5 seconds instead of being
refused until the backend's heartbeat gives the old socket up.

The runner retries a lost connection after 250 milliseconds, doubling the
wait after each attempt that fails, up to 10 seconds. A connection that went
online starts that over when it ends, however it ends, so a Host that was
online for hours reconnects at once after a drop rather than after the wait
an earlier outage grew to. The Host log records each attempt's failure once
while the same failure repeats, and records again a failure that follows a
connection that went online, so each drop leaves its reason in the log
before the next *online* line ([Host log](#host-log)).

A connection's end ends everything it carries. Requests waiting for the
runner's answer fail as the device being offline, its service streams and
direct streams close, and its `rpc` calls are stopped, so their handlers see
the cancellation ([Handle an rpc call](commands.md#handle-an-rpc-call)). The
backend's work for the connection, such as an `rpc` handler committing its
change or the answer to one of the runner's requests, gets up to 5 seconds
to finish; whatever still runs then is aborted, and the backend logs each
piece by name. Only then is the device offline, and a new hello for the
device waits for that end, so it is answered within those 5 seconds. For
example, a `demi browser` call whose handler waits for the very device its
connection served would otherwise keep that connection from ending, and
every new connection of the device would wait behind it, unanswered, until
the backend restarted.
It answers a known token with `hello_ok` once it has bound the connection to
the token's device, which is online from then until the connection ends. A
hello that meets the backend's shutdown gets no answer: its connection closes,
as every runner's does at
[shutdown](../backend/backend.md#startup-and-shutdown). Both ends decode every
MessagePack message into the types of the runner wire's contract crate, which
they both link, and validate it at entry; a message that fails closes the
connection, since its sender broke the protocol
([Validation at entry](../architecture/contracts.md#validation-at-entry)).
The side that closes it names the message's type and the decoding error in
the close frame's reason, and the other side reports that reason as the
connection's end. For example, a test's stale runner that cannot read a
command declaration's newer field ends the connection with that field's
name, and the command lost with the connection says it, rather than only
`runner disconnected`.
Integer fields travel as MessagePack integers, and byte fields as MessagePack
binary.

The authenticated local management endpoint exposes status, drain, and a
request to write the runner's current pairing state to its log again, which
the installers use on a runner they find running ([Installation, pairing and removal](#installation-pairing-and-removal)). Draining
stops admission, waits for active work, and releases the installation lock so
the next runner, such as a newer release, can start. A runner that drains or
stops ends its backend connection in order: it sends the messages it has
queued, then a WebSocket close frame (going away), and waits up to five
seconds for the backend's close frame, so the backend sees the runner leave
rather than lose it.
[External command clients](commands.md#external-command-clients) defines local
endpoint access and installer verification.

A message on the connection is at most 4 MiB, the limit the runner wire's
contract crate defines for both ends. The side about to send a larger one fails
the one request that message belongs to with `too_large` and keeps the
connection; a directory with tens of thousands of entries, for example, cannot
be listed. A receiver still closes a connection that delivers a larger message,
since its peer broke the protocol. Bulk bytes never need a large message: file
contents and command IO travel through [pipes](#pipes-and-output).

The limit is eight times the largest regular message, a 5,000-file working
tree list or a shell command as long as a model can write, about 0.5 MiB
each. One connection carries everything, so a message in flight holds up the
rest; at 4 MiB a message still leaves within the 30-second write deadline on
an uplink of a little over 1 Mbit/s, and a full inbound queue of eight
messages stays within 32 MiB.

The runner reads messages into that queue of eight. Handling a message only
routes it to the job, request, or stream it belongs to, which runs apart from
the connection, so the queue drains without waiting for a disk, a service, or a
job. When the queue is full anyway, the runner stops reading until a message
leaves it, and the backend's writes wait meanwhile. A full queue never closes
the connection; only a message that breaks the protocol, by its size or its
content, does. In the other direction the backend queues at most 64 messages
for a runner, and a sender waits for room instead of buffering without limit.

### Runner updates

A runner always runs the runner release of the backend it serves, and
follows it when the backend moves to another release
([Upgrades](../delivery/upgrades.md)). For example, a server is upgraded
from release `a1b2…` to `c3d4…`. While `a1b2…` still serves, the laptop's
runner is told that `c3d4…` is coming and downloads its executable in the
background, its device online throughout and the page showing *Downloading
update · 40%*. The server switches; the runner reconnects, is answered with
409 and `c3d4…`, finds the executable already in place, starts it, and the
laptop is back online seconds after the backend, and nobody ran the
installer. A device on a slow link no longer looks dead: before, a server
in Hong Kong reached the backend at 7 KB/s, downloaded its update after the
switch, showed offline for seven minutes, and its first download broke off
after six of them while a new connection took 12 seconds.

The runner opens its socket at `/api/runner` with three headers:
`Demi-Runner-Release`, the release it was installed from,
`Demi-Runner-Target`, its target, and `Demi-Runner-Token`, its device token,
by which the backend knows which device a 409 sends to an update. Before the socket opens, the backend
compares the release with its current runner release, the one `runners/`
names ([Runner releases](native-runtime.md#runner-releases)). The same
release opens the socket, and the hello follows as above. Another release
gets 409 and a JSON body that names the backend's release and, when that
release has one for the runner's target, the executable's size and SHA-256:

```json
{ "release": "<release>", "executable": { "sha256": "<SHA-256 in hex>", "size": 9461832 } }
```

A backend without runner releases, as in development, checks no release;
the hello's protocol check still refuses a runner of another wire version.

The check is made before the socket opens because the runner wire changes
from release to release: a runner of an earlier release could not read a
refusal written in a later one. The two headers and the 409 body are the
one part of the connection that every release keeps
([What crosses releases](../delivery/upgrades.md#what-crosses-releases)):
fields may be added to them, never changed or removed. `runner-protocol`'s
`release` module defines them.

A paired device's runner that receives an executable updates itself:

1. It takes the installation's install lock, the one the installers take,
   so that no installer runs meanwhile.
2. It downloads the executable, unless it downloaded it ahead (below),
   from `/runner-artifacts/<release>/<target>/<file>` on its backend, checks
   its size and SHA-256 against the answer, and puts it in place in
   `releases/<release>/` through a staging directory, as the installers do.
3. It records the new release in its installation state, `release-id`,
   which the installation's launcher reads, and removes every release
   directory of its installation except the new one and its own.
4. It ends what it still runs, as a drain does: its local endpoint, its
   command services, and the jobs it kept through the connection loss,
   stopped as a stop does, since the new runner cannot take them over; the
   backend learns of them from the new runner's hello as lost to the update.
   Then it lets its locks go.
5. It starts the new executable with its own arguments and environment but
   the new release: on Linux and macOS in its own process, on Windows beside
   it, and then it exits. The new runner connects.

**Downloading ahead.** While the old release still serves, an upgrade
tells its backend which runner release comes next
([Prepare](../delivery/upgrades.md#prepare)), and the backend sends each
connected paired device's runner `prefetch`, with the release and its
executable for the runner's target as the 409 names them. The runner
downloads it into `releases/<release>/` as step 2 does, under the install
lock, while it goes on serving; a runner that already holds that release
answers at once. It reports `prefetched` with the release, or `prefetch
failed` with the reason, which the Host log also keeps; a failure is
harmless, since the update after the switch downloads what is missing. The
backend serves the next release's executables from the directory the
upgrade names, and keeps no state of a prefetch past its connection.

**One download, resumed.** Every download of a runner executable, ahead or
in an update, keeps its compressed bytes in a staging file named by the
release, `releases/.download-<release>/`, and continues a broken one with an
HTTP Range request from the bytes it has, instead of starting again; it
decompresses and checks the whole once it is complete, and a staging file
that fails the check is removed. A download that receives nothing for 30
seconds gives that connection up and opens a new one at once, resumed, since
a new connection often takes a better path than one that stalled. The
artifact route answers Range requests on the compressed file
([Runner releases](native-runtime.md#runner-releases)). An executable is
about 10 MB compressed.

**What the page shows.** The backend counts the bytes it serves of a runner
executable to the device whose token the request carries, and the device's
state carries the download's progress, announced as it moves on the page's
synchronization channel at most once a second:

| While | The device shows |
| --- | --- |
| A prefetch downloads, the runner online | Online, with *Downloading update · 40%* |
| After the switch, the runner downloading or restarting | *Updating*, with its percent while a download moves |
| No byte served to it for 2 minutes since the 409, and no hello | Offline |

A device is updating from the 409 that named the executable until its next
hello, for as long as its download moves, so a slow link that keeps
receiving shows *Updating · 26%*, never offline. A runner of a release
before 0.1.14 sends no token, so its device shows as offline during that
one update.

An update that fails, such as a download that breaks off or an executable
whose SHA-256 differs, leaves the runner on its release. It writes the
reason to the Host log and tries again at its next connection attempt; the
device shows as offline meanwhile. A 409 without an executable means the
backend has no runner for the device's target: the runner writes that to the
Host log and keeps trying, since a later release may have one.

A managed guest's runner never updates itself: it comes from the image the
machine manager is configured with, which is built from the same release as
the backend ([Demi's programs in a Cloud](../cloud/managed-hosts.md#demis-programs-in-a-cloud)).
A 409 for it means the backend and the manager run different releases; the
runner exits with an error that names both, and the Cloud's boot fails.

## Installation, pairing and removal

A person adds their laptop from Add Device: they paste the install command
into a terminal, and the terminal shows a pairing code, such as `7KQ2-M9XD`.
They enter it in the dialog, and the terminal answers that the laptop is
paired as `Alexs-MacBook-Pro.local`, with the command that removes the runner
again. Then the installer exits, and the runner keeps running in the
background.

The installers the backend serves, `install.sh` for Linux and macOS and
`install.ps1` for Windows, put the runner of the backend's runner release
into the installation's directory, `~/.demi/instances/<backend>/` unless
`DEMI_HOME` names another, and start it in the background. They then stay
in the foreground until pairing ends:

- While the runner waits to be paired, the installer shows its pairing code,
  and a new code when the runner receives one, since a code expires. The
  runner writes each code only to its own output, its log, which the
  installation's directory holds private to its owner; the Host log never
  holds one ([Host log](#host-log)).
- Once the runner is paired, or when it was paired already, the installer
  shows the device's name and the removal command, `<installation>/run
  uninstall` (on Windows, `& '<installation>\run.ps1' uninstall`), and exits.

The runner writes the same lines to its log, so a person who closed the
terminal early finds them there; closing it does not stop the runner. The
runner writes its log itself when it is started with `run --log`, as the
installers and `run start` start it, rather than the installer redirecting
its output: it appends to `runner.log`, and when the file passes 10 MiB it
renames it `runner.log.1`, replacing the one before, and starts a new one.
So what a runner printed before it crashed is still there after `run start`
starts it again.

A runner started with `run --log` also leaves the terminal it was started
from, first of all: on Linux and macOS it makes a session of its own, which
has no controlling terminal, so pressing Ctrl-C at the installer, or closing
its terminal or SSH connection, ends the installer and never the runner or
its jobs. A process that leads its process group cannot make a session,
such as a runner a person starts in a terminal's foreground, or one that
replaced itself with its update. Such a runner asks the system whether it
still has a terminal: opening `/dev/tty` fails with `ENXIO` only when it has
none. When it has one, it starts itself again with the same arguments in a
session of its own, made between the new process's fork and its exec, says
so on its console, and exits with 0; nothing of the installation is open
yet at that point, so the new runner takes the lock and the log as the first
would have. When the question gets no answer, such as with no `/dev/tty` at
all, the runner refuses to start and says why. Only a runner that has no
terminal makes the signals its starter ignored on its behalf, `HUP` under
`nohup` and `INT` and `QUIT` for a background command of a shell without job
control, take their default action again, so the runner's jobs start with
the signal actions a person's own programs have.

The installer leaves this to the runner and never turns on the shell's job
control (`set -m`): with `curl … | sh` at a terminal, a shell turning job
control off again hands the terminal back to curl's process group, which
has ended by then, and dash fails there with exit code 2 after the runner
has started. `run start` starts the runner in a session of its own,
made between the runner's fork and its exec, so Ctrl-C while `run start`
waits never reaches the runner, not even before the runner's own first
step.

The installer and `run start` read the pairing codes and the connection's
outcome from where the log ended when they began waiting, whether they
started the runner or found it running, and follow the log across a
rotation. A runner they find running wrote its state before they
began, so they first ask it, through its management endpoint, to write its
current state to its log again: its current pairing code while it waits to
be paired, the device's name and the removal command once it is paired, or
why it cannot connect. The lines come from the runner alone, so the
installers and `run start` never compose them, and a pairing code still
reaches no channel but the log.

`run start` starts the installation's runner in the background again, as the
installer does, and returns once it is connected or has said why it cannot
connect; a runner already running is left as it is. It is how a person
starts a runner that a restart of the computer or a kill stopped, and the
web app shows it in the help of a paired device that is offline, ready to
copy, as `<installation>/run start`, with the installation directory the
runner reported in its last hello since the backend started, or the default
one before that, and the home directory written as `~`, or as
`$env:USERPROFILE` on Windows, where `powershell -File` does not expand `~` (on Windows,
`powershell -ExecutionPolicy Bypass -File '<installation>\run.ps1' start`, so
the default execution policy does not refuse it).

### Starting with the computer

A paired device comes back by itself: Alex restarts his MacBook for a system
update, logs in, and the laptop is online in Demi again before he opens a
browser, as Dropbox or Tailscale would be; a runner that crashes is started
again by itself. Nobody runs `run start` for either.

The runner keeps itself registered with the computer's service manager for
its user, the one each system offers without administrator rights:

| System | Registration | Starts | Started again |
| --- | --- | --- | --- |
| macOS | A LaunchAgent, `~/Library/LaunchAgents/sh.demi.runner.<id>.plist`, in the user's `gui` domain | At the user's login | After a crash or a non-zero exit, `KeepAlive` with `SuccessfulExit` false: at once, and 10 s apart when it ends again within 10 s of starting |
| Linux | A systemd user unit, `~/.config/systemd/user/demi-runner-<id>.service`, `WantedBy=default.target`, with lingering turned on for the user (`loginctl enable-linger`) | At boot with lingering; at the user's first login without it | `Restart=on-failure`, `RestartSec=10` |
| Windows | A scheduled task with a logon trigger for the user, named by the user's SID, run with the user's limited rights and hidden; the `HKCU\…\Run` value when the system refuses the task | At the user's logon | The task's restart on failure, after a minute, the shortest Task Scheduler takes; not with the `Run` value |

A service manager gives the runner its own small environment, not a
terminal's: on macOS `PATH` is `/usr/bin:/bin:/usr/sbin:/sbin` and `SHELL`
and `LANG` are unset, and the user's `PATH`, Homebrew's, nvm's, bun's, lives
in zsh's files, which a job's login shell, reading the bash ones, never
reads. So the device environment comes from the user's login shell, not
from whoever started the runner ([Host operations](#host-operations)), the
same after a restart as from a terminal. On macOS `LANG`, which Terminal
sets and no profile does, the runner sets itself when the shell leaves it
unset: the user's language and region with UTF-8, `zh_CN.UTF-8` for Chinese
(China); when the system has no such locale, such as `en_CN`, the first it
has of the same language, else `en_US.UTF-8`; without a UTF-8 `LANG`, `ls`
and `git` print a non-ASCII file name as question marks.

`<id>` is the installation's directory name, so runners of several backends
on one computer are registered apart; Windows keeps every user's tasks in
one folder, so the task's name also carries the user's SID. Each registration runs the
installation's launcher, `<installation>/run run --log`, never a release's
binary, so it outlives every update; the runner writes its own log, and the
service manager's output goes nowhere. On macOS the agent's process type is
`Interactive`, which the system does not throttle.

- **The runner keeps it.** Every runner, at each start, makes sure its
  registration exists and names this installation, and writes it when it
  does not; so an installation from before this rule gains it at its next
  update, and a person who deleted the file has it back at the next start.
  Writing it may start a second runner at once, which finds the lock held
  and exits as below. The installer starts the runner as before and leaves
  the registration to it.
- **Exit status decides a restart.** A runner exits 0 when nothing would
  come of starting it again: stopped, drained, removed, its device no longer
  paired (it logs the removal command, as now), another runner of the
  installation holding the lock, or a state no start can change, such as a
  saved configuration or token it cannot read or a backend that is not the
  one it was paired with, each logged. It exits non-zero only on a failure
  a new start may cure. So a revoked device or a second copy never loops.
- **The manager's runner is the one that runs.** A runner the manager did
  not start, the installer's or an update's successor on Windows, hands
  over once its registration works: it releases the lock, asks the manager
  to start the installation's runner, and exits 0; the manager names its
  own with `DEMI_SUPERVISED` in the registration's environment, so its
  runner never hands over. So a runner is supervised from the first minute
  after the install and after every update, and a crash before the next
  login is recovered too. Where the manager refuses, the runner goes on
  unsupervised and says so as below.
- **`run start` asks the service manager** when the registration exists
  (`launchctl kickstart`, `systemctl --user start`, the task's start), so
  the runner it starts is the manager's, and spawns the runner itself only
  where there is none.
- **Removal removes it.** `run uninstall` and a revocation the runner
  receives remove the registration as the runner's last step, after the
  directory, so the service manager never starts a runner that is gone.
- **Where none can be made**, such as Linux without a systemd user manager
  (most containers, WSL without systemd) or a Windows that refuses both, the
  runner runs as before and says so in its log and its hello: *This device
  will not start Demi by itself after a restart: <reason>.* Lingering the
  system refuses to turn on is said the same way, with the command that
  turns it on, `sudo loginctl enable-linger <user>`, since without it the
  runner starts only at a login. The installer shows the line, and Devices
  shows it on the device, so the user learns it before a restart, not
  after.

On macOS a runner the LaunchAgent starts answers for its own access to the
user's protected folders, Desktop, Documents and Downloads, and to what
the user allowed only Terminal; a runner started from a terminal answers
through the terminal. macOS remembers such a grant for the program's
designated requirement, so the release workflow signs the runner with
Demi's own code-signing certificate, the same in every release, with the
identifier `sh.demi.runner`; an ad hoc signature names the build's hash,
and the system would ask again after every update. The certificate is
self-signed: it is not for Gatekeeper, which never checks the runner, since
the installer and the update download it with curl, which marks no file as
quarantined.

`run uninstall` removes the runner of one backend from the device:

1. It asks the backend, when it can reach it, to revoke the device, so the
   device leaves the user's list as it leaves the computer; nothing refuses
   a revocation, and it names the projects that went with the device, whose
   files stay ([Web API](../product/web-api.md)). A backend it cannot reach keeps listing the device, offline,
   until the user revokes it.
2. It drains the runner: no new work, and the running jobs end first.
3. It removes the installation's directory, only when the directory holds an
   installation, the state files an installer or a runner writes there, and
   is neither the user's home directory, nor a directory that contains it,
   nor the root of a filesystem: a runner that ran with a mistaken
   `DEMI_HOME`, such as the home directory, has written its state there, and
   still never removes it. In such a directory `uninstall` removes only the
   files the runner wrote, its state files, `releases/`, `log/` and its own
   artifact cache, and says what it left. A directory that holds no
   installation is never touched, and `uninstall` says why. It removes the
   directory with the device token, the
   releases, the log and the artifact cache when the cache is the
   installation's own; a cache that `DEMI_ARTIFACTS` names, which runners of
   several backends may share, stays. Other backends' installations on the
   device are not touched. On Windows, where a running program's file cannot
   be deleted, a process the runner starts as it exits removes the directory.

An agent pairs a computer it reaches, such as one it can `ssh` to, without
the user typing the code. `demi host install-command` prints the install
command, the one Add Device shows, for each system or for the one
`--system` names; the agent runs it on the computer, reads the pairing code
the installer shows, and passes it to `demi host pair <code>`. The backend
claims the code for the conversation's user as Add Device does, and attaches
the new device to the conversation, so `demi host shell --host` reaches it
at once ([Conversation commands](sessions-and-targets.md#attached-hosts)):

```
agent                          the computer                   backend
  │ ssh … '<install command>' ──▶ runner starts, waits
  │ ◀── "Enter this pairing code in Add Device: WCXP-…"
  │ demi host pair WCXP-… ─────────────────────────────────────▶ claim
  │                               runner gets its device token ◀┘
  │ ◀── "Paired as DESKTOP-PLU; attached as DESKTOP-PLU"
```

The code is proof enough: it reaches only the runner's output and its log,
which only an account that runs commands on that computer reads, so an
agent holding it already controls the computer. A code that expired or was
never issued is refused with a sentence that says to read the newest code
the installer shows, since the runner receives a new one when a code
expires.

The backend composes the install commands from its public URL, as it
composes a device's `startCommand`; the Add Device dialog and `demi host
install-command` show the same text
([Web API](../product/web-api-hosts.md#workspaces-devices-and-attached-hosts)).

Revoking a device in Settings removes its runner too. The backend sends the
connected runner `revoked` before it closes the connection, and the runner
removes itself as `run uninstall` does, without asking the backend again; its
jobs end with the connection, as revocation ends them. A runner that was
offline learns of the revocation only as a refusal of its device token at its
next connection, which a backend that lost its data would also give, so it
does not remove itself then: it stops, and writes to its log that the device
is no longer paired with the backend, with the removal command.

## Host operations

Filesystem and raw process requests do not require a shell job. The runner
performs them on the target device as the account it runs under, whose own
permissions decide which paths they reach. A Host's default working directory
is where work starts when a request names none; it is not a sandbox, a
workspace boundary, or a permission check. A process the backend starts outside
a shell job, such as a provider's CLI, is a raw process request. Signals travel
by name from one closed set that raw processes and jobs share. When the backend
drops the handle of a raw process that has not ended, it asks the runner to kill
the process and does not wait: nothing controls such a process any more.

Every request crosses the Host's connection, which may take hundreds of
milliseconds each way: a laptop behind a proxy on another continent from its
server takes about half a second for a round trip. So a filesystem operation
that the backend needs as one answer is one request, never one per entry. A
directory listing carries each entry's name, kind, size and modification
time, and for a symbolic link its target as written and what the target is
(a file, a directory, another kind, missing when nothing is there, or
unreadable with the system's reason when it is there but cannot be reached,
followed through every link on the way), which the runner reads beside the
entry as it lists the directory: a
home directory of 105 entries, listed with one request per entry, took about
fifty seconds to show in the folder picker, and takes one round trip so.

The filesystem requests have the shapes that keep it so:

- **Look at several paths.** One request names several paths and answers
  each: whether it exists, its kind, a directory's entries with their
  metadata, a file's bytes up to a bound the request gives, with its size, or
  the error that kept the runner from reading it. A plugin's reads of a
  conversation's files are this request
  ([Reading a conversation's files](../architecture/plugins.md#reading-a-conversations-files)),
  and so is any check of several paths, such as a plugin's directories on a
  Host.
- **Read several files.** One request names the files and one pipe carries
  their contents, each after its length, in the request's order: a command's
  edit copies ([Edit copies](edit-tracking.md#edit-copies)) and a command's
  media.
- **Read with the file's metadata.** A read answers the file's size,
  modification time and version with its first bytes, so no `stat` precedes
  it. A read that refuses a file over a size reads one byte more than the
  bound and refuses when that byte arrives.
- **Write a directory.** One request carries a directory's listing, each
  file's path and mode, and one pipe its contents; the runner writes it into a
  temporary directory beside the final one, makes it read-only as the request
  says and renames it into place. Removing directories is one request too
  ([Host directories](../architecture/plugins.md#host-directories)).
- **Write without replacing.** A write says what to do when its path exists:
  replace the file, refuse with `file_exists`, or take the first free name of
  the form `name-2.ext`, `name-3.ext`, and answer the name it took; taking a
  name stops early at a name that already holds exactly the same bytes, and
  answers it without writing, so the same file sent again is one file. A write
  makes the missing parent directories of its path. No check precedes a
  write.

Requests that do not depend on each other go out together, not one after
another: the two sides of a changed file, the reads a command's completion
needs. Their pipes share one connection: the runner's HTTP client speaks
HTTP/2 where the backend's edge offers it, so a concurrent pipe costs no new
connection, which through a proxy and a TLS edge costs two or three round
trips of its own.

A pipe that carries bytes from the runner to the backend is one of two kinds,
which the request that names it decides:

- A **content pipe** carries one finite thing: a file, kept output, a
  command's edit copies or media. The runner sends it as the body of `PUT
  /api/pipes/:id` on its pooled connection.
- A **stream pipe** carries bytes that must arrive as they are written, for
  as long as they flow: the output of a service stream
  ([Service streams](#service-streams)).
  The runner sends it over a WebSocket it opens at `/api/pipes/:id`, its
  bytes in binary messages and its end in the close frame. A TLS edge in
  front of the backend, such as Cloudflare's, holds a request's body until it
  ends or passes about 2 MB, and passes WebSocket messages as they come: the
  live browser view of a still page, whose frames are small, showed nothing
  for minutes behind Cloudflare while its pictures waited in such a body, and
  went white again each time the view opened anew.

A pipe the runner reads from the backend is a `GET /api/pipes/:id` whose
response streams, which such an edge passes as it comes. Such an edge also
ends a response that sends nothing for about 100 seconds, before its head as
well as after: `demi shell status 17 --wait 5m` waits silently, and a stop
waits for a runner that is away, so their output pipes broke with
`pipe refused (524)` or `error decoding response body`. So the backend
answers the request at once and frames the body:

| Frame | Bytes | Meaning |
| --- | --- | --- |
| Data | `0x01`, a 4-byte big-endian length, then that many bytes, at most 64 KiB | The pipe's next bytes |
| Idle | `0x02` | Nothing to send: the backend writes it after 30 seconds without a frame |
| End | `0x03`, a 4-byte big-endian length, then a MessagePack map | The pipe's end: `{ ok: true }`, or `{ ok: false, cause }` with the words the reader reports |

- **Only the end frame ends the pipe.** A body that stops without one is a
  broken pipe, which the reader reports with its cause, as a failing relayed
  pipe does ([Handle an rpc call](commands.md#handle-an-rpc-call)), never as
  an ordinary end of its bytes.
- **The frames are the pipe's own.** The reader passes only the bytes of data
  frames on, so a command's output holds exactly what was written to it, and
  an idle frame never reaches it.
- **A stream pipe's WebSocket keeps itself alive the same way.** Its writer
  sends a WebSocket ping after 30 seconds without a message, which such an
  edge counts as traffic, so a live view of a still page outlives the edge's
  limit.
- **The same framing is the pipe's on every path.** Data, idle and end are
  the parts of a byte stream any transport between the runner and the
  backend carries, the relay's HTTP as much as a direct channel, so a pipe
  moves between them without a second definition.

Raw process environment selection follows these rules:

| Request | Child environment |
| --- | --- |
| No `env` | Inherit the device environment. |
| Explicit `env` | Use the supplied values. |
| `env` with `inheritEnv: true` | Overlay supplied values on the device environment. |
| Null value in an overlay | Remove that inherited variable. |

The device environment is the user's login environment: at its start the
runner runs the account's login shell, `SHELL` or the account's entry when
unset, as an interactive login shell, `-i -l -c`, and takes the environment
it prints, as VS Code resolves a shell's environment; a shell that fails or
takes over 10 s leaves the runner's own environment, and the log says so.
From that it takes away the names
that begin with `DEMI_` and, on Windows, those that begin with `=`, the
hidden per-drive directories such as `=C:` that cmd keeps and no program may
be given back; Windows' `Path` is the shell's `PATH`. Those configure the runner, such as
`DEMI_RELEASE_ID` and `DEMI_RUNNER_NAME`, or belong to whatever started it,
such as the `DEMI_*` settings of a developer's shell; none of them is meant
for the programs the runner starts. For example, a test that a job runs and
that reads `DEMI_RELEASE_ID` to choose a release would otherwise read the
runner's. The same environment reaches jobs, raw processes and resident
services.

Runner-owned variables, the local endpoint, the opaque context handle,
`DEMI_HOME` and the alias directory on `PATH`, override caller values. The
opaque handle ties command callbacks to the live execution, registration, and
pinned manifest; the live execution also holds the job's
[command context](native-runtime.md#command-context).
The runner releases contexts when execution completes, is cancelled, or loses
its backend connection. Provider CLI assembly can request environment inheritance,
but the runner has no provider-specific behavior.

### File contents

A file's contents never travel in a message; they go through a pipe, the
way command IO does. Messages carry the request, its reply and the file's
metadata.

- `fs_hashFile` names a file and the most bytes it may have; the runner
  answers its size and SHA-256 and sends none of its bytes, so the backend
  can tell whether it holds them already
  ([Attachment commands](commands.md#attachment-commands)). A file over that
  size answers `too_large` with its size, before any byte is read.
- `fs_readFile` names a file, an optional byte range (`offset`, and `length` up
  to the end when absent) and an output pipe. The runner opens the file and
  replies once it is open and positioned; only then does it stream the range
  into the pipe. A file that cannot be opened is an error reply, never an
  empty stream. The reply carries the file's version, from its size and
  modification time. A request may name the version it holds: when the file
  still has it, the reply says it is unchanged and the runner streams
  nothing.
- `fs_writeFile` names a file, whether to create its parent directories, and an
  input pipe. The runner writes into a temporary file beside the destination
  and renames it into place only when the pipe ends cleanly, then replies. A
  pipe that fails or is cancelled removes the temporary file and leaves the
  destination as it was.
- A write's reply comes within 60 seconds of its last byte leaving the
  backend, for `fs_writeFile` and `fs_writeDirectory` alike. By then the
  runner holds what the connection still queues, which it reads in moments,
  and has only its file to finish. A write whose reply has not come by then
  fails as interrupted, and the backend closes the connection that carried
  its pipe, so the runner's read ends with an error and removes its
  temporary file. Whoever asked for the write decides whether to try again: a
  Claude Code process that could not be started is a request that got no
  answer, which the agent retries
  ([Claude Code](../providers/claude-code.md#how-a-runtime-gets-its-process)).
  For example, a session file's last bytes once stayed queued where gVisor
  left them, and the write waited all night
  ([Backend socket](../cloud/managed-hosts.md#backend-socket)); this deadline
  turns such a stall into one failed start and a retry. The pipe itself keeps
  no deadline: only the reply of a write whose bytes have all left does.
- A pipe that fails, for example because the reader of a preview went away,
  stops the read and closes the file.
- Every pipe end named to the runner is reported with `pipe_done`, including
  one a refused request never used.

For example, the page seeks a video to the middle of a 300 MB file. The
backend asks for `fs_readFile` from byte 150,000,000 into a new pipe; the runner
opens the file, seeks, replies, and uploads the bytes as the backend accepts
them. When the user picks another file, the backend fails the pipe, and the
runner's upload ends and the file closes.

A write's temporary file is named `.demi-partial-` and six random characters,
for example `.demi-partial-q7XkP2`, and so is the one a
[file command](commands.md#file-commands) writes beside the file it replaces.
Only a runner or a command stopped in the middle of a write, by a crash or a
kill, leaves one behind, and nothing reads it again. Deleting one is always
safe: the file it was written for keeps its contents, and a write still running
when its temporary file goes fails instead of replacing the file.

### Working tree

A `git_changes` request lists the uncommitted changes under a directory, and
`git_show` sends a file as the last commit has it. The runner answers both in
process with gitoxide; the device needs no git executable. Both name the
directory as `root`; a `git_show` also names the file's path relative to it
and an output pipe, and streams the whole file into the pipe like any
[file contents](#file-contents). Git stores the file compressed, often as the
difference from another version, so the runner decodes it whole in memory
before replying.
The `git_changes` reply:

| Field | Meaning |
| --- | --- |
| `repository` | False when the directory is not inside a git repository; the other fields are then empty. |
| `gitDir` | The absolute path of the repository's git directory, which a page reads reports under by what each of its entries depends on. |
| `head` | The commit the changes are against; null before the first commit. |
| `files` | One entry per path `git status` lists under the directory, path relative to it: its `status`, git's two status letters for it; its `kind`, how the working tree differs from `head`: `added`, `modified`, `deleted`, or `renamed` (with the old path in `from`); and the lines added and removed against `head`. |
| `truncated` | True when the list stopped at 5,000 files. |
| `watched` | True when the runner answered from a watched baseline, described below. |

The list holds the paths `git status --porcelain --untracked-files=all` lists,
ignored files being absent, and `status` is the two letters it prints before
each: the index against `head`, then the working tree against the index. For
example, a new file nobody staged is `??`, a staged new file edited again is
`AM`, a file edited without staging is ` M`, a conflicted one is `UU` or
another conflict pair. A rename staged in the index is one entry at its new
path, with `R` first; a file moved in the working tree without staging is a
deletion (` D`) and an untracked file (`??`), as git reports it. Two cases
differ from git's list: a path git lists twice, a deletion staged in the
index and an untracked file on the disk, is one entry, `??`; and a path in
neither `head` nor the working tree, added to the index and then deleted from
the disk, is left out, having no side to compare. The web app marks each file
from its `status` the way VS Code's Git does.

`kind`, `from` and the line counts compare `head` with the working tree,
whatever the index holds: a path whose status git lists but whose content
matches `head`, such as an executable bit changed or a staged edit undone on
the disk, is `modified` with 0 and 0. Line counts skip binary files, by the
rule [edit tracking](edit-tracking.md#scope) uses, and files over 8 MiB; they report 0
and 0. `git_show` refuses a blob over 8 MiB with
`too_large`, answers `ENOENT` for a path the last commit does not have, and
`not_repository` outside a repository.

Three more requests read the repository's history, in process with
gitoxide too, for the Change view's Commits mode
([Commits mode](edit-tracking.md#commits-mode)). Each names `root`, and
answers `not_repository` outside a repository.

| Request | Reply |
| --- | --- |
| `git_branches` | `branches`, the local branches by name, most recently committed first, and `current`, the branch checked out, or null with `head`, the commit, when HEAD is detached. Remote-tracking branches are not listed. |
| `git_log` | Names `branch`, or none for HEAD, and `after`, a commit id to continue from, or none. `commits`: at most 50, newest first, following first parents only, each `{ id, message, author, time }`, the whole message, the author's name, the commit's time; `more`, true when older ones follow. A branch that no longer exists answers `ENOENT`. |
| `git_commit` | Names `commit`, an id or an abbreviation of at least 7 hex digits. `commit`, as one `git_log` entry with `parents`; `files`, as `git_changes` gives them, with `status` the one letter git's log gives a commit's file, `A`, `M`, `D` or `R`, `kind`, `from` and the line counts comparing the commit with its first parent, or with nothing for a root commit; `branches`, the local branches it is on, `current` first. A merge compares with its first parent, as GitHub and VS Code show it. An id the repository does not have answers `ENOENT`; an abbreviation that names two answers `ambiguous`. |

`git_show` takes `commit` too, to send a file as that commit has it instead
of the last commit's. A commit's file list is cut at 5,000 files as the
working tree's is.

The first request for a directory walks its whole tree. It also starts a
filesystem watch of the directory, and of the repository's `.git` when that
lies outside it, which records only the paths changed since: reading a file
changes nothing, and neither does metadata alone under `.git`. On macOS the
watch is an FSEvents stream, which covers a tree of any size with one
subscription; notify's kqueue backend there would hold a file descriptor for
every file, more than a repository and the usual limit of 256 allow. No
request waits for the watch to start: FSEvents can take seconds to start a
stream when the system is busy. The first request that finds the watch
running walks the whole tree once more, since the watch saw nothing before
it ran.

The next request re-examines the recorded paths, with the other path of any staged
rename among them so that the two still pair, and merges them into the
previous result. It walks the whole tree again instead when the repository's
`HEAD`, a ref or `packed-refs` changed (a commit, a checkout), or the index's
entries did (a staging): an index that `git status` only rewrote with fresh
file times, which it does on every run, holds the same entries and walks
nothing. It walks it again too when a `.gitignore` or
`.gitattributes` changed, since those decide what git lists for other paths,
or when more than 100 paths changed: a walk over some paths checks every index
entry against each of them, so past about a hundred it costs more than a whole
walk. Paths git ignores are not recorded and do not count, by the same filter
[subscriptions](#watching-files) use, so a build that writes 20,000 files
under `target/` leaves nothing to re-examine. The watch covers only the directory, so each request also compares the
`.gitignore` and `.gitattributes` files in the directories above it, up to the
work tree, with what the last walk found. Settings outside the repository, such
as the user's global ignore file, apply from the next whole walk. When the
watch cannot be created (an inotify limit, permissions, an unsupported
filesystem) or fails, the runner walks the whole tree for every request; when
it lost events or the platform asks for a rescan, the next request walks the
whole tree, and so does the one after a computation that failed or timed out,
since that computation had already taken the paths the watch recorded.
`watched` reports whether a watch is running.

The runner keeps at most eight watched directories per connection and drops one
after fifteen minutes without a request, unless a
[subscription](#watching-files) still uses it; closing the connection drops
them all.

Working-tree work runs on blocking threads off the connection's control thread.
At most two computations run at a time; a request beyond that waits for one to
finish ([Load](#load)), and requests for the same directory share one
computation. A computation stops at its next check when the connection closes
or after thirty seconds of running (`timeout`). A failure inside the git
library answers `internal` for that request and affects nothing else.

### Watching files

The backend subscribes, for the pages it serves, to what they keep of a
Host's files, and the runner says when a subscription's subject changed, by
its id alone; it never sends a path or content
([File watch](../product/web-api-hosts.md#file-watch)). For example, a page keeps
`src/app.ts`'s text and the working tree's changes list. The agent edits
`src/app.ts` and runs `cargo check`, which writes 20,000 files under the
ignored `target/`: the runner reports two subscriptions changed, the file and
the working tree, once each, and nothing for `target/`.

**Subscriptions.** `fs_subscribe` names an id, a kind and a path; the
runner answers `live` once the file system watch behind it runs, or `failed`
with the reason when it cannot (an inotify limit, permissions, a file system
that reports nothing). Each subscription stands alone: one that fails
affects no other. The kinds, and when each changes:

| Kind | Changes when |
| --- | --- |
| `file(path)` | The file is created, written, removed or renamed, or its metadata changes; or a folder above it is removed or renamed |
| `folder(path, names)` | An entry of it comes, goes or is renamed, by the entry's identity, its device and inode, so a file written in place is no new entry; or the folder or one above it is removed or renamed |
| `folder(path, details)` | As `names`, or an entry's content or metadata changes, which moves its size or its time |
| `worktree(root)` | A path under `root` that git does not ignore changes, metadata included, since a changed executable bit changes `git status`; in the repository's git directory the index, `HEAD`, a ref under `refs/`, `packed-refs` (in a linked worktree, the refs of the common git directory), `config` or `info/exclude` changes; or a `.gitignore` or `.gitattributes` changes. Outside a repository it changes only when a `.git` appears under `root` |
| `head(root)` | `HEAD`, a ref under `refs/` or `packed-refs` of `root`'s repository changes; in a linked worktree `HEAD` is the worktree's own and the refs are in the common git directory, both watched |

A git directory concerns only its own repository: in a checkout with nested
repositories, such as Chromium's, a commit in `src/v8` changes `src/v8`'s
subscriptions and none of `src`'s.

**Numbers.** The runner counts the file system events it receives with one
number per process that only grows, and names the process's run with an
epoch that changes when the runner starts again. `live` carries `since`, the
number at which the subscription started to watch, and the epoch. A change
is reported as `changed` with the subscription's id and `at`, the number of
the event that changed it. Every read of what a subscription covers, a
file's text or description, a folder's listing, the working tree's changes,
a changed file's sides, a committed file, takes the number before it reads
and answers it with the epoch, through the backend and through a
[direct channel](#direct-channels) alike, since the number is the process's.
So an answer read at 40 is out of date when a change at 41 arrives, in
whichever order the two reach the page, and an answer read at 41 already
holds the change an event at 41 reported: a change made after a read always
has a higher number than the read, since the number was taken before it.

**Batches.** The runner gathers changes for 100 ms and sends each
subscription at most once per batch, with the highest `at` it saw. There is no
limit on how many subscriptions one batch names, since a batch names no
paths.

**Lost events.** When the platform says it dropped events, the subscriptions
the drop concerns changed, reported as any change, with the number of the
drop: FSEvents' `MustScanSubDirs` names a folder, and only the subscriptions
under it changed; `UserDropped`, `KernelDropped`, `RootChanged`, `Mount` and
`Unmount`, inotify's queue overflow and notify's `need_rescan` concern every
subscription on that stream. A drop on a stream outside the working tree
changes nothing inside it.

**What git ignores is filtered out here.** The runner decides whether a
changed path is ignored by the working tree's repository's rules, with a
cache of the ignored folders, so a path under `target/` costs one prefix
comparison. An ignored path changes no `worktree` subscription and is not
counted among the [working tree](#working-tree)'s changed paths; a `file` or
`folder` subscription inside an ignored folder, such as a log under `target/`
a page shows, changes as any other.

A subscription inside a working tree rides on the tree's watch: the relay
and a direct watch hold the tree's watch while anything inside it is
subscribed, the page saying nothing of it, and a tree's watch that fails
fails the subscriptions inside it. A `head` subscription rides on the watch
of its repository's tree.

**Leases.** A subscription is a lease of 10 minutes from when the runner
received its `fs_subscribe` or its last `fs_renew`, which names many ids at
once. The runner ends a subscription whose lease ran out and releases what it
held; `fs_unsubscribe` ends one at once. Closing the connection ends them
all. Starting and stopping a shared platform watch run on the blocking pool,
never on the runner's control thread. The last subscriber releases its watch
without waiting for a thread to join; the connection owns that cleanup and
awaits it when it closes. Closing also ends a watch still being started, so
neither a late start nor a direct channel can leave one behind.

On Linux the folder-adder thread receives an explicit stop message. It must
not wait for all folder-event senders to disappear: the watcher it owns also
owns one of those senders. Its folder queue is bounded; if it fills, the watch
reports failure rather than silently missing new folders. Failure to start
the thread is a watch failure too.

The backend renews a subscription while a page still renews it
([File watch](../product/web-api-hosts.md#file-watch)).

**How the platforms watch.** One subscription costs nothing of its own when
the file system watch behind it already runs:

- On macOS a working tree is one FSEvents stream, which covers a tree of any
  size with one subscription, excluding up to eight large ignored folders with
  `FSEventStreamSetExclusionPaths`; notify's kqueue backend would hold a file
  descriptor per file, more than a repository and the usual limit of 256
  allow. A file or folder outside the working tree is watched alone with
  kqueue, never through a stream over its parent's whole tree, at most 64 of
  them, and a file replaced by a rename is watched again under its name; a
  `folder_details` subscription there is an FSEvents stream over that folder
  alone, since kqueue does not see its entries' contents change, and counts
  among the 64. kqueue sees its own node removed or renamed, not a folder
  above it: outside the working tree, a rename of a folder above a
  subscription goes unseen until the node itself changes. A
  subscription inside an excluded folder gets a watch of its own.
- On Linux inotify watches the working tree's folders except the ignored
  ones; a subscription inside an ignored folder adds a watch of that folder.
- FSEvents can take seconds to start a stream when the system is busy, so
  `fs_subscribe` does not wait for it: `live`, with its `since`, says when
  changes from then on are covered.

### Service streams

A `service_open` request asks the runner to open a
[user stream](native-runtime.md#user-streams) and carry its bytes both ways.
It names the stream, its [command context](native-runtime.md#command-context),
the declared package and operation, optional `args` and `json` that the
invocation receives as a command's does, the conversation's directory, which
becomes the invocation's `cwd`, and two pipes ([Pipes and output](#pipes-and-output)): `input`, whose bytes the
runner delivers to the invocation as input chunks when the operation asks for
them, and `output`, into which it writes the invocation's standard output. Its
standard error goes to the [Host's log](#host-log). The runner
starts the invocation in the resident service that holds the conversation's
state, starting the service when needed, and answers `service_opened`, or
`service_error` with `unknown_operation`, `service_failed` (the backend passes
this on to the plugin as a refusal of its own, so the page can name the
service that did not start), or `refused`; no
bytes move before that answer. The request may also attach artifacts with
their locations, which the stream may install beside its package's own. Starting
the service may need its executable, and the invocation may ask for other
artifacts: the runner asks the backend for their locations as it does for a
job's command ([Install artifacts](native-runtime.md#install-artifacts)),
naming the stream instead of a job, and the backend answers only while the
stream is open. A [direct channel](direct-channel.md)'s stream, which the
runner reports with `direct_stream`, asks the same way. The input pipe ending ends the invocation's
input; the invocation's completion ends the output pipe, which is how the
backend learns the stream is over; a pipe failing or the connection to the
backend closing cancels the invocation. The runner reports each pipe end with `pipe_done` like any other
pipe. When the invocation completes, the runner also sends `service_done` with
the stream, the invocation's exit code and the bounded tail of its standard
error: a one-shot call has no page to tell, so its caller learns a failure
and its words from this message, not from a stream that merely ended. The service stream is generic mechanism: the
runner does not parse what flows through it. The backend uses it for the
[live browser view](../browser/live-view.md).

### Direct channels

A page can reach the runner without the backend in the middle, over WebRTC
data channels the backend introduces and authorizes
([Direct channel](direct-channel.md)). The backend forwards a page's offer as
`direct_offer`, the runner answers with `direct_answer`, `direct_close` ends a
peer, and the runner reports each direct stream's opening and closing with
`direct_stream`, which counts as the conversation's activity. The runner carries out a direct operation with the same
functions as the backend's request for it, and closes every peer when its
connection to the backend ends.

## Host log

A Host that cannot say what went wrong cannot be debugged. For example, the
live view module fails to list the conversation browser's tabs on a Cloud and
writes why to its standard error; without a log those words are gone, and the
page only sees a view that never learns its tabs.

The runner keeps one log per Host, in its own data directory: text lines, each
with a time, a source and, when the work belongs to one, a conversation id.
Every part of the runner writes its diagnostics as `tracing` events, the
logging interface Demi's Rust programs share, with the source and the
conversation as event fields, and the Host log is a `tracing` layer that writes
those events as lines. No module holds a handle to the log's files. A resident
service's standard error enters the same way, one event per line as it arrives.

| Source | Lines |
| --- | --- |
| `runner` | Connecting and losing the backend, starting and stopping services, a [preinstalled artifact](native-runtime.md#preinstalled-artifacts) it does not use and why, opening, refusing and ending streams, failed Host operations, guest boot |
| `service:<package>` | Every line a resident service writes to its standard error, as it arrives |
| `stream:<operation>` | The standard error of a [service stream](#service-streams)'s invocation, for example `stream:browser.live` |

- The log is bounded: two files of 4 MiB, the older replaced when the newer
  fills. It outlives a runner restart and, on a Cloud, a stop and a wake; a
  reset starts it again. A paired device keeps it in `log/` under its
  installation state. A managed guest's state directory is temporary, so the
  guest keeps it in `/var/log/demi`, on the
  [system layer](../cloud/managed-hosts.md#images).
- Writing never holds up the work it describes: the layer queues each line,
  and one writer thread owns the files. A line the queue has no room for is
  dropped and counted, and a write the disk refuses loses that line alone.
- A `log_read` request returns lines after a cursor, up to a limit of 1000,
  optionally of one source; `log_lines` answers with the lines oldest first
  and the cursor to continue from, or `log_error` when the files cannot be
  read. Without a cursor the answer ends at the newest line. The cursor is a
  line number that grows across both files and across restarts; one whose
  lines are gone, or that comes from a log a reset removed, continues from
  the oldest line still kept. The writer thread answers the request between
  two lines it writes, so the answer holds every line queued before the
  request and a read never races a write. The backend serves it as the
  [device log route](../product/web-api-hosts.md#device-log).
- What a source writes is diagnostics: what it tried and why it failed. Page
  content, typed text, cookies, tokens and file contents never go to the log.
- A failure the user's page is waiting on is also told to the page, by the
  request's answer or a stream's `notice`. The log is where the cause is
  found, never the only place a failure is told.
- A resident service that exits or breaks the protocol is recorded with its
  exit status after the standard-error lines that led to it, and every call it
  failed carries the last part of that output as well
  ([Invoke and retire a service](native-runtime.md#invoke-and-retire-a-service)).

The same log, route and bound serve a paired device and a Cloud.

## Load

The runner never refuses, drops or fails a request because others are in
flight. Work that lasts as long as its caller wants is not counted at all:
native command calls, commands the backend implements, user streams, network
streams and local command connections. Each is paced by its own backpressure,
a stream window or a pipe that moves only what the other side accepts, so a
slow reader holds back only itself and memory grows only with what runs. An
agent's `demi browser wait` or a user's open live view holds nothing another
command needs.

Host work that finishes on its own runs on blocking threads, off the control
thread, a few at a time, and a request beyond that waits for a slot:

| Work | At a time |
| --- | --- |
| Filesystem requests | 32 |
| Working-tree requests | 8 |
| Working-tree computations | 2 |
| Filesystem syncs | 4 |

A waiting request still ends when its connection closes.

Every job, stream and request shares the runner's one table of open files. A
job's login shell alone can hold about a hundred while it reads a profile that
loads a version manager such as nvm, and launchd gives a service on macOS a
limit of 256. So when it starts, the runner raises its own limit as far as the
system allows: to the hard limit, and on macOS to at most the kernel's limit
for one process. Every process it starts gets back the limit the runner was
started with, the one it would have from a terminal, unless its job's `ulimit`
set another ([Builtins that act on a process](#builtins-that-act-on-a-process)),
because some programs
misbehave with a very high one: a program that uses `select()` cannot watch a
descriptor numbered 1024 or above, and some programs close every descriptor up
to their limit before they start. Windows has no such limit.

Every pipe, local command connection and open file holds one while it lasts.
When none is left,
whatever needs one waits until another closes, instead of failing: pipes,
local command connections, filesystem and working-tree
requests, file transfers, process and job starts, and native service starts.
Inside a running job, every descriptor its shell makes waits too: pipes and
redirections, the copies it makes for subshells, pipeline stages, builtins and
`2>&1`, a here-document's file, a background list's `/dev/null`, and the
standard streams and descriptors a standard utility or a program it starts
receives. For example, out of open files, `echo one | cat > piped.txt` waits,
then writes the file. Nothing tells the runner when one closes, so it tries
again, at most a tenth of a second apart; a job's wait ends when the job is
cancelled. Running out is never an answer either: a working-tree request does
not report a directory as outside a repository because it could not open the
repository's files.

A standard utility waits too when it opens a file or starts a program. Its
other file work, such as `ls` reading a directory or `sort` spilling to a
temporary file, fails as it would on any system out of open files: making
every utility wait would change most of them, and the raised limit makes
running out rare.

A process start also waits, for about a second, while its program is busy.
Linux refuses to run a file that any process holds open for writing, and the
runner causes that itself. For example, it copies a service's executable into
its cache and starts it while another thread starts a job's `git`. Starting a
process copies the runner process first, and the copy holds every file the
runner had open, the cache file too, until it runs `git` a moment later. A
service start in that moment fails with `Text file busy` although the runner
has closed the file, and a job that writes a script and runs it can meet the
same. Every copy the runner makes runs its program at once, so the wait is
short; a program still busy after a second is open for writing elsewhere, and
its start fails. Every process the runner starts waits this way: services, raw
processes, a job's commands and the programs its utilities start, such as
`env` and `xargs`.

One refusal remains. A conversation browser command that conflicts with
another command on the same tab answers `tab_busy`; that is about the tab, not
load ([Conversation browser](../browser/browser.md#one-tab-registry)).

## Shell jobs

A shell job owns its working directory, environment, IO, and asynchronous work.
It also carries the [command context](native-runtime.md#command-context) the
backend built for it; the runner never derives that context from the job's
environment.
Brush runs inside the runner process, on a shell runtime separate from the
control thread. Declared roots call the shared command dispatcher; external
tools such as Git, Python, and Node run as child processes. Every in-process
file write passes through the job's scope, which reports the files the job
created or modified when it exits ([Edit tracking](edit-tracking.md)).

Each interpreter unit, meaning the job's script and every pipeline stage,
subshell, background list, and process substitution, runs on a thread of its
own, and so does each embedded utility, from a large pool that serves only
shell work. On Unix the runner reads and writes its end of each job pipe
asynchronously, so a job's input and output take no threads; on Windows each
end takes one thread from the shell pool.
[Concurrency](../architecture/concurrency.md#runner) says why shell work has a
pool of its own.

The diagram shows ownership, not execution order. Cancelling job A releases its
work while preserving the runner and job B.

```text
Runner process
+--------------------------------------------------+
| Job A                    Job B                   |
| +--------------------+   +--------------------+  |
| | Brush execution    |   | Brush execution    |  |
| | Background tasks   |   | Background tasks   |  |
| | IO and child work  |   | IO and child work  |  |
| +--------------------+   +--------------------+  |
+--------------------------------------------------+
```

Each job starts a fresh login shell in the directory its request names, which
for a conversation's command on its primary Host is the working directory of
the agent that ran it, and on an attached Host that Host's starting directory ([Running shell tools](../agent/shell-tool.md#running-shell-tools)).
Brush loads the system profile and first readable user login profile. The
runner then restores its execution context and places command aliases first
in PATH. Nothing of an earlier job carries over, neither its directory nor its
shell variables and functions; only persisted profile changes do. A directory
that does not exist, such as a workspace the user deleted, fails the job
before its script runs, with exit status 1 and the line
`demi: cannot start in <directory>: No such file or directory`. The backend
receives the foreground exit status.

A job's environment is the device environment
([Host operations](#host-operations)) with the job's own variables added:
the command endpoint and context handle, `DEMI_HOME`, the alias directory on
`PATH`, `DEMI_JOB_ID`, `DEMI_LIVE_INPUT`, and for a conversation's job
`DEMI_SCRATCH`, its scratch directory ([Scratch directory](#scratch-directory))
([Command context](native-runtime.md#command-context),
[A job's own input](#a-jobs-own-input)). The runner
sets no temporary directory for a job: `TMPDIR` is the device environment's,
or unset, so a job's temporary files go where any program of the device's
user puts them, `/var/folders/…/T/` on macOS and `/tmp` on a Linux device
without `TMPDIR` (a Cloud names one, [Images](../cloud/managed-hosts.md#images)).
For example, a dev server a job starts creates its Unix socket in `$TMPDIR`
and keeps it after the job ends. A temporary directory of the job's own
would break both: macOS limits a socket's path to 104 bytes, which a path
under the job root exceeds, and a daemon the job started would outlive the
directory.

### Scratch directory

Each conversation has a scratch directory on each Host it runs on, for the
files only the agent needs: a screenshot it looks at, a probe script, a log
it reads back. For example, the agent saves `$DEMI_SCRATCH/page.png`, views
it and moves on; the file shows in no pill and no Change view, and it is
still there when the agent looks again an hour later, which `/tmp` does not
promise, since a system may clear it.

- **Where:** `<DEMI_HOME>/scratch/<conversation id>/`, which the runner
  creates at the conversation's first job on that Host; on Windows under
  `%USERPROFILE%\.demi`. The root agent's is that directory; each subagent
  gets its own folder inside it, `agent-<number>/`, by the number `demi
  agent` gives it, so two subagents that both save `shots/before.png`
  overwrite nothing of each other's, and the parent still reads what a
  child saved by the path the child reports, since the child's folder is
  inside its own.
- **`DEMI_SCRATCH`** names the agent's own one in every job, on the
  primary Host and on attached ones, so the commands a job runs find it in
  their environment.
- **The backend knows it too:** the runner's hello names its scratch root,
  the absolute `<DEMI_HOME>/scratch`, beside its home directory, so what the
  backend writes into a conversation's scratch, such as a file
  `demi conversation history open` saves, goes to that conversation's
  directory whatever a job's environment says.
- **It goes with the conversation:** archiving keeps it. When the runner
  connects, the hello's answer names the conversations of the user that
  still exist, and the runner removes every scratch directory of a
  conversation not among them, so a deleted conversation's scratch leaves a
  Host offline at the time as soon as it connects, with nothing to remember
  in between. A backend that cannot read the conversations then drops the
  connection rather than name none, which would remove every scratch
  directory; the runner connects again.

The user login profile is the one in the job's home, the directory the job's
`HOME` names, and the profiles see that directory as `$HOME`. A job's `HOME`
is the one its request sets, else the device environment's, else the home
the runner reports for the device
([Resolve a target](sessions-and-targets.md#resolve-a-target)), so every job
has one. For example, a runner that a service manager starts without `HOME`
gives its jobs its account's home: they read `~/.profile` there, and a line in
it such as `. "$HOME/.local/bin/env"` finds its file. Without that default,
brush would read the same profile with `$HOME` unset, and the line would look
for `/.local/bin/env`.

Tests that start jobs give each job a home of its own, so no test reads the
login profile of the machine's user. The system profile belongs to the
machine, and jobs in tests read it too. A test therefore does not assume how
long a job takes to start or how many open files its start holds. A system
profile that loads version managers such as nvm can take a job a quarter of a
second to source and hold about a hundred pipes at once, since every unit of
a job shares the runner's open-file table ([Load](#load)).

A job waits for background tasks and process substitutions before reporting
completion. For example:

```sh
(sleep 2; echo done) & echo started
```

The caller sees `started`, then a running job, then `done` and completion.
A tool timeout returns the running job's handle. `demi shell status` observes that job,
and `demi shell stop` stops it ([Cancellation and completion](#cancellation-and-completion)).
Background tasks remain job-owned rather than becoming detached services.

### Background tasks and timeouts

For example, a script starts a dev server in the background, checks it, and
stops it:

```sh
(cd app && bun run dev) &
server=$!
curl -fsS localhost:3000/health
kill $server
wait $server
```

`kill` stops the subshell and everything it started, `bun` and the backend
`bun` runs included, and `wait` gives `143`, the status of a task that `TERM`
ended. In bash the same `kill` signals only the subshell's own process, and
the dev server it started runs on, holding its port.

- **`$!`.** A background task, a list after `&` or a coprocess, runs in the
  runner, so no operating-system process stands for it. `$!` names it with
  an id that no process ID takes, from 4194305, one more than Linux's
  largest process ID, numbered within the job, and `jobs -p` lists the
  same ids. `kill`, `wait` and `jobs -p` take a task's id wherever bash
  takes a process ID.
- **`kill`.** The default signal is `TERM`, as in bash. A task's id signals
  the whole task: `TERM` stops its shell work at its next step and sends
  `SIGTERM` to every process group it started; `KILL` kills them and the
  rest of its work. A process ID or a negative process group ID, after `--`
  or not, signals that process or group, as in bash. Errors are bash's:
  `kill: (999999) - No such process`, ``kill: `abc': not a pid or valid job spec``,
  and `kill` alone prints bash's usage with status 2. `kill $$` and `kill 0`
  still fail ([Builtins that act on a process](#builtins-that-act-on-a-process)).
- **`wait`.** `wait <id>` waits for that task, or for a process the job
  started, and gives its status as bash does: the task's last status, or
  128 plus the signal that ended it. An id that is neither fails with
  `wait: pid 999999 is not a child of this shell` and status 127. `wait`
  alone waits for every task and gives 0.
- **`disown` and `nohup`** keep a task in its job: the command runs on
  while the task does, and stopping the command stops it, as the runner
  leaves no process that no command stands for. `disown` succeeds and
  changes nothing else; `nohup` runs its command ignoring `HUP`, as GNU's
  does. A server meant to outlive a script runs as a command of its own,
  with `intervalMs` null ([Tool input](../agent/shell-tool.md#tool-input)).
- **`timeout DURATION COMMAND…`** runs COMMAND as `command` would, a builtin,
  a standard utility or a program, so its writes are tracked like any
  other, and stops it as `kill` stops a task once DURATION has passed. Its
  options, durations and statuses are GNU's: `-s SIGNAL` (`TERM` by
  default), `-k DURATION` to send `KILL` after that much more, `--preserve-status`,
  `--foreground`, `-v`; `124` when the time ran out, `137` when `KILL` ended
  it, `125` for its own failure, `126` and `127` when COMMAND cannot run.
  For example, `timeout 600 cargo test` ends a hung suite after ten minutes
  and leaves `124` for the script to test.

### Windows paths

On Windows a job's shell takes drive paths as Windows writes them,
`C:/Users/alex/notes.txt` or, quoted, `'C:\Users\alex\notes.txt'`; a bare
backslash path does not work, since bash reads each backslash as an escape,
as it does on every system. The Git Bash and MSYS form `/c/Users/…` is not a
drive path here: it names `/c` at the root of the current drive, which does
not exist, and the context block tells the model to write `C:/…` instead.
Paths the runner prints, such as the directory a shell starts in, use the
system's own form, `C:\Users\alex\Downloads`, and so do the paths the
backend builds on a Host, such as where an attachment is saved: the backend
joins them with the separator of the system the Host's runner reports, in
one helper every caller uses, so a Windows Host shows
`C:\Users\alex\.demi\attachments\…`, never `C:\Users\alex/.demi/…`.

### Standard utilities

A job's standard utilities behave as GNU's do on every Host, macOS included.
For example, on a Mac `sed -i 's/a/b/' notes.txt` edits the file in place
as on Linux, and `sed -i '' 's/a/b/' notes.txt`, the form macOS's own sed
takes, fails as GNU sed fails, reading the empty argument as the script and
the script as a file: `sed: can't read s/a/b/: No such file or directory`.
The context block tells the model so
([Switch the primary target](sessions-and-targets.md#switch-the-primary-target)).

These utilities run in the runner, from uutils and the GNU-compatible tools
vendored beside it, so their writes pass through the job's scope and edit
tracking records them ([Edit tracking](edit-tracking.md)): `basename`, `cat`,
`chmod`, `chown`, `cmp`, `cp`, `cut`, `date`, `df`, `diff`, `dirname`, `du`,
`env`, `find`, `grep`, `head`, `jq`, `ls`, `mkdir`, `mktemp`, `mv`, `nl`,
`od`, `paste`, `realpath`, `rg`, `rm`, `rmdir`, `sed`, `seq`, `sleep`, `sort`,
`stat`, `tac`, `tail`, `tee`, `timeout`, `touch`, `tr`, `uniq`, `wc` and
`xargs`. A Host's own programs of these names stay where the system keeps
them, such as macOS's BSD tools in `/usr/bin`.

- **What they print.** Options, output, exit statuses and messages are
  GNU's, in English, as uutils prints them where it has the utility:
  `ls: cannot access 'nope': No such file or directory` with status 2, and
  `wc` ends its counts with `total`. `sed` follows GNU sed: an unreadable
  input file prints `sed: can't read <file>: No such file or directory`, the
  other files are still processed, and the status is 2.
- **Paths.** Every path a utility takes is resolved against the job's
  directory, never the runner's own, for reading its metadata, changing its
  mode or owner, setting its times and asking for its file system alike.
- **Copies.** `cp` makes the copy as GNU's does: its times are the time of
  the copy and its mode the source's less the job's umask, unless `-p`,
  `-a` or `--preserve` keeps them, and an existing destination is written in
  place, so a hard link to it stays one. It clones the data where the file
  system can, as `--reflink=auto`, which is GNU's default; `-c`, macOS
  `cp`'s flag for a clone, means the same. For example, restoring a source
  file from a copy with `cp` gives it a new time, so a build sees it changed.
- **Programs a utility starts.** `find -exec`, `xargs`, `env` and `timeout`
  start a command; a bare name that is one of these utilities runs that
  utility in the runner, as the shell would, and a path such as
  `/usr/bin/sed` runs that program. `type sed` says the shell's utility;
  `which sed` names the system's program, since `which` searches only `PATH`.
- **The shell's own builtins** word their errors as bash does:
  `cd: ./browse: No such file or directory`,
  `nosuch: command not found` with status 127.

GNU's behaviour on every Host is one behaviour to know, whatever the Host's
system: a script written on a Linux Cloud runs on a Mac unchanged. A model
that writes a BSD form gets GNU's error for it, which names the problem, as
GNU's messages are the ones models know best.

### Builtins that act on a process

In bash, a few builtins act on the shell's own process: `exec` replaces it
with a program, `ulimit` and `umask` set the limits and file mode mask that it
and its children have, `kill $$` signals it, and `suspend` stops it. A job's
shell has no process of its own: it runs in the runner, whose process every
job, stream and request shares. A job that ran `exec node server.js` the bash
way would turn the runner into `node`, which ends every other job and the
device's connection, and a job's `umask 000` would make every file the runner
creates afterwards writable by anyone. So in a job each of these acts for the
job's shell instead, or fails. A job's shell starts with what the runner gives
the processes it starts: the runner's umask and limits, and for open files the
limits the runner was started with ([Load](#load)).

The runner's own mask is the one it was started with. The shell installer
keeps the installation's files private with mask `077`, but starts the runner
with the mask of the shell the user ran it in, so a file the agent makes on a
device gets the mode the user's own programs would give it. A Cloud's runner
starts with `022`, which the machine manager sets for the sandbox's first
process. Windows has no mask: a file takes the permissions of the directory
it is made in.

| Builtin | In a job |
| --- | --- |
| `exec CMD` | Runs CMD as `command CMD` would, a standard utility in the runner or a program through the job's process start, then ends the shell with CMD's status. In a subshell it ends the subshell. With only redirections, they stay with the shell, as in bash. |
| `ulimit` | Sets and shows the limits of the processes the shell starts from then on; a subshell keeps its own. Without `-S` or `-H` it sets both limits, as in bash. A hard limit raised above the job's own without privilege fails at once, as in bash. The system checks every other new limit when it is set: the shell starts `/bin/sh -c :` with it, and a limit the system refuses, such as open files above macOS's cap, fails there and changes nothing, as it would in bash. |
| `umask` | Sets and shows the mask of the processes the shell starts and of the files its redirections and standard utilities create. The runner's own mask still applies beneath it inside the runner, so there a job's mask can only take permissions away. For example, under a runner mask of `022`, a job's `umask 002` gives its programs group-writable files, but its redirections still create files with mode `644`. |
| `kill` | Signals any process but the runner, and a background task by its id ([Background tasks and timeouts](#background-tasks-and-timeouts)). `$$` is the runner's process ID and 0 its process group, so `kill $$` and `kill 0` fail with a message. |
| `suspend`, `fg` | Fail: a job has no job control, as a bash script has none, and `suspend` would stop the runner. |

A job's limits apply to its processes only: its builtins and standard
utilities run in the runner, with the runner's limits. The other builtins act
on the shell alone already: `cd` and the directory stack, `trap`, which
installs no signal handler in the runner, `set` and `shopt`, `exit`, `wait`,
`jobs` and `bg`. `times` shows the runner's processor time, not the job's.
`time` reports the processor time of the processes the timed pipeline
started, each from its own `wait4` usage, and none of the runner's: a
builtin or standard utility in the pipeline runs inside the runner, so its
time is left out, and the report says so with a line `(work inside the
runner not counted)` when the pipeline held any. For example, `time sleep 60`
reports about 0 seconds of user and system time, where it once reported the
runner's and other jobs' 36 seconds.

### A job's own input

A command reading its stdin must know whether that stdin is the job's own
input, which `demi shell input` feeds and which ends only with the job, or a
finite one: a pipe, a file, a heredoc. For example, `demi file view` with no
path reads a medium from a pipe in `convert … png:- | demi file view`, but
run alone it would wait for the job to end, so there it refuses
([Media the model views](../agent/shell-tool.md#what-demi-file-view-shows)).

Brush gives a builtin its descriptors as the shell has set them up for that
call, after redirections, pipelines, command and process substitutions,
`exec` redirections and subshells: `ExecutionContext::try_fd(0)` is the file
the command reads. Its kind does not answer the question: under the
runner's execution host every descriptor is an `OpenFile::Controlled` that
wraps its system file. The system file does answer it, so brush needs no
change:

- **The reference.** The runner keeps a copy of the reading end of the job's
  stdin pipe open for the job's life and names it in the job's environment
  as `DEMI_LIVE_INPUT`: on Unix the pipe's device and inode, which every copy
  of either end shares and no other open file has while the copy is open; on
  Windows the runner's process id and the copy's handle, which a command
  compares with its own handle through `CompareObjectHandles`.
- **The comparison.** A declared command compares its fd 0 with the
  reference: a builtin in the runner before it dispatches, an alias in its own
  process before it forwards
  ([External command clients](commands.md#external-command-clients)). One
  function makes both comparisons, and the invocation carries the answer.
- **A relayed stdin.** A job whose stdin the backend relays from another
  command, as `demi host shell` starts one, gets no `DEMI_LIVE_INPUT`: its
  stdin is the invoking command's.
- **The environment is the script's.** A script that changes
  `DEMI_LIVE_INPUT` makes only its own commands misjudge their stdin, which
  reaches nothing but that job, so the variable needs no protection; the
  command context, which does, never travels in the environment
  ([Command context](native-runtime.md#command-context)).

### Cancellation and completion

Completion means the job has released its local work and IO, not merely that its
foreground script returned. The shell scope tracks interpreter tasks, utility
workers, and external children until they finish.

| Outcome | Cleanup |
| --- | --- |
| Success | Join remaining job-owned work and preserve produced output. |
| Failure | Cancel remaining work, join it, and report the failure. |
| Stop | `TERM`, then `KILL`, below; release IO and reap children. |

Embedded execution cooperates with cancellation. Blocking IO must be
interruptible: on Unix, a unit blocked on a read or write waits on the file and
on its job's cancellation together, so cancelling the job wakes it at once; on
Windows, the runner cancels the blocked call. External children belong to a Unix
process group or Windows Job Object. The runner continues handling control
requests while a job blocks on input or output.

A job is stopped in two steps, as a program in a terminal is. `TERM` stops
its shell work at its next step, so the script runs no further command and
its builtins and utilities end, and sends `SIGTERM` to every process group
the job started, so a dev server can close its connections, stop the
programs it started and remove its socket. A program can leave the job's
groups, as one that calls `setsid` to run as a daemon does, so each signal
also goes to every process that still carries the job's `DEMI_JOB_ID` in its
environment, which every process the job starts inherits, as GitHub's
runner finds a job's processes by `RUNNER_TRACKING_ID`: the runner reads the
environments of its user's processes (`/proc/<pid>/environ` on Linux,
`KERN_PROCARGS2` on macOS) at each signal. macOS shows a process's environment
only for programs that are not Apple's own, so each signal also goes to the
descendants, by parent process id, of every process it reaches: a `setsid`
child of `/bin/sh` whose parent still runs is found that way. A process
that cleared its environment, or an Apple program orphaned by a double fork,
escapes, and none of the runner's own resident services carries a job's id. The job ends once every process of those groups, and every process a signal of the stop reached, has exited.
`KILL` kills whatever remains, and is what a
stop sends when the job has not ended 5 seconds after `TERM`. Work running
inside the runner, an embedded utility or an interpreter unit, cannot be
killed: it stops at its next check of the job's cancellation, and each such
utility checks it at least once per entry or block it reads, so even a quiet
`find /` stops. A thread can still sit in a system call that only a signal
would interrupt, such as an `open()` the kernel holds while macOS asks the
user for access to another app's data. So once `KILL` has gone, the job ends
1 second later whatever in-process work is left, reporting `SIGKILL`, and
the Host log names the work it left behind, which ends by itself at its next
check. For example, `find ~ -name AGENTS.md` blocked under
`~/Library/Group Containers` behind an unanswered privacy prompt ends 6
seconds after its stop, where it once ran until the prompt was answered
([The `demi shell` commands](../agent/shell-tool.md#the-demi-shell-commands)). Before, a
stop went straight to `SIGKILL`, and a dev server killed that way left its
backend running in a group of its own, holding its port.

A stopped job reports the signal that ended it.

A job or raw process ends with its exit code, or else the name of the signal
that ended it. With neither, the runner says why it has no status: the work
could not start, or the runner failed it before its end was known, in the
runner's own words. A failure of the runner's beside a known status goes to
the Host log.

A cancellation request alone does not establish that execution stopped: a
command is stopped only when its runner says its job ended. For example, the
user stops a build while the Mac running it is reconnecting: the command's
tab says *Stopping* and the command still counts as running; the backend
sends the stop again as soon as the runner's link is back, `TERM` and then
`KILL` 5 seconds later if the job has not ended, and the command turns
stopped when the runner reports the job's end. A runner that does not come
back within the time it keeps jobs while away ends them itself, and the
command then ends as lost, with that reason, as any command of an absent
runner does.
Cancellation does not undo completed file changes or other side effects.

## Command lifetime

Builtin calls and external forwarding use the same execution context. Releasing
that context releases its command bindings and its leases on resident services.
The command set and package catalog follow the backend's
[startup binding contract](native-runtime.md#bind-an-exact-package).

For example, a laptop's Wi-Fi drops for two minutes while a test suite and a
dev server run on it. The runner keeps both running and keeps their output.
When it connects again, its hello lists them, and the backend picks them up:
the suite's end and the output both printed meanwhile reach the conversation
as if the connection had never dropped, and the dev server is still serving.

- **A connection loss stops nothing.** The runner keeps its
  jobs, their directories and execution contexts, and its resident services,
  with the conversation state they hold, such as a browser's tabs. A job's
  output goes on into its kept output; the messages the runner would have
  sent about it are not queued, since the next connection reads what it
  needs from the kept output. A declared command a job runs that needs the
  backend, such as an `rpc` call, waits for the connection.
- **The runner notices a loss itself.** It pings the backend every 30
  seconds and counts a connection unanswered for 60 seconds as lost, so a
  connection the network dropped without closing it is lost as a closed one
  is.
- **The hello lists the runner's jobs**: each job's id, whether it runs or
  ended, with its status, each stream's length and its media count, and the
  runner's own instance, a number each start of the runner draws, so the
  backend tells a runner that kept its jobs from one that started anew
  ([Recovery and persistence](sessions-and-targets.md#recovery-and-persistence)).
  A job the backend has no command for is stopped.
- **A refused connection stops them.** When the backend refuses the
  runner, since the device was revoked or removed, the runner stops its jobs,
  as a stop does ([Cancellation and completion](#cancellation-and-completion)),
  and its services; nobody will read them again.
- **A runner that ends loses its jobs**: their shell work runs in its
  process. That happens when it crashes, when it updates itself to another
  release ([Runner updates](#runner-updates)), and when a Cloud stops.

Reconnecting the network does not itself change the backend's command set.

## Pipes and output

A job keeps its output in its directory on the device and sends the backend
views of it in `job_output` messages, each naming its stream and where its
bytes start in the stream (`offset`):

- The first 8 KiB of each stream (`JOB_VIEW_BYTES`), as the runner reads
  them. The model's view of a running command comes from these and from the
  newest bytes below
  ([Results and previews](../agent/shell-tool.md#results-and-previews)).
- While the backend follows the job, the output beyond them: at most one
  message per stream every 250 ms (`JOB_LIVE_INTERVAL`), with the newest bytes
  read since the previous one, at most 16 KiB (`JOB_LIVE_BYTES`, which holds
  4,096 characters of up to four bytes each). A message whose `offset` lies
  beyond the end of the stream's previous one says that the runner left the
  bytes between out. When following starts, each stream that has passed its
  first 8 KiB sends its newest bytes at once.
- While the backend does not follow the job, that a stream grows beyond them,
  with its newest bytes: at most one message per stream every 2 seconds
  (`JOB_GROWTH_INTERVAL`), with the stream's last 8 KiB beyond its first,
  which end at its length; the next message sends them again as far as they
  are still the newest. The model's idle time counts from these messages
  ([Results and previews](../agent/shell-tool.md#results-and-previews)).
- A job's last output leaves before `job_exit`: while followed, what the
  backend does not hold; otherwise each stream's newest bytes up to its end.
- `job_exit` gives each stream's length. When a stream went beyond what the
  backend received, the backend reads the rest from the kept output, below.
  When that read fails, the command's whole output is what the backend
  received: each stream's first bytes, a line that counts the bytes left
  out, and each stream's newest bytes, so the end of a build's log, where it
  says what failed, still shows.

What goes beyond the first 8 KiB never slows the job: a message that finds
the connection's queue full waits for the stream's next interval, and then
carries the newest bytes.

The job's kept output is every read of its stdout and stderr, in the order
the runner read them: one record per read, naming its stream and holding its
bytes, in the runner wire's encoding, so the backend decodes it with the
wire's types. Its bound counts the records' bytes, so it bounds the disk the
output takes exactly: up to 16 MiB of records (`JOB_KEPT_BYTES`), every read
is kept. Beyond that it keeps the first 8 MiB of records whole, a read that
does not fit there split at the bound, and at most the last 8 MiB, in
segments of 1 MiB of which the oldest goes as new reads come, with one record
between the two parts that counts the bytes of output left out. A job that
prints without end, such as `yes`, therefore holds at most 16 MiB of the
device's disk. A record adds about 20 bytes to its read, which the runner
makes up to 64 KiB at a time, so the bound holds nearly 16 MiB of output.
`job_read { jobId }` streams the kept output, as it stands, through a pipe.
The backend reads it when the job ends and a stream went beyond its first
8 KiB, and while the job runs, for `demi shell output`
([The whole output](../agent/shell-tool.md#the-whole-output)).

A job's **media** ([Media the model views](../agent/shell-tool.md#media-the-model-views))
are kept beside its output. They arrive as the medium records of the
invocations of `demi file view`
([Response records and completion](native-runtime.md#response-records-and-completion)),
whatever those invocations' stdout is ([Return media](commands.md#return-media)).
For each medium, the runner:

1. Checks it: at most 16 MiB, from a leaf that declares `media`. A medium
   that fails the check fails its command.
2. Keeps it within the job's bounds, 32 media and 64 MiB, numbering the job's
   media from 1 in the order they arrive, and writes it to `media/<n>` in the
   job's directory. A medium beyond the bounds is read to its end and
   dropped, and its line says it was not kept.
3. Writes the medium's line into the job's output, after what the job's
   output holds when the medium arrives.
4. Sends `job_medium { jobId, number, mediaType, size, sha256 }`, with the
   facts the medium record gave, once the
   medium is written, so the backend knows each medium even when the
   connection is lost before the job ends. Every `job_medium` of a job
   precedes its `job_exit`.

`job_media_read { jobId, number, output }` streams one kept medium through a
pipe. The backend reads, when the job ends, each medium the result attaches
whose blob its owner's namespace does not hold yet.

The runner makes the job's directory when the job starts, private to its
user:

```text
jobs/                               the job root
  edits.lock                        the installation's edit lock (Edit tracking)
  job-<random>/                     one per job
    output/                         the kept output
      head                          its first part
      end-<n>                       the segments of its last part
    media/<n>                       each medium of the job, numbered from 1
    changes/                        what the job's edits recorded
```

A job's directory lasts until the backend has what it needs of the job:

- Once it has read the job's end, the kept output when it needed it, the
  media it did not hold and the edit copies
  ([Edit tracking](edit-tracking.md)), the backend sends
  `job_release { jobId }`, and the runner removes the directory.
- A connection loss keeps every job's directory for the backend's next
  connection ([Command lifetime](#command-lifetime)); one whose job the
  backend has no command for is removed with the job.
- A runner removes every job directory in its job root when it starts: what
  a runner that ended without that cleanup left.

The Host therefore keeps nothing of a command once it has ended: what Demi
keeps of it, the backend has stored
([The whole output](../agent/shell-tool.md#the-whole-output)).

On a paired device the job root is `jobs/` in the installation state, which is
`~/.demi/instances/<backend>/` or the directory `DEMI_HOME` names
([Connection and identity](#connection-and-identity)). A Cloud keeps its
installation state in `/run/demi`, which every boot makes anew, and its job
root at `/var/lib/demi/jobs/` on its system image, since the kept output of a
few busy jobs can outgrow `/run`'s memory
([Images](../cloud/managed-hosts.md#images)).

A job starts unfollowed. `job_follow { jobId, follow }` starts or stops the
following; the backend follows a job while a page shows its conversation
([Live output](../agent/shell-tool.md#live-output)). The model's `demi shell status`
shows the output since its previous look; `demi shell output` prints the whole
output ([The whole output](../agent/shell-tool.md#the-whole-output)).

The runner owns its HTTP pipe transfers; the backend's pipe broker owns their
rendezvous and lifetime. Binary payloads remain bytes. EOF ends input, while
cancellation aborts execution. Live input follows
[explicit command demand](commands.md#deliver-io-and-release-an-invocation), so a
command that never reads stdin does not consume subsequent interactive input.

Live `spawn_stdin` and `job_stdin` frames carry at most 64 KiB of bytes, matching
the [native stdin chunk limit](native-runtime.md#request-body-and-input-demand);
the sender splits larger writes into ordered frames before sending EOF. The
runner holds at most 64 of these frames for a job or process that has not
taken them; one more cancels that job or process, because routing a message
never waits for work that does not read its input.

## Managed guests and verification

On a Cloud, the runner is an ordinary UID 1000 process under init inside a
gVisor/systrap sandbox. It decodes an explicitly selected temporary managed-boot
file with the runner wire's contract types and fails if credentials are missing
or invalid; it never falls back to pairing. Filesystem operations and jobs use
that account. The runner performs no PID 1 boot, host mount, or network setup.
[Managed hosts](../cloud/managed-hosts.md#container-initialization) owns that
responsibility split, boot credentials, and lifecycle.

[Crates and packages](../architecture/crates-and-packages.md#crates) defines
what the runner crate owns and must not do. Tests that exercise a runner start
the built executable through the testing feature of the crate that owns the
backend's end of a runner, and tests that exercise a native service start its
built binary through the command-sdk library's testing feature; both find
the binary the way every test finds a built program
([Module layout](../architecture/crates-and-packages.md#module-layout)). The
[build guide](../delivery/builds-and-releases.md#validation) defines target
execution checks; their results belong to acceptance reports.

Acceptance on a paired device and on a Cloud observes these outcomes:

- A burst of file requests from the web app during a slow manifest install or
  disk sync keeps the connection and every running job.
- Many concurrent jobs running pipelines of embedded utilities all complete;
  none waits for a thread another job's unit holds.
- A connection that never sends its `hello` is closed after 30 seconds.
- A resident service that exits reports its exit status and standard-error
  tail both in the Host log and in the calls it failed.
- A declared command's stdout is `job` when it is the job's stdout pipe,
  directly, through a loop, a group, a subshell, a background list or
  `exec 3>&1` with `>&3`, as a builtin and as an alias that `xargs` starts;
  it is `elsewhere` in a pipeline, under a redirection to a file or
  `/dev/null`, in a command or process substitution, and in a job that
  `demi host shell` started; on Linux, macOS and Windows.
