---
status: proposed
---

# A script ends as bash ends it, and what it leaves behind runs on

Supersedes: the background tasks of a shell job (`crates/runner-shell/src/work.rs`), where `disown` and `nohup` keep a task in its job and a job runs on until every task it started has ended.

## Problem

An agent starts a long measurement in the background and goes on:

```sh
(for n in 1 2 3; do ./bench $n; done) > out/bench.txt 2>&1 &
echo started
```

In bash the script prints `started` and exits at once; the loop runs on, its
output in `out/bench.txt`. In Demi the call does not return: the job counts
the loop as its own work and runs until the loop ends, so after 30 seconds
the call moves to the background. The agent, expecting bash's behavior,
believes the loop detached and nothing will tell it when it ends, and writes
its own waits (`sleep 240; cat out/bench.txt`, `while pgrep bench; do sleep
5; done`), each a call that wakes it again. `nohup` and `disown`, the usual
ways to say "let this outlive the script", change nothing.

## Decision

A command's script behaves as bash's does when its output goes to a pipe,
which is how every program that captures a script's output runs it:

- **The command ends** when its script has exited and nothing it started
  still holds the command's output (stdout or stderr) open, as a reader of a
  pipe sees its end. A background job holds it unless its own redirections
  send both elsewhere: `cmd > log 2>&1 &` does not hold it; `cmd &`,
  `cmd > log &` (stderr still on it), `a | b > log &`, `cd app && cmd > log
  2>&1 &` and `(a > f; b > f) &` do, in bash as in Demi, because the job's
  shell keeps the descriptor. Whether a job holds it is read from its
  redirections as written, opening nothing; the job opens them itself as it
  starts, as bash's forked child does, so opening a FIFO blocks the job
  alone. A here-document, a process substitution or a redirection whose
  target is expanded leaves the job holding the output.
- **What the script left running runs on** once the command has ended,
  detached: it belongs to no command, wakes no agent, and ends when it ends
  or is killed. A program the script started keeps its process ID, so `ps`,
  `pgrep` and `kill` of a later command reach it. A background job that is
  shell work inside the runner, a loop or a subshell, has no process ID; its
  `$!` is an ID unique on the runner, which `kill` of any later command on
  that Host reaches, as bash's `kill` reaches a subshell's process. The ID
  is gone once its job ends, as an ended process's ID is.
- **A program's own background children** run on when the program exits, as
  in bash: `./start.sh` that runs `server &` and exits leaves the server
  running, held by the command while the command runs and detached with it.
  Today the runner kills a process group when its leader exits.
- **A detached job keeps what it runs with.** Its environment, working
  directory and the command's context stay until the last of it ends, so a
  `demi` command it runs still works. Its file writes are not recorded as
  the command's edits, before the command ends as after: a program of a
  background job writes its redirected files itself, not through the
  runner, so it outlives the runner. The backend keeps
  the job's origin, which a `demi` command's call is admitted by, after the
  job's exit, until the runner says the context is released:

  ```rust
  enum Outbound {
      // ...
      /// The context of a job that declared commands is released: sent after
      /// its `JobExit`, on the job's own queue, at once when nothing detached
      /// holds the context, else when the last detached work ends.
      JobContextEnded { job_id: String },
  }
  ```

  The backend drops the origin on it, and when it loses the device or hears
  from a new instance of its runner; a connection lost and made again to the
  same runner keeps it. A backend restart loses the origins it kept, so a
  `demi` command in detached work fails after it. The wire version goes up by
  one.
- **When the runner stops**, its shell work stops with it. A program a
  detached job started runs on, as a program a closed bash session left
  does: the runner's systemd unit on Linux stops the runner's process alone
  (`KillMode=process`), not its control group. On Windows the runner's job
  object ends it.
- **A stopped command ends with its work.** A Stop, by `TERM` or `KILL`,
  detaches nothing: the command ends once every process group it started,
  its background jobs' included, has ended, at most `KILLED_WORK` after
  `KILL`.
