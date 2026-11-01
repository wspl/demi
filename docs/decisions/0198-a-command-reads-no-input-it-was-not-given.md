---
status: proposed
---

# A command reads no input it was not given

## Problem

An agent searches a repository for a word with `rg -l changeset`. ripgrep,
given no path, searches its stdin when stdin is a pipe or a file, and every
command of the `shell` tool has a pipe for stdin that stays open until the
command ends, so that `demi shell input` can write to it. Nothing writes, so
`rg` waits for the call's window to pass, moves to the background and never
ends. `tail`, `cat` and `wc` without a file, and `python3 -` without a
heredoc, wait the same way.

An agent that met this once stops using `rg` and searches with `grep -r .`,
which does not skip what `.gitignore` names. In a slot that holds 15 GB of
`target/` and 22 GB of `.cache/`, a search `git grep` answers in 0.08
seconds takes 11 to 21 seconds, or runs for minutes until it is stopped.
`demi shell input`, which the open pipe is for, is almost never used: a
prompt that needs an answer mostly needs a terminal too, and a pipe does not
give one.

## Decision

A command's stdin is empty: it reads the end of its input at once, as it
would from `/dev/null`. `rg pattern` searches the working directory, and
`tail` without a file prints nothing and exits. `demi shell input` is
removed, and with it the only reason to keep a pipe open.

- A heredoc, a pipe or a redirection in the script is the script's own and
  unchanged: `cat <<'EOF'` reads the heredoc.
- A command whose stdin the backend relays from another command, as
  `tar c . | demi host shell --host ci "tar x"` relays it, keeps the
  relayed stream. A caller with no input passes none on:
  `demi host shell --host ci "rg marker"` searches the far directory, as
  it does locally, instead of an empty stdin.
- A program that would ask a question takes its answer from the script, as
  in CI: a flag such as `--yes`, a heredoc, or `yes |`.
- The model's `demi shell` group lists status, output and stop.

### How it works

What gives a job's input to its commands goes, and nothing replaces it:

- The runner opens `/dev/null` (`NUL` on Windows) as the stdin of a job
  whose `TaskCommand::Shell` has no relayed `stdin`, instead of a pipe; the
  pipe's writer, the copy of its reading end and `DEMI_LIVE_INPUT` go.
- `stdio::is_live`, the invocation's `live_input`, the `live` flag of a
  command's request and the forwarding of a caller's live stdin to
  `demi host shell`'s far job go: no stdin is a job's own input any more.
- A command line a job's command forwards to the runner (`RawCommand`)
  says whether its stdin is anything but the null device (`stdin`), in
  place of `live`. A declared command without input then neither reads a
  body nor opens a pipe to the backend, so the far job of
  `demi host shell` gets the null device too.
- `demi file view` with no path reads stdin as it reads a pipe; an empty
  one says it was given no medium. Its refusal to wait for a job's input
  goes.
- `demi shell input`'s declaration and its RPC go. A raw process the
  backend starts (`TaskCommand::Process`) keeps its own input.

### What an older runner or page meets

- An older runner keeps the open pipe until it updates; the backend no
  longer offers `demi shell input`, so nothing writes to it.
- A command that runs across the update keeps the stdin it started with.
  A job's shell that outlived its runner still sends `live`; `live: true`
  reads as no input, so its declared commands do not wait on a pipe that
  ends only with the job.

## Spike

- **The wait.** In a slot, `rg -l -i changeset` with no path was stopped by
  `timeout 8` after 8 seconds; the same with `.` as its path, or with
  `</dev/null`, answered in 0 seconds. Python's `os.fstat(0)` in a command
  says its stdin is a FIFO.
- **The cost of `grep -r`.** In the same slot, `grep -rln` from `.` with
  `node_modules` and `target` excluded ran for more than 90 seconds; with
  `.cache` excluded too, 2 seconds; `git grep -l`, 0.08 seconds.

## Prior art

- [Codex](https://github.com/openai/codex/blob/main/codex-rs/core/src/spawn.rs)
  starts a shell tool's command with `Stdio::null()`, "because otherwise
  some commands may hang forever waiting for input", naming ripgrep. Its
  `exec_command` keeps stdin open only with `tty=true`.
- Claude Code's Bash tool has no way to write to a command's stdin.
- `docker run` gives a container no stdin unless `-i` asks for it.

Demi takes Claude Code's: no input, and so no field that asks for it.

## Alternatives

- **Keep `demi shell input` behind a field of the `shell` tool**, such as
  `input: true`, as Codex's `tty`. It keeps a REPL drivable, at the price
  of a field every call's schema carries for a command almost never used.
- **Tell the model to add `</dev/null` or a path.** A rule for one
  symptom in the prompt, which every model has to remember for every
  program that reads stdin.

## Open questions

None.
