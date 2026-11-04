---
status: accepted
---

# A command runs in the background when asked

## Problem

Alex asks the agent to start the release build, which takes 15 minutes,
and to look through the code meanwhile. A `shell` call holds until its
command ends, up to 10 minutes, so a call that runs the build keeps the
agent from looking at anything else. The agent starts the build with
`nohup ./build.sh release > build.log 2>&1 &` instead. The call returns at
once, but nothing tells the agent when that process ends: the guideline
forbids waiting with sleep or a loop, so it creates a schedule that greps
`build.log` every few minutes and wakes it on `BUILD-DONE`.

Alex then sees a schedule in the conversation for a build the agent ran
itself. The work panel shows no build running, Stop cannot stop it, and its
output is in a file only the agent knows. The build's end reaches the agent
late, by the schedule's interval, or never, when the log says something the
grep did not expect.

In a test of the shell tool as it is, with four tasks that each start a
command of 12 to 25 minutes and ask for other work meanwhile, Claude Opus
detached the command with `&` or `nohup` in 17 runs of 20, and ran it in
the call in the other 3.

## Decision

**`shell` takes `background: true`: the call returns at once, with the
command running, and the command's end wakes the agent, as the end of a
command that moved to the background does.** The command is the
conversation's: the work panel shows it running, its own Stop there stops
it, and `demi shell output` reads it. Like any command in the background,
it keeps running when the user stops the agent's turn.

The model reads, in the tool's description:

> background: true starts a command you do not wait for, such as a long
> build: the call returns at once, and its end wakes you.

and in the schema, for `background`, as `service` has its own line:

> A command you do not wait for, such as a long build.

The call returns before the command has printed anything, so its result
says only that it runs:

> [Command 12 runs in the background, as asked. Its end wakes you.]

A script that fails at once tells the agent as any command in the
background does, when it ends. The result leaves out the other background
lines' "Do not wait for it": the agent asked not to wait.

A `background` call returns at once, so it counts as a call of the step
that returned: another call of the same response that runs for longer than
10 seconds then moves to the background too, as it does beside any quick
call.

### Design

- `agent-tools` `ShellInput` gains `background: bool`, default `false`. A
  call reads it with `service` as one `Hold`, how long the call holds:
  `UntilEnd`, `Background` or `Service`. A call that asks for both is
  refused as an invalid input, and runs nothing.
- A `Background` call starts the command, takes its status at once and
  returns. A command running then is one that moved to the background,
  `Moved::Asked`: nothing stored, reported or shown differs from a command
  whose call's window passed.
- The log line `a shell call returned` says `moved = "asked"`.

## Spike

The same four tasks, five runs each, with the tool as decided: Claude Opus
started the command with `background: true` in all 20, and created no
schedule. Two tasks whose command takes 20 seconds or less, five runs
each, used no `background` in either version. GPT was not tested.

## Prior art

- Claude Code's Bash tool takes `run_in_background`: the call returns at
  once, and the model is told when the command completes, so it does not
  poll ([background bash commands](https://code.claude.com/docs/en/interactive-mode)).
  The models Demi runs are trained on it, which is why they reach for a
  background of their own when the tool has none.
- Codex's `exec_command` returns after `yield_time_ms` with the session
  still running, and the model polls it with `write_stdin`
  ([unified exec](https://github.com/openai/codex/blob/main/codex-rs/core/src/unified_exec/mod.rs)).

## Alternatives

- **Say in `service`'s description that it suits a long build.** It already
  returns early and wakes the agent at the end, but a build is not a
  service, and one name would stand for two things.
- **Tell the model that another call of the same response moves the
  command to the background after 10 seconds.** It names a mechanism the
  model would plan around, and a response with nothing else to run cannot
  use it.
- **Say in the schedule's description that it is not for the
  conversation's own commands.** It corrects the symptom, and the model
  still has no way to run a command beside its work.

## Open questions

None.
