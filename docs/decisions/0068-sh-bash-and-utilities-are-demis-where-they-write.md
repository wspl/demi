---
status: proposed
---

# A job's `sh` and `bash` are its own shell, and a utility is Demi's only where it writes files or starts programs

## Problem

On a Mac an agent runs `sh -c "sed -i 's/a/b/' x.txt"`. `sh` is macOS's
`/bin/sh`, bash 3.2, and the `sed` it finds is macOS's: it reads `s/a/b/`
as the backup suffix and fails, and had it changed `x.txt`, the change
would not be among the command's edits, which only the runner's own
utilities record. The same holds wherever a job starts `sh` or `bash`:
Linux's `sh` is dash, Windows' `sh` is Git's MSYS2 one and its `bash` the
WSL launcher, and on macOS and Linux a `#!/bin/bash` script runs the
system's bash too. Meanwhile the runner carries its own copies of
read-only utilities, `cat`, `grep`, `ls`, `sort`, `jq`, `rg` and more,
which Linux and a Mac already have, and on a Mac a model gets GNU's where
it writes macOS's forms.

## Decision

**Two rules decide which utility runs:**

1. **Demi's own, on every system:** the job's shell, `sh` and `bash`, and
   every utility that can write files or start programs, so that what it
   writes, and what the programs it starts write, is recorded as the
   command's edits: `cp`, `mv`, `rm`, `ln`, `mkdir`, `touch`, `chmod`,
   `install`, `tee`, `dd`, `split`, `sort` (`-o`), `sed`, `find`,
   `xargs`, `env`, `timeout`, `nohup` and the like. The files `cp`,
   `touch`, `mktemp` and `install` create are edits too, where until now
   they were not, so `cp -r` records each file it copies, within the
   command's limits on its edits. On macOS each also
   takes the forms macOS's own takes where GNU's refuses them or reads
   them otherwise (`sed -i ''`, `cp -c`, `ln -sfh`, `mktemp -t name`,
   `xargs` running nothing on empty input), so a model that writes macOS's
   forms there gets what it meant. `awk` is the exception: it writes
   files rarely, and stays the system's.
2. **Every other utility is the system's own:** GNU's on Linux, macOS's on
   a Mac, Git for Windows' on Windows. A utility the Host lacks is missing,
   as it is on the machine itself; the model installs it with the
   system's package manager, and Demi does nothing about it.

The table of the runner's utilities lists rule 1's, the same on every
system, and its module's entry comment states the two rules. The read-only
utilities the runner embeds today, `jq` and `rg` among them, leave it.

