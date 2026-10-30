---
status: proposed
---

# A command runs on through a runner update, in a shell process of its own

Supersedes: 0068 (A job's `sh` and `bash` are its own shell, and a utility is
Demi's only where it writes files or starts programs), the part that makes a
shell's `$$` unique in the runner: it is unique in its job, and two jobs share
one only by chance, as two runners' shells do now.

## Problem

Alex's agent runs `cargo build && cargo test` on Alex's Mac, which takes ten
minutes. Six minutes in, Alex upgrades the server. The Mac's runner finds that
the backend serves another release and replaces itself: it first sends `TERM`
to every job, then `KILL` after the stop's grace, and starts the new release.
The build dies, the command ends as lost to the upgrade, and the agent starts
the ten minutes over. A dev server another conversation left running dies the
same way and frees its port.

The runner stops the jobs because it cannot hand them over: a job's script
runs in brush, inside the runner's process, so the script's place, its
variables and its directory are gone with the old release. On Windows every
program a job starts is also in a job object that kills its members when the
runner's handle to it closes, which the runner's exit does.

The same design makes one job's failure every job's: a utility that panicked
again while it unwound aborted the runner, and with it every job of every
conversation on the device.

## Decision

A command that runs while its device's runner updates, or after the runner
crashed, runs on. Its output reaches the conversation without a gap once the
new runner connects, and its result is the one it would have had without the
update: `cargo build && cargo test` goes on to `cargo test`. A background task
a command left running, such as a dev server, runs on too, and a later
command's `kill` still reaches it. The device shows offline for the seconds
the update takes, as now.

A command still ends when someone ends it: a stop, a revoked or removed
device, `demi-runner drain` and uninstall stop jobs as they do now, and end
every shell with what an ended job left running in it, as that work ended
with a runner that ran it in its own process. A Cloud's
upgrade restarts the Cloud, which ends its jobs as before; a later record
takes that on.

### A shell process per job

Each job runs in a process of its own, the runner's executable as
`demi-runner shell --job <id>`, which holds brush, Demi's utilities and the
job shell contract's server (`runner-shell`). It runs the release that
started it until its job and what the job left running have ended, so a
runner of another release takes it over.

```text
runner (each release)                 shell process (one per job)
+------------------------------+      +--------------------------------+
| connection, commands, jobs   |      | brush and Demi's utilities     |
| kept output, edit report     |<---->| programs: cargo, tail, ...     |
| job shell contract client    |socket| job shell contract server      |
+------------------------------+      +--------------------------------+
        ^  declared commands (`demi ...`)          |
        +---------- local command endpoint --------+
```

- **Start.** The process binds a private local socket, writes its address on
  its standard output and serves one job. On Unix it forks first and the
  process the runner started ends, as containerd's shim does: the shell is in
  a session of its own and no child of the runner's, so a runner that replaced
  its program in its process never has to reap it, and a service manager's
  stop of the runner's process group misses it. On Windows the runner starts
  it outside its job objects, and the shell owns the job objects of the
  programs it starts.
- **Records.** The runner records how to reach each shell in
  `shells/<job>.json`, beside the installation's job root: the contract
  version, the job and the socket's address. The job's directory keeps the
  job's start in `start.json`: the job, its conversation, where its edits are
  recorded and its execution context (its id, command context, manifest,
  viewable media types and alias directory, which the job's processes name).
- **Output.** The runner keeps the job's output, as now. The shell numbers
  each stream's bytes and holds each chunk until the runner says it kept it,
  so a runner that connects again receives what it did not keep. While no
  runner takes them it holds up to 16 MiB; beyond that it stops reading the
  job's output, which then waits in the programs' pipes, as a terminal's
  paused output does, so a job that prints without end costs no more memory.
- **End.** The shell holds the job's exit until the backend has it and
  releases the job's directory, so a runner that starts again before then
  still receives it, and goes once what the job left running is gone too. A
  shell whose job ended and that no runner reaches for an hour goes. A shell
  that ends without its job's exit takes the job's processes with it, found
  as a stop finds them.
- **Declared commands.** A builtin of a declared command forwards its
  invocation over the runner's local command endpoint, as a command alias
  does, so the runner's dispatcher stays the only one. The endpoint's address
  is one per installation instead of one per start, so a program a job started
  before an update reaches the new runner: `/tmp/demi-<uid>/<name>/ipc.sock`,
  in a directory its user alone may enter, as tmux keeps its sockets in
  `/tmp/tmux-<uid>/`, or the pipe `demi-<name>` on Windows; a system where
  that address cannot be, such as one without a writable `/tmp`, listens at an
  address of its own, as before. A job's command
  waits up to 30 seconds for an endpoint that is gone to come back, so a
  `demi` call made while the runner restarts waits for it.
- **Background tasks.** A shell numbers its tasks from a place it draws at
  random, as a runner does now. A `kill` of a task id the job's shell does not
  have goes to the runner, which asks every other shell; the one that has the
  task signals it.

### The contract on the wire

The job shell contract (`crates/runner-process/src/job_shell.rs`) becomes
frames on the job's socket: a length, a kind and a body, which is a JSON
message, a chunk of output with its stream and offset, or a chunk of input.

| Runner to shell | Shell to runner |
|---|---|
| `hello { contract }` | `hello { contract, release, started, ended, kept offsets }` |
| `start { script, cwd, env, live, commands, edits }` | `started`, `start_failed` |
| input, `input_end` | `input_taken`, `input_closed` |
| `drain`, `signal`, `cancel` | output, `drained`, `outliving`, `output_end` |
| `kept { stdout, stderr }`, `exit_kept` | `exit`, `released`, `log` |
| `signal_task`, `task_signalled` | `signal_task`, `task_signalled` |

- A new message or field is optional, and a side that does not know one
  ignores it, so adding to the contract does not change its version.
- Only a change an older side would misread raises `contract`. The runner
  then keeps the previous version's adapter for as long as a shell of it can
  run, as containerd keeps its bridge to v2 shims.

### Taking over

A runner that starts reads the job root before it removes anything:

1. For each job directory with a `start.json`, it restores the job's
   execution context, connects to the job's shell within 5 seconds and reopens
   the kept output where it stopped, with each stream's length and last bytes
   read back from the files; a count of what went beyond the kept output's
   bound is kept beside it for this, with the first segment kept, so a
   writer that stopped between the two leaves nothing counted twice. The
   job's media keep their numbers, and the next one takes the number after
   them. The next hello lists the job, and the
   backend's path for a listed job applies: it goes on, and output the backend
   missed is read again from the kept output.
2. It connects to every other recorded shell, whose job ended and was
   released, for its log lines and so that a `kill` reaches what it left
   running.
3. A shell that does not answer has its job's processes killed, found by the
   job's id in their environment, as a stop finds them. A shell of a contract
   the runner does not speak is told to cancel its job. Either job is lost as
   today, and its directory and record go.

A runner that updates leaves its jobs: nothing it cancels or drops reaches
them, their input stays open, their contexts' alias directories stay for the
next runner, and on Unix it replaces its program in its own process, as now.

## Spike

- **Every path, on every system.** A job printing a line a second for 40
  seconds, then running `demi host current`, was left by its runner eight
  seconds in, on macOS 27 (Apple silicon), Windows 11 (x64) and Ubuntu (arm64,
  musl build), once by a crash (`kill -9`, `taskkill /F`) and a fresh start,
  once by an update. Each time the new runner took the job over within one
  second of starting, the job printed its 40 lines and its last two, the
  declared command answered, and the job ended with its status. The kept
  output held each byte once: 381 bytes for 381 printed. The update's test
  build replaced the runner with itself on a signal, which the update's code
  does once its download is in place.
- **A background task across an update.** A command left `sleep` running in
  the background and ended; the runner was killed and started again; a later
  command's `kill` of the task's id ended it, and its shell went.
- **Cost of a process per job.** Starting and ending the runner's release
  executable took 3.6 ms on the Mac and 28 ms on the Windows PC (median of 30).
- **macOS privacy grants.** A program signed ad hoc under another identifier,
  started by the runner, read the same protected folders the runner could and
  was refused the same one, with no prompt; so did a program it started, and
  one re-parented to `launchd`. What the system decides once the runner that
  started a shell has crashed was not tried: it needs a runner signed with
  Demi's certificate that the test can end, and is tried on the first signed
  build.
- **Windows.** A child in a kill-on-close job object died with the process
  that held the job's last handle, so the shell starts outside the runner's.
  No step needed more than a user's rights.
- **How often each side changes.** In 60 days, 75 commits changed the shell's
  code: bash and GNU behavior, Windows support, bugs, comments and builds,
  which stay inside the shell. About 8 changed the contract, each by adding to
  it.
- **Brush's state.** Brush's `serde` feature saved and restored a shell's
  directory, variables, arrays, functions, options and aliases. The place a
  script runs at lives in nested `async` functions, which cannot be saved.

## Prior art

- containerd runs each container under a shim. `containerd-shim-runc-v2
  start` binds a socket, starts the shim daemon in its own process group and
  returns the address, which containerd keeps in the bundle's
  `bootstrap.json` with the contract version and protocol
  ([`manager_linux.go`](https://github.com/containerd/containerd/blob/main/cmd/containerd-shim-runc-v2/manager/manager_linux.go)).
  A containerd that starts reads every bundle, connects within 5 seconds and
  cleans up a shim that does not answer
  ([`shim_load.go`](https://github.com/containerd/containerd/blob/main/core/runtime/v2/shim_load.go)).
  It keeps a bridge for shims of the previous task API, v2, beside v3
  ([`bridge.go`](https://github.com/containerd/containerd/blob/main/core/runtime/v2/bridge.go)),
  and its service unit has `KillMode=process`, which Demi's systemd unit has
  too.
- Podman's conmon forks away from Podman, calls `setsid`, logs the container's
  output to a file and writes its exit code to a file in `--exit-dir`, so
  Podman learns the end without having run meanwhile
  ([`conmon.c`](https://github.com/containers/conmon/blob/main/src/conmon.c)).
- tmux keeps its server's socket in `/tmp/tmux-<uid>/`, a directory it checks
  is its user's alone.

Demi departs in three ways:

- The shell is the runner's own executable, as a shim is not: a program of its
  own would be a release artifact for each target, through the build,
  installers, updates and the Cloud's image, and would change with the shared
  crates as often.
- The runner keeps the output, as conmon does not: it already keeps it for the
  backend's views and reads, so the shell holds only what the runner has not
  kept.
- The contract grows by optional fields rather than by versions side by side,
  since a shell outlives at most a few updates.

## Alternatives

- **Stop the jobs and say what did not run.** The program could run on and
  report its end, but the script's remaining steps would not run, and the
  agent would have to run them again.
- **Hand the programs over across the update, the shell in the runner.** The
  runner would keep each program's pipes open across `exec`, or pass their
  handles on Windows, and the new runner would read them. The programs live,
  but the script dies with the shell.
- **Replay the script in the new runner.** Brush would log every result that
  comes from outside the shell, programs, file tests, globs, `$RANDOM`, and
  the new runner would run the script again from its start with the logged
  results until it reaches the running program. It changes brush in about
  twenty to thirty places, and a result the log misses sends the replay down
  another branch without a sign.
- **A shell program of its own, or one the runner carries.** A program of its
  own costs the release and update paths above. Carried inside the runner, it
  would be built before it, which compiles the dependencies a second time.
- **Update once the jobs end.** The device would be offline meanwhile, and a
  dev server never ends.

## Open questions

None.
