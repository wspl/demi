---
status: proposed, superseded in part by 0066
---

# A foreground program writes its redirected file itself, and its edit is read when it exits

## Problem

An agent runs `sh -c 'sleep 8 & echo hi' > f; echo done`. Bash prints `done`
at once; Demi prints it 8 seconds later. A program's output to a regular
file passes through a pipe the runner reads and writes into the file, so
that each write is recorded as the command's edit, and the pipe ends only
when every process holding it has closed it: `sleep`, which `sh` started,
holds it for 8 seconds.

The same pipe loses edits and time. Each 64 KiB the program writes saves the
whole file again as an edit's after side, so a program that writes 3 MiB or
more takes the job's 64 MiB of edit copies and its edit shows no diff; on
Windows, where the pipe reads in smaller pieces, 1 MiB is enough, and 8 MiB
takes 6.3 seconds.

## Decision

A foreground program whose standard output or standard error is redirected
to a regular file gets the file itself, as bash's programs do. The runner
reads the file's before side as it hands the file over, and its after side
once, when the program exits; that is the program's edit, which joins an
earlier edit of the command by today's rule (an edit continues the one
before it only when that one's after side is this one's before side).

```rust
// command-sdk
impl Recorder {
    /// Reads the before side of `path`, which a program will write itself.
    pub fn hold(&self, path: &Path) -> io::Result<Held>;
}
impl Held {
    /// Records the edit from the before side to the file as it is now,
    /// once the program that wrote it has exited.
    pub fn settle(self) -> io::Result<()>;
}
```

- **What a user sees:** `done` at once; the command still runs until
  nothing holds its own output, as 0061 says, so `sleep` holding the
  command's standard error holds the command. A program's edit shows its
  diff up to the recorder's size limit, 8 MiB, where it lost it from 3 MiB.
- **Two commands writing one file at once:** each command's edit is the
  file from when its program opened it to when it exited, so two programs
  whose times overlap each count the other's writes within that time as
  their own. Today each write was recorded apart.
- **A program's own background children** that write the file after the
  program exits write into it, and their writes are not the command's
  edit, as a detached job's are not (0061).
- **A program that replaces its file**, by a rename over it or by removing
  and creating it, has the file as it ends recorded, where the pipe
  recorded the writes to the file it first opened.
- **On Windows, `>>` still passes through the pipe.** A file opened for
  appending has no write access, which MSYS programs such as Git Bash's
  refuse ("Bad file descriptor"); opening it for writing and seeking to its
  end loses appending's atomicity, which lost a line when two commands
  appended at once. `>` and `2>` go to the file itself on Windows too.
  So on Windows a program's background children that hold a `>>` file keep
  the command waiting, as every redirection did before:
  `sh -c 'sleep 8 & echo hi' >> f; echo done` prints `done` after 8 seconds.
- **The same file opened twice in one command** (`> f 2> f`) is recorded as
  two edits of it, the second repeating the program's whole change.
- A background job's programs write their files themselves already (0061),
  unrecorded; nothing changes for them.

## Spike

A prototype on macOS and on Windows (msvc), against today's pipe in the
same build:

| Case | Today | This design |
|---|---|---|
| `sh -c 'sleep 8 & echo hi' > f; echo done`, `done` printed (macOS) | 8.03 s | 0.02 s |
| `> f`, `>> f`, `2> f`, `> f 2>&1`, `> f 2> g` and the other forms, the edit recorded (macOS) | as before | the same, byte for byte |
| `> f 2> f`, the edit recorded (macOS, Windows) | one edit | one more, repeating the whole change |
| 8 MiB written by a program, job time and edit (macOS) | 0.28 s, no diff | 0.06 s, diff kept |
| 8 MiB written by a program (Windows) | 6.30 s, no diff | 0.19 s, diff kept |
| `sh -c 'echo hi' >> f` (Windows, MSYS `sh`) | works | "Bad file descriptor" |
| Two commands appending to one file, times nested | each its own writes | the outer one counts the inner one's |

On Windows a program's background children end when the program exits,
against 0061, since the process library's wait on a Windows job returns at
the job's first event rather than when its last process exits; that is a
defect, fixed apart from this decision, and with it the Windows `>>` wait
above appears.

## Prior art

POSIX shells open a redirection's file and give the program the descriptor
itself
([Shell Command Language, Redirection](https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html#tag_19_07)).
Demi does the same, and reads the file around the program to record its
edit.

## Alternatives

- **Keep the pipe and stop waiting for it when the program exits.** The
  children still holding the pipe would lose their writes when the runner
  stops reading, or the runner would read on with no command to record for.
- **Intercept the program's writes** (`ptrace`, `LD_PRELOAD`). Not on every
  system, not for a statically linked program, and much more code.
- **Reopen `>>` for writing on Windows.** It loses appending's atomicity.

## Open questions

None.
