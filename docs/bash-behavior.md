# Bash / just-bash behavior

The oracle for shell behavior is **GNU bash on Linux** (builtins, expansion,
job control) plus the **real PATH binaries** it would exec (GNU coreutils and
whatever else is on `PATH`). Models diagnose from that Unix. A Demi shell that
invents a different exception path produces self-consistent false evidence
(`ls -l` vs `stat` vs `test -x` vs spawn stderr) and the model follows it.

**Do not fill a bash-column cell from memory.** Re-run GNU bash
(`bash --norc --noprofile`) or extend
`packages/shell/src/__tests__/bash-oracle.test.ts`, which is the living
oracle for every bash fact in this document. When that test and this
document disagree, the test (the measured process) wins; update the table
to match it.

When Demi code and this document disagree, the code is wrong: change the
code, or narrow this document to an allowed difference listed in [Allowed
differences](#allowed-differences). Do not invent a third behavior to “make
the agent more robust.”

Related: [just-bash fork policy](./just-bash-fork-policy.md),
[Implement a Host](./guides/implement-a-host.md),
[package boundaries](./package-boundaries.md) (`just-bash`, `@demicodes/shell`).

## Surfaces

| Surface | Code | What it is |
|---|---|---|
| just-bash | `Bash` + `InMemoryFs`, no `hostSpawn` | In-process virtual machine (browser / unit tests). No kernel, no real `PATH`. Identity and devices are virtual by design. |
| Host-backed shell | `@demicodes/shell` `BashEnvironment` + `Host.fs` / `Host.process.spawn` | Persistent shell on a real computer (local, container, remote). This is what a coding agent runs. **It is required to match bash.** Portable commands exist only so a Host without coreutils still has `cat`/`ls`; they are not a second Unix. |

just-bash comparison tests (`packages/just-bash/.../src/comparison-tests/`)
record real bash fixtures for parser, builtins, and portable commands against
an in-memory fs. They do not cover `hostSpawn`, spawn `cwd`, or
`preferHostSpawn` fallback. Host-backed behavior is owned by `@demicodes/shell`.
The bash column of the tables below is asserted by
`packages/shell/src/__tests__/bash-oracle.test.ts` against `bash --norc
--noprofile` (no aliases, `LC_ALL=C`).

## Dispatch

bash (non-POSIX): aliases (interactive only) → functions → builtins → `PATH`
(then 127). Functions override builtins, including `cd` and `echo`. A name
that is not a builtin (`ls`, `chmod`, `stat`, `find`, `whoami`) is the file
`PATH` finds (`type -t` is `file`). In POSIX mode, defining a function with
the name of a special builtin (`export`, …) is an error and the shell exits
before later commands.

`type -t pwd` / `echo` / `printf` / `test` / `[` / `cd` are `builtin`.
`type pwd` prints `pwd is a shell builtin`. `command -v pwd` prints `pwd`.
`type ls` prints `ls is /usr/bin/ls` (or wherever `PATH` resolves).
`command -v ls` prints that path. `type /bin/ls` prints `/bin/ls is /bin/ls`.

### just-bash (no `hostSpawn`)

Registered portable commands and in-process builtins implement the script.
There is no OS `PATH`. `type ls` / `which ls` describe that virtual world.

### Host-backed shell

`executeExternalCommand` in just-bash, as Demi wires it today (diverges
from bash where the tables say so):

1. Special builtins (`export`, `unset`, `exit`, … — not `cd`).
2. If the name is in `ctx.commands` (Demi registers the fork portable set
   here, plus product commands): run that handler. **`hostSpawn` is not
   consulted**, except for names in `HOST_PREFERRED_SCAN_COMMANDS`
   (`rg`, `grep`, `find`), which probe the host first.
3. Functions.
4. Remaining builtins (`cd`, `test` / `[`, `true`, `false`, …). `echo`,
   `printf`, and `pwd` are listed as builtins for `type`, but Demi also
   registers portable implementations; the registry entry wins, so those
   names run as portable commands, not as interpreter builtins.
5. Else `hostSpawn(name, args, { cwd: state.cwd, env })`.
6. `bash`, `sh`, `sleep` are omitted from the portable set, so they always
   `hostSpawn`.

Consequence: `ls` and `/bin/ls` are different programs. `ls` is just-bash.
`/bin/ls` contains `/`, is not in the registry under that string, and
`hostSpawn`s the real binary.

| Name the script uses | bash | Host-backed shell |
|---|---|---|
| `cd`, `test`, `[`, `export`, `set`, `source`, `.` | builtin | interpreter builtin |
| `pwd`, `echo`, `printf` | builtin | portable command (registry shadows the builtin) |
| `ls`, `cat`, `chmod`, `stat`, `mkdir`, `rm`, `cp`, `mv`, `which`, `file`, `whoami`, `hostname` | `PATH` binary | portable command; host binary is never exec’d |
| `grep`, `rg`, `find` | `PATH` binary | `hostSpawn` first; on “not found” (see [Spawn failures](#spawn-failures)), remember the miss and use portable for the rest of the interpreter lifetime |
| `git`, `bun`, `node`, … (not in the portable set) | `PATH` binary | `hostSpawn` |
| product `Command` (`demi`, embedder tools) | n/a | registered handler; must not be sent to `hostSpawn` |

Required alignment with bash:

- Interpreter builtins stay in-process (`cd`, `test`, `pwd`, `echo`, `printf`, …).
- Product registered commands stay in-process (they are not Unix names).
- Every other name `hostSpawn`s the `PATH` binary.
- Portable Unix implementations run only when `hostSpawn` reports
  **executable not found** (no such file on `PATH`), never when spawn failed
  for another reason, and never in preference to a binary that exists.
- `hostSpawnUnavailable` must not stick across `PATH` / `hash -r` changes.
  bash re-searches `PATH` unless the hash table says otherwise.

## Working directory

bash cwd is the process’s directory **inode** (`pwd -L` / `$PWD` are the
remembered path string). `rm -rf "$PWD"` does not change the inode. Children
`fork` and inherit it. bash does not `chdir` to the parent, to `$HOME`, or to
the Host `defaultCwd`.

| Action after `mkdir d && cd d && rm -rf "$PWD"` | bash (measured) | Host-backed shell |
|---|---|---|
| `pwd` / `pwd -L` | prints the old path, exit 0 | portable `pwd` prints `state.cwd` (same as `-L`) |
| `echo "$PWD"` | old path | `state.env` `PWD` (same) |
| `pwd -P` | exit 1; stderr `pwd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory` | portable `pwd -P` swallows `realpath` failure and prints the logical path, exit 0 |
| `/bin/pwd` (GNU pwd, default `-P`) | exit 1; stderr `/bin/pwd: couldn't find directory entry in '..' with matching i-node` | `hostSpawn`s with the missing path as `cwd` |
| absolute-path command (`/bin/echo ok`, `/bin/true`) | succeeds, exit 0 | `posix_spawn` `chdir`s the path string first → `ENOENT`, mapped as command-not-found (see below) |
| `ls .` / `ls` | exit 0, empty listing (the unlinked dir is empty) | portable `ls` stats the path string → cannot access |
| `ls -ld .` | exit 0; `nlink` 0, name `.` | path-string `stat` fails |
| `cd .` | exit 0; stderr `cd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory`; `$PWD` becomes `$PWD/.` | `cd` stats the path string → `No such file or directory` |
| `cd ..` immediately | exit 0; `$PWD` becomes the parent path (logical `$PWD/..`) | same string walk if `state.cwd` is still the old path |
| relative create (`echo x > f`, `/bin/touch f`) | exit 1; `f: No such file or directory` (kernel `ENOENT` on create in a rmdir’d directory; Python `open` fails the same way) | Host `fs` resolves against the path string and errors |
| next absolute command | still that inode; children inherit it | `spawn({ cwd: state.cwd })` with the deleted path |

`pwd -P` after the rmdir leaves `$PWD` unchanged but a later `cd ..` in that
same shell can set `$PWD` to `..` instead of the parent. Measure `cd ..` in a
shell that has not just failed `getcwd`.

`Host.defaultCwd` is only the initial cwd of a new shell (and the
`HostBackedFileSystem` relative-path default). It is not a fallback when
`state.cwd` is gone. Silently replacing a missing cwd with the parent,
`defaultCwd`, or `/` is a defect: relative paths then run in a tree the
script did not `cd` into.

Required: the process that actually `posix_spawn`s holds a **directory fd**
for the shell’s cwd (`cd` opens the new directory, closes the old fd). Spawn
`fchdir`s that fd (on Linux, `cwd=/proc/self/fd/N` is equivalent). Foreground
and background `spawn` use the same fd. `pwd -P` reports `getcwd` failure
instead of inventing a path. If the fd is gone, the shell is gone — same as a
dead bash process.

## Spawn failures

bash classifies exec failures by what failed:

| Situation | bash (measured) | Host-backed shell |
|---|---|---|
| name not on `PATH` | stderr `bash: line N: foo: command not found` (or `file: line N:` when running a script), 127 | `hostSpawn` error → `exitCode` null → `BashEnvironment` forces 127 and stderr `` `${command}: ${signal}` `` (Bun: `posix_spawn 'foo' … ENOENT`) |
| file exists, not executable | stderr contains `Permission denied`, 126 | depends on whether the name is portable-registered (often never exec’d) or `hostSpawn`d |
| path is a directory | stderr contains `Is a directory`, 126 | runtime-dependent |
| bad shebang interpreter (bash 5.2) | stderr contains `cannot execute: required file not found`, 127 | runtime-dependent |
| hashed `PATH` entry removed | exec of the remembered pathname; stderr `…: No such file or directory`, 127 — not “command not found”. `hash -r` then re-searches `PATH` | no hash table for `hostSpawn` names; `hostSpawnUnavailable` sticks for the interpreter lifetime |
| cwd inode missing, binary exists | command still runs (absolute path / already-resolved `PATH` lookup in this shell) | same `ENOENT` as missing binary |
| `preferHostSpawn` probe | n/a | `isHostSpawnNotFound`: `exitCode === 127` and stderr matches `/ENOENT\|not found/i` → treat as “host has no such binary”, fall back to portable, remember forever |

Required: `Host.process.spawn` / `hostSpawn` expose a **spawn-error kind**,
not a faked command result:

- `executable_not_found` — `execve` ENOENT for the binary (127, bash wording)
- `permission_denied` — EACCES (126, `Permission denied`)
- `cwd_unusable` — must not occur once cwd is a dirfd; if it does, the error
  names the directory, never the command
- other errno — pass through

Portable fallback is allowed only for `executable_not_found`. Stderr regexes
are not a spawn-error channel: a missing cwd, a missing `PATH` entry, and a
binary that itself prints `not found` are different events.

## File metadata

bash `test` / `[` are builtins; `ls`, `stat`, `find`, `chmod` are `PATH`
binaries. They all read the same `stat(2)` / `lstat(2)` result.

| Observation | bash / GNU (measured) | just-bash portable (also what Host-backed `ls`/`chmod`/`stat`/`which` run) |
|---|---|---|
| `ls -l` mode | `st_mode`: `-rwxr-xr-x` for 0755, `-rw-r--r--` for 0644, `-rwsr-xr-x` / `-rwxr-sr-x` / `-rwxr-xr-t` for setuid/setgid/sticky | files always `-rw-r--r--`, directories always `drwxr-xr-x`; `catch` fills the same fake line. `stat` / `find %M` / `tar` already use `formatMode(stat.mode)` |
| `ls -l` type | `lstat`; symlinks are `lrwxrwxrwx … -> target` | `stat` (follows); no `l`, no `->` |
| `ls -l` owner / nlink | uid/gid names; hard link nlink 2 | `1 user user` |
| `ls -l` `total` (directory listing) | allocated-block total (not the number of names) | entry count |
| `ls -F` | `*` on execute bits, `@` on symlinks, `/` on dirs | `mode & 0111` for `*` (disagrees with portable `ls -l` on the same file) |
| `chmod` | changes `st_mode` (`chmod 4755` shows `-rwsr-xr-x`) | Host `fs.chmod` actually changes the file |
| `test -x` / `[ -x f ]` | `access(X_OK)`: the class that applies to euid (owner uses owner bits — a `001` file owned by you is not executable) | `stat.mode & 0100` (owner bit only) |
| `test -O` / `-G` | owned by euid/egid (`/dev/null` is not `-O` for a non-root user) | true if the path exists |
| `test -c` | character device (`/dev/null` is `-c`) | path allowlist (`/dev/null`, …) |
| `test -ef` | same `st_dev`+`st_ino` after following symlinks (hard link matches; symlink matches its target) | string path equality |
| `stat -c %A` / `%a` | `st_mode`; GNU `stat` on a symlink reports `lrwxrwxrwx` / type `symbolic link` (lstat) | `formatMode(stat.mode)` / octal of `stat.mode`; `%F` is only directory vs regular file |
| `stat -c %u` / `%U` / `%g` / `%G` | real ids / names | `1000` / `user` / `1000` / `group` |
| `find -perm /111` | real mode (includes 0755 files, dirs, symlinks; excludes 0644) | real `stat.mode` when portable; Host-backed usually the real `find` unless the spawn probe “failed” |
| `which` (Debian `which`) | only executable `PATH` entries (0644 → exit 1, no stdout) | `exists()` only; no execute bit |
| `type -P` / `command -v` | print the `PATH` file even without execute bits; executing that file is still 126 | `type` uses `SHELL_BUILTINS` / registry, does not ask the Host `PATH` |

`HostFileStat` carries `mode`, `size`, `mtime`, and type flags — not uid, gid,
nlink, ino, or dev. Portable `ls -l` therefore cannot print a real owner line
from the Host contract today; the Host-backed fix is to run GNU `ls`, not to
approximate it.

Required: any two observations of the same file agree on type and mode.
Portable `ls -l` uses `lstat` + `formatMode` (including symlink type and
setuid/sticky). It does not substitute a constant mode string on success or
in `catch`.

## Identity

just-bash in-memory: virtual user `user`, uid/gid `1000`, hostname
`localhost`. Security tests assert the real host is not leaked.

Host-backed shell: the computer in front of the model. `buildExportedEnv` +
`Host.process.spawn` must see that computer.

| Name | bash on that Host (measured) | Host-backed shell |
|---|---|---|
| `whoami` | same as `id -un` | portable: always `user` |
| `hostname` | kernel hostname (`hostname` exit 0) | portable: always `localhost` |
| `$USER` / `$LOGNAME` | inherited from the environment (unset if not exported into `bash --norc`) | embedder `initialEnv` |
| `$UID` / `$EUID` | same as `id -u` | `virtualUid` `1000` |
| GNU `id` / `stat -c %U` | real ids / names | `id` `hostSpawn`s (correct if cwd works); portable `stat` prints `user` |

Required: on a Host-backed shell, `whoami`, `$USER`, `$UID`, and `ls -l`
owner are the same principal as a bash child of that Host. Virtual identity
stays in just-bash without `hostSpawn`.

## `cd` / `pwd` edge cases

| Topic | bash (measured) | Demi |
|---|---|---|
| `pwd` / `pwd -L` | `$PWD` (logical) | portable prints `ctx.cwd` |
| `pwd -P` | `getcwd`; failure is fatal (exit 1, stderr above) | `realpath` failure → logical path, exit 0 |
| GNU `/bin/pwd` | default `-P` (physical); `-L` prints logical `$PWD` while that path still exists. After rmdir of cwd, both fail with the i-node stderr above | `hostSpawn` |
| `Host.defaultCwd` when `spawn` omits `cwd` | n/a | `LocalHost` uses `defaultCwd`; that is not “cwd was deleted” |
| `cd -P` to a missing path | `bash: cd: …: No such file or directory`, exit 1 | `realpath` failure → logical path |
| `cd` to a missing path | `bash: cd: …: No such file or directory`, exit 1 | same wording |
| `cd -` | `$OLDPWD`, prints the new directory | same |
| `CDPATH` | search + print the resolved path | implemented |

## Environment of children

bash children receive **exported** variables only.

| Host | Child env |
|---|---|
| Required | `session` exported set (plus embedder `initialEnv` marked exported) |
| `LocalHost` | `{ ...process.env, ...params.env }` — Node’s environment leaks into every child |
| Container / remote Hosts | whatever that Host implements; must not merge the embedder process env |

## What `type` / `command -v` describe

`type` uses `SHELL_BUILTINS` (includes `echo`, `pwd`, `ls` is not a builtin).
`hostResolveCommand` exists on the interpreter and is unset in
`BashEnvironment`, so `type` / `command -v` do not ask the Host `PATH`.

| Script | bash (measured, `bash --norc`) | Host-backed shell |
|---|---|---|
| `type pwd` | `pwd is a shell builtin` | builtin word from `SHELL_BUILTINS`, while execution is the portable command |
| `command -v pwd` | `pwd` | does not ask the Host |
| `type ls` | `ls is /usr/bin/ls` (resolved `PATH`) | registered / portable, not `/bin/ls` |
| `command -v ls` | that same path | does not ask the Host |
| `type /bin/ls` | `/bin/ls is /bin/ls` | file path; execution `hostSpawn`s |

Required: `type`/`command -v`/`which` describe the same program dispatch will
run.

## Allowed differences

These are Demi product behavior, not Unix clones. They stay; they must not
leak into Unix observation (`ls`, `stat`, spawn errno).

- Observation window (`timeoutMs`), output caps, binary stdout placeholder,
  command artifacts (`stdout.txt` / `stderr.txt` / `stdout.bin`).
- `shell_exec` / `shell_status` / `shell_write` / `shell_abort` / `yield`.
- Command bridge shims on `PATH` for registered product commands.
- `rejectTimedPipelines`, execution limits, no 64-bit arithmetic in just-bash.
- Incomplete job control (`fg`/`bg`/`disown` stubs; `%n` wait is the
  supported subset).
- Portable command **flag coverage**: missing flags error; implemented flags
  do not contradict GNU/bash for the same invocation.
- just-bash without `hostSpawn`: virtual identity, no real devices, no kernel
  cwd inode.

Not allowed: silent cwd repair, collapsing distinct spawn failures into
command-not-found, portable implementations that lie about `st_mode` or
ownership when the filesystem already has the bits, registry shadowing of
`PATH` for Unix names when the Host has the binary.

## Tests

Oracle tests run the **same script** in GNU bash and in the surface under
test. Assert exit code and the Unix fact (mode bits, whether the command
ran, whether stderr is `getcwd` vs `command not found`). Do not assert
prompt strings. Do not treat Demi output as the expected value. Do not
copy a bash result from this table into a test by hand — put the `bash
--norc --noprofile` invocation in the test and let it measure.

- Bash column of this document:
  `packages/shell/src/__tests__/bash-oracle.test.ts`
- just-bash portable vs recorded bash fixtures:
  `packages/just-bash/.../src/comparison-tests/` + `runRealBash`
- Host-backed Demi vs that oracle: `@demicodes/shell` tests with `LocalHost`

Minimum matrix (already in the oracle test, plus Host-backed counterparts):

1. `chmod 755` / `chmod 644` × `ls -l` × `test -x` × `stat -c %A` × `find -perm /111` — one answer for “executable”.
2. `ln -s` × `ls -l` × `test -L` × `readlink`.
3. Delete cwd, then `pwd`, `pwd -P`, `/bin/pwd`, an absolute-path command, `ls .`, `cd .`, `cd ..`.
4. Missing name vs non-executable file vs directory vs deleted cwd — distinct errors.
5. `type ls` vs `type pwd` vs `command -v`.
6. `whoami` vs `id -un` vs `$UID` vs `id -u`.
7. `grep` with a real binary → `system-command` audit; with no binary on `PATH` → portable; with cwd deleted → **not** portable.

Anti-cases (must fail the suite if they happen):

- spawn failure changes cwd to `defaultCwd` / parent
- `ENOENT` from a missing cwd classified as `executable_not_found`
- `ls -l` on a `0755` file prints `rw-r--r--`
