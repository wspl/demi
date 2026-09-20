# Demi Next: The Claude Code CLI

The Claude Code provider talks to the vendor through the vendor's CLI
([Claude Code execution boundary](providers-and-vault.md#claude-code-execution-boundary)).
Demi installs and updates its own copy of that CLI on the machines that run it.
It never uses a `claude` found on `PATH`, a copy the machine's owner installed,
or a system package: what runs is a version Demi chose and verified.

The CLI is started with no tools, no session persistence and no device
configuration, so it is a channel to the vendor and nothing else. It does not
touch the workspace, which is why **where it runs is a choice**, not a
consequence of where the conversation's files are.

## Which version

The version Demi wants is the vendor's newest: the `latest` pointer of the
official distribution (`https://downloads.claude.ai/claude-code-releases`).
Nothing stores a wanted version. The backend reads the pointer and that
version's `manifest.json`, which gives each platform's byte size and SHA-256,
and keeps the answer for a while (hours, not requests); **Check for updates**
reads it again at once.

An entry can be **held** at one version instead, for the day a new CLI breaks
what Demi expects of it. Holding is the only version a provider entry stores,
and it is the exception: a held entry says so wherever its version is shown.
The provider package names the newest version it was tested with; a newer one
in use is worth a note in settings and a line in a failure report, never a
refusal.

What is **installed** is a fact of each machine and is asked of that machine
when someone wants to know. The backend keeps no table of it. A machine that is
offline or asleep has no answer, and showing a version is never a reason to
wake one.

A newer version does not interrupt work. A machine that already has a usable
version keeps answering with it while the newer one installs beside it; the
next CLI process after the install is the newer one. Only a machine with no
usable version waits for an install. Installing a version removes the others:
a process already running one keeps its executable where the system allows a
file in use to be removed, and where it does not, the removal is left for a
later install.

## The package

`demi.claude` is a native package of its own
([Bind an exact package](native-runtime.md#bind-an-exact-package)), not part of
`demi.builtin`: the runner knows nothing about Claude Code, and removing the
package and the backend's use of it leaves the rest of Demi as it was. It has
two operations:

| Operation | Input | Answer |
|---|---|---|
| `claude.ensure` | A release record: `{ version, platforms }`, each platform's official `url`, byte `size` and `sha256` | `{ version, path }`: the executable's absolute path |
| `claude.status` | — | `{ platform, installed }`: this machine's platform key and the versions it has, newest first |

The backend reaches them through a [service stream](runner.md#service-streams),
which carries no arguments: the record is the stream's input, read to its end,
and the answer is one JSON document on its output, since the output is all a
service stream brings back: `{ ok: true, … }`, or `{ ok: false, code, message }`
with one of `invalid_release`, `unsupported_platform`, `download_failed`,
`verification_failed`, `install_failed`. An output with no document is a
service that failed. The command context names the
conversation being served, or the provider entry for work that belongs to none.

The backend builds the release record from the manifest; the package does not
read pointers or choose versions. It picks the platform from the machine, not
from the runner's own target: a runner built for musl runs on glibc machines,
and the CLI's `linux-*` and `linux-*-musl` builds are not interchangeable. A
platform the release does not carry is `unsupported_platform`, never another
platform's build.

`claude.ensure` installs under the Host user's `.demi/claude/<version>/`; a
Cloud image may carry a version under `/opt/demi/claude/<version>/`, which is
used when it is the wanted one. It downloads to a temporary file, enforces the
size, verifies the digest, and only then publishes the executable atomically.
Callers asking for the same version share one download. A file that fails
verification is removed and the install fails; there is no fallback to another
version, location or executable.

The backend starts the CLI through the ordinary Host process interface with the
path `claude.ensure` answered, the account's token, a configuration home of
Demi's, and the CLI's own updater turned off. Demi is the only thing that
changes the version.

## Where it runs

Two kinds of work need the CLI, and a **placement** decides the machine for
each. Placement is one rule in one place: what calls the CLI asks it for a
Host and does not know why it got that one.

| Work | Example | Placement today |
|---|---|---|
| Account work: not part of any conversation | Installing after an account is added; **Test connection** | The acting user's Cloud |
| Inference | A conversation's request | The conversation's execution target |

A placement answers with candidates in order, and the work takes the first that
is available. Today each list has one entry. The shape is what lets this
change without touching the callers: inference that falls back to Cloud when
its target has no usable CLI, or everything on Cloud, are each a different
list. Cloud is always there to be a candidate.

Account work runs on Cloud because it is the machine Demi owns, and every
deployment has one ([Managed Cloud hosts](managed-hosts.md)): waking it for a
test surprises nobody, and a CLI installed there right after the account is
added is already warm when the first Cloud conversation asks. Usage and plan
need no CLI and no machine at all: the backend asks the vendor itself
([Provider quota](../provider-quota.md)).

On a shared instance the acting user is the master, so account work runs on the
master's Cloud and inference on each user's own target.

## What the user sees

Installing is not an event of the conversation. A request whose machine has no
usable CLI installs one and then proceeds, under the same **Requesting** the
request would show anyway; an update installs with nothing shown at all.

A failure is always shown, with its reason and the version: the release could
not be read, the download failed or did not match its digest, the platform has
no build, the disk is full, the executable did not start. In a conversation it
is that request's error, and the next request tries again. For account work it
is the result of the action that asked, and an install that failed after an
account was added does not undo the account: the settings page shows the
account as added and the CLI as not installed, with **Install** to try again.

Settings show, for a Claude Code entry: the vendor's newest version, whether
the entry is held, and for each machine that can be asked now, the version it
has.

## Implementation limits

None of this is implemented. The CLI is whatever `claude` the execution target
has on `PATH`; nothing installs, verifies, reports or updates it, and
**Test connection** is refused for this provider. The release scripts build and
publish one crate (`demi-commands`); carrying a second package needs them to
take the crate as a parameter. The runtime catalog, `service_open` and
`DEMI_NATIVE_CONFIG.releases` already handle several packages.
