---
status: proposed
---

# On Windows, `>>` gives a program its file opened for writing at its end

Supersedes, in part: 0065 (a program writes its redirected file itself),
where on Windows `>>` still passes through the pipe.

## Problem

On Windows a `>>` file passes through the runner's pipe, because a file
opened for appending has only append access, which programs of Git for
Windows' MSYS2 runtime (`sh`, `awk`, `perl` and the rest of Git's
`usr\bin`) refuse: they take a handle without write access for one they
cannot write. Two cases go wrong:

- `sh -c 'sleep 8 & echo hi' >> f; echo done` prints `done` 8 seconds late,
  since `sleep` holds the pipe; on macOS and Linux it prints at once.
- A background job gets its file itself (0061), so
  `sh -c 'echo hi' >> log &` writes nothing and fails with
  "Bad file descriptor".

## Decision

On Windows the runner opens a `>>` file for writing, moves to its end and
gives it to the program, in the foreground and in a background job alike,
as on macOS and Linux it gives the file itself; no redirection passes
through a pipe any more. A foreground program's edit is recorded as 0065
says, from the file as it is handed over to the file as the program leaves
it.

Only a program gets such a file; the shell's own writes and the embedded
utilities' (`echo … >> f`, `tee -a`) still open it for appending, which the
runner's own code can use.

Each write of the program, and of the children it leaves holding the file,
lands where its own last write ended, not at the file's end: what anything
else appends to the file meanwhile can be overwritten.
`sh -c '(sleep 2; echo late) & echo hi' >> f; echo done >> f` leaves `hi`
and `late` in `f`, where bash on macOS or Linux leaves `hi`, `done` and
`late`, since `O_APPEND` puts every write at the end; the command's edits
still show `done`, written before `late` overwrote it. A server appending
to its log for hours does the same to a line another command appends, and
after `: > log` empties the log, the server's next write lands at its old
place, leaving the file that many zero bytes and then the new line, where
`O_APPEND` writes from the start. 0065 refused this file for losing
appending's atomicity; that cost is taken here, since the pipe 0065 kept
holds a command and fails a background job's MSYS2 program.

The file is opened again from the program's own handle (`ReOpenFile`), not
from its path, so any `>>` a program gets is opened at its end, whatever
renamed the file, and in a job that records no edits or after its command
ended. A program whose standard output and standard error go to one file
(`>> f 2>&1`, `>> f 2>> f`) gets one handle for both, so the two streams
never overwrite each other.

## Spike

On Windows (Git for Windows 2.46, MSYS2 runtime 3.4.10), a file opened for
appending and for writing at its end was handed to 23 programs as their
output:

| Program | Append access | Write access at the end |
|---|---|---|
| MSYS2 programs (`sh`, `echo`) | exit 1, nothing written | written |
| MSYS2 programs (`sed --version`, `ssh -V`) | exit 0, nothing written | written |
| Git's launchers `bin\sh.exe`, `bin\bash.exe` | exit 1, nothing written | written |
| Native programs (git, python, node, cmd, powershell, curl) | written | written |
| Native programs that start an MSYS2 one (python's and node's `sh`, a git alias) | exit 0 or 1, nothing written | written |

An MSYS2 program is known from its executable (it loads `msys-2.0.dll`),
and Git's launchers from their place beside Git's `usr\bin`; a native
program that starts one later cannot be known, so append access cannot go
to the programs that would use it safely.

## Prior art

Cygwin, on which MSYS2 is built, opens a file for `O_WRONLY | O_APPEND`
with write access and appends by writing each block with `NtWriteFile`'s
`FILE_WRITE_TO_END_OF_FILE` offset
([`fhandler_base::open` and `raw_write`](https://github.com/cygwin/cygwin/blob/main/winsup/cygwin/fhandler/base.cc));
the `O_APPEND` flag passes only to the Cygwin processes it starts, and a
native program it starts gets the handle with write access. Git Bash
therefore gives a native program's `>>` the same file Demi now gives. It
departs for an MSYS2 program: Git Bash hands it `O_APPEND`, so its writes
land at the end, which a native parent such as the runner cannot do. The
runtime takes a standard handle as writable only by its write access
(`init_std_file_from_handle` in Cygwin's
[`dtable.cc`](https://github.com/cygwin/cygwin/blob/main/winsup/cygwin/dtable.cc)).

## Alternatives

- **Keep the pipe for `>>` on Windows** (0065). A program's background child
  that holds the file holds the command, and a background job's `>>` fails
  in MSYS2 programs.
- **Append access for programs known not to be MSYS2 ones.** Native
  programs that start an MSYS2 one (git hooks and aliases, npm scripts,
  python's and node's subprocesses) would lose their output without an
  error.
- **Have the MSYS2 runtime take an append-only handle as `O_WRONLY |
  O_APPEND`**, upstream in Cygwin. Every program could then get an
  append-only file and nothing would be overwritten; it waits on Cygwin,
  MSYS2 and Git for Windows releasing it, and on every Host updating Git.

## Open questions

None.