- **A background task that holds the command's output** holds the command,
  as a background job writing to a pipe holds the pipe in bash: the command
  runs until the task lets go, moves to the background after 30 seconds,
  and its end wakes the agent, as any command's does.
- **`nohup`** is one of Demi's own utilities on every Host, Windows
  included, as GNU's is, so a script does not depend on a Host having one:
  it runs its command with `SIGHUP` ignored where the system has signals,
  and, the command's output never being a terminal, writes no `nohup.out`.
- **`disown`** removes a job from the script's job table, so `wait` with no
  argument and `jobs` no longer see it; `disown -h` marks it and keeps it,
  and `-r` takes the running jobs alone, as bash's do. It changes nothing
  else.

The examples become:

```text
cmd > log 2>&1 & echo started      returns at once; cmd runs on, detached
nohup cmd > log 2>&1 &             the same, cmd also ignores SIGHUP
cmd & echo started                 runs until cmd ends (cmd holds the output)
cd app && cmd > log 2>&1 &         the same: the job's shell holds it
(cd app && cmd) > log 2>&1 &       returns at once
```

Nothing changes for a script with no `&`, for `wait`, `kill` and `$!`
within a script, or for `service: true`.

A runner of an earlier version keeps today's behavior until it is replaced,
which every upgrade does.

## Spike

A prototype of about 170 lines on the embedded shell, run on macOS, Linux
and Windows against bash 3.2 and 5.2 reading the script's output through two
pipes:

1. **Which jobs hold the command.** On 15 scripts the prototype and both
   bashes agreed case by case: `cmd > f 2>&1 &`, `cmd &> f &`, `{ …; } > f
   2>&1 &`, `(…) > f 2>&1 &` and `nohup cmd >/dev/null 2>&1 &` return in
   about 0.01 s; `cmd &`, `cmd > f &`, `cmd 2> f &`, `a | b > f &`, `cd x &&
   cmd > f 2>&1 &` and `(a > f; b > f) &` are held until the job ends. It
   needed the shell to apply a background job's redirections before the job
   runs; without that, `cmd > f 2>&1 &` was held 3 s.
2. **A job outlives its command.** After the command ended, a detached loop
   kept writing its file, and its programs kept running, on all three
   systems; a later command's `pgrep` and `kill <pid>` reached them. The
   `nohup` it used was the system's: macOS's, and on Windows Git's, which a
   Host without Git does not have. A
   command's Stop before its end killed its jobs and their programs; after
   its end it touched none. Found and now in the design: a loop had no ID a
   later command could `kill`; its writes went on into the command's edit
   record; the command's context was removed at its end; a program's own `&`
   child was killed with its group.
3. **Cost.** A detached program costs one thread and about 0.17 MB, a
   detached subshell two threads, 33 descriptors and 0.4 to 0.6 MB, as they
   cost today inside their command; once they end nothing of them is left.

## Prior art

- **bash** with its output to a pipe: a script exits when its last
  foreground command does; a reader of the pipe sees its end when every
  process holding the pipe has exited or closed it, so a background job that
  writes to the pipe holds the reader and one redirected elsewhere does not.
  `nohup` ignores `SIGHUP` and redirects only a terminal; `disown` edits the
  job table.
- **Codex CLI** ends a command at most 2 seconds after its shell exits, so a
  grandchild that inherited stdout cannot hold it
  ([exec.rs](https://github.com/openai/codex/blob/main/codex-rs/core/src/exec.rs)).
  Demi keeps bash's rule instead: `cmd &` holds the command until it lets go
  of the output, and moves to the background after 30 seconds.
- Demi departs only where it has no terminal: there is no `SIGHUP` on a
  command's end to survive, since a command's end sends none.

## Alternatives

- **Keep a job until all its work ends (today).** The script's call never
  returns while anything it started runs, which no bash user expects, and
  the model works around it by polling.
- **Detach, but wake the agent when detached work ends.** More than bash
  does; it keeps every detached task tied to an agent that may have moved
  on, and a model never expects it.
- **Keep detached work under the command and list it.** The command would
  stay "running" after its script ended, which is today's behavior under
  another name.

## Open questions

None.