**`sh` and `bash`** run as the job's own shell on every system, whether
named bare or by a path whose file name is `sh` or `bash` (`/bin/bash -c`,
`/usr/bin/env bash`, a `#!/bin/sh` line, whether the script is the model's
or an installed tool's), and from the utilities that start programs
(`find -exec`, `xargs`, `env`). Each runs as a new shell with its caller's
exported variables, directory and descriptors, so its utilities and its
writes are the job's; `sh` runs in POSIX mode. It takes bash's options:
`-c`, `-s`, a script file, `-e -u -x -v -n -f -l -i`, `-o`, `-O`,
`--posix`, `--login`, `--norc`, `--noprofile`, `--rcfile`, `--version`,
`--help`. Its `$$` is its own, unique in the runner, and its `SHLVL` one
above its caller's. A script without a `#!` line runs as bash runs it. A
program another program starts by its own search, such as Python's
`subprocess` or `npm run`, finds the Host's `sh` as it does today.

**Finding a command.** `which`, `command -v`, `type` and, on Windows, `where`
answer for a name the runner runs itself with the path of the Host's program
of that name, as the system's would, or with the bare name where the Host has
none. A rule-1 utility named by any path runs as the runner's own, as `sh` and
`bash` do, so `"$(which sed)" -i …` runs the runner's `sed`, its edit is
recorded, and `[ -x "$(which sed)" ]` holds.

**Windows.** Git for Windows' `usr\bin`, where its utilities are, stays where
the Host's `PATH` has it, and is added at its end where it has none, so
Windows' own programs, such as `tar` or the `find` a batch file calls, keep
their names. Demi's shell
and its utilities read `/tmp` as Git's do, as the user's temporary folder
(`%TMP%`, else `%TEMP%`), and a drive written as Git writes it, `/c/…`,
as `C:\…`, so a path one of them takes the other takes too. Other paths
Git reads under its own folder, such as `/`, `/usr` and `/etc`, are not
mapped.

The runner finds Git for Windows by its registry entry (`InstallPath` under
`HKCU\Software\GitForWindows`, then `HKLM\SOFTWARE\GitForWindows`), then
in its usual folders, then from `git.exe` on `PATH`, whoever installed it;
a machine-wide one serves every user. It looks when it starts, so a Git
for Windows installed while it runs serves from its next start. When
there is none, the device's setup, which runs in the user's own PowerShell
window, installs it with Git for Windows' own installer, silently:

- on an account in the Administrators group, for every user, without
  asking, behind the one UAC prompt the installer raises;
- on a standard account, as the user chooses when asked: for every user,
  with an administrator's name and password at the UAC prompt, or for this
  user alone in `%LOCALAPPDATA%\Programs\Git`, with no prompt; where the
  window cannot ask, for this user alone.

A device without Git for Windows is not added: when the install fails or is
refused, the setup stops and says why. Demi never updates or removes it.
Git for Windows itself lacks a few utilities, such as `rev`, `hexdump`,
`cal` and `uuidgen`, which are then missing as on any Host that lacks
them.

**What a model reads** of its utilities says rule 1's are Demi's own,
GNU's, with macOS's forms on a Mac, and the others are the system's own,
naming the system. Where a tool's description names a read-only utility
the Host may lack, such as `rg`, it names one every Host has.

Nothing crosses the wire. What a model reads of its utilities is written
by the backend, so a backend of an earlier release with this runner
describes its utilities wrongly until both are upgraded.

## Spike

| Point | What ran | What it showed |
|---|---|---|
| `sh` and `bash` as a new brush shell | a builtin in a brush job shell, built from the caller's context | `-c` with `$0` and arguments, `-e -u -x -n -l`, `--posix`, `-o`, a script with arguments, standard input, exit statuses, traps, nesting, `exec`, pipelines; exported variables in and unexported ones out, `cd` not leaking |
| Its cost | 500 `sh -c true` | 0.63 s, against 5.6 ms for each of macOS's `/bin/sh -c true` |
| macOS forms | each writing utility, macOS's own against uutils', in a job | the forms rule 1 names, among them `mv -h` and `ln -h` printing help and moving nothing |
| Git for Windows' utilities from Demi's shell | a job shell of this design on Windows 11, with Git for Windows 2.46 and 2.56 installed for every user and for one standard user, `usr\bin` first on `PATH` | paths as `C:/Users/…`, `C:\Users\…`, `/c/Users/…` and relative ones; pipes both ways with the embedded utilities over 200,000 lines; exit statuses 1, 2, 7 and 137; UTF-8 in contents, file names and arguments; `env`, `xargs`, `find -exec`, `sh -c` and `which` finding Git's `awk`. About 30 ms to start each of Git's programs, under 1 ms for an embedded one |
| Installed tools' scripts | autoconf configure scripts (jemalloc 5.3, pcre2 10.42, libtool 2.4.7) and libtool's compile, link and install, in the job's own shell against the system's `sh` on macOS and on Linux | the generated Makefiles, headers and `libtool` scripts are the same but for their own paths; what broke on the way, and is fixed, was a `&` delimiter in `sed`'s `s`, an unknown escape in its replacement, a backquote's `\\` and `\"`, `cd ""` and `mv` of a deep relative path |
| The installer, silent | Git for Windows 2.56's installer with `/VERYSILENT /NORESTART /SUPPRESSMSGBOXES` | an administrator's elevated process installs for every user with no prompt in about 20 s; an administrator's unelevated one raises the UAC prompt itself and installs for every user once it is accepted, and exits 2 when it is not; a standard user's fails, `/CURRENTUSER` included, unless `/DIR` names `%LOCALAPPDATA%\Programs\Git`, where it installs with no prompt in about 28 s and writes `HKCU\Software\GitForWindows`; a standard user finds and runs a machine-wide one |

Not yet run: a standard user's install for every user with an
administrator's name and password, and stopping a job while Git's programs
run in it.

## Prior art

- Claude Code on native Windows recommends Git for Windows: with it, its
  Bash tool runs in Git Bash, whose utilities are Git's; without it,
  commands run in PowerShell
  ([setup](https://docs.claude.com/en/docs/claude-code/setup)). Demi runs
  its own shell there and takes Git's read-only utilities.
- Git Bash puts Git's `usr/bin` ahead of Windows' directories in its
  `PATH` (Git for Windows' `/etc/profile`); Demi adds it at the end, so that
  Windows' own `tar` and `find` keep their names.

## Alternatives

- **Embed every utility a system lacks or may lack**, as this record first
  said: about 60 more utilities, some 32,000 lines, a table per system to
  keep true, and still not every tool a model reaches for, such as `awk`.
- **The system's `sh`, `bash` and writing utilities.** Their writes go
  unrecorded.
- **Demi's read-only utilities on Windows instead of Git's.** Git for
  Windows is on nearly every Windows machine a coding agent works on, and
  carries more than Demi would embed.

## Open questions

- `whereis`, which lists a name's files and manual pages rather than what
  runs, is proposed to stay the system's.
