# GNU differential corpus

This directory holds one recorded corpus per standard utility:
`testdata/gnu/<utility>/cases.json`. Each corpus is a differential test
against the utility's reference implementation (GNU coreutils 9.4, grep
3.11, sed 4.9, findutils 4.9, diffutils 3.10, `rg` 14.1, `jq` 1.7), run as a
black box. `internal/testing/gnucorpus` (see its package doc) reads these
files and runs them against the Go utilities; `internal/testing/gnucorpus/record`
produces them.

## Format

A corpus is a JSON array of cases:

```json
{
  "name": "dash-n-count",
  "core": true,
  "argv": ["-n", "3", "twenty.txt"],
  "stdin": "eA==",
  "tree": { "files": { "twenty.txt": "bGluZTEK..." } },
  "want": {
    "stdout": "bGluZTEKbGluZTIKbGluZTMK",
    "exitCode": 0,
    "stderrNonEmpty": false,
    "tree": { "files": { "twenty.txt": "bGluZTEK..." } }
  }
}
```

- `name` identifies the case (becomes the subtest name); `core` marks it as
  core (an option the old tests use, or one coding agents commonly use) or
  extended (everything else worth supporting).
- `argv` is the utility's arguments; the utility's own name (`argv[0]`) is
  added by the harness, not listed here.
- `stdin`, and every file's content under `tree`/`want.tree`, are
  base64-encoded byte strings (Go's `encoding/json` does this for `[]byte`
  automatically), so binary content round-trips exactly; this makes the JSON
  unreadable at a glance, which is the deliberate trade-off for byte-exact
  comparison.
- `tree` is the file tree created under the case's working directory before
  the utility runs: `files` (path → content), `dirs` (empty directories),
  `symlinks` (path → link target text), `modes` (path → permission bits) and
  `owners` (path → `[uid, gid]`).
- `want.tree` is the same shape, snapshotted after the utility runs; for a
  case that does not write files it is simply unchanged from `tree`. `modes`
  and `owners` are always captured for every path but compared only for the
  paths `want.tree` lists, so a case that does not care about a file's mode
  or owner does not list it there.
- `checkTimes` (only present on a few `touch` cases) lists paths whose
  modification time is compared; every other case ignores a path's time
  entirely, because it is otherwise not reproducible between recording and
  checking (see "What is out of scope" below).
- Every case runs with `LC_ALL=C`, `LANG=C`, `TZ=UTC`, and `HOME` set to a
  fixed directory inside the case's own temp dir (`gnucorpus.BaseEnv`);
  `env` in `case.json` adds to or overrides that for one case.

## Re-recording

```sh
go run ./internal/testing/gnucorpus/record            # every utility
go run ./internal/testing/gnucorpus/record cat sed     # just these
```

The recorder runs each case's input against the reference binary (resolved
through `PATH`) in the same kind of sandboxed temp dir the harness gives the
Go utility, and writes the result as that case's `want`. Case *inputs*
(`argv`, `stdin`, `tree`, `core`/extended, `env`, `checkTimes`) are the
source of truth, hand-authored in
`internal/testing/gnucorpus/record/definitions/*.go`; running the recorder
regenerates every `want` from them and must reproduce the committed files
byte for byte (this is one of the checkpoint checks). If it does not, either
a case is not actually deterministic (see below) or the reference tool's
behavior changed.

## What is out of scope

A few utilities have options whose correct output is inherently
non-reproducible across separate runs or hosts. Per the brief, these are
avoided rather than forced into the harness:

- **Timestamps and "now".** `ls -l`/`-t`, `stat`'s default (no `--format`)
  output, and plain `date` (no `-d`) all embed or depend on the current
  wall-clock time or a file's real modification time. This corpus uses
  `stat -c` with fields that exclude times, sorts `ls` by name or size
  rather than time, and always drives `date` with `-d`/`--date` so its
  output depends only on the fixed moment given, never on "now". `diff`'s
  unified/context formats print each file's modification time, so the
  corpus gives the compared files a fixed, explicit `checkTimes`-style
  time via `tree.times` before diffing them.
- **Disk usage and mounts.** `df` reports the real host's filesystem sizes
  and mount points, which this harness's input tree cannot pin down; its
  corpus is limited to argument/error handling. `du` avoids the same problem
  for on-disk block counts by always using `-b`/`--apparent-size`, which
  reports the byte sizes this harness does control.
- **Randomness.** `mktemp`'s purpose is an unpredictable file name, so a
  successful run's stdout and the tree entry it creates cannot be pinned
  down; its corpus covers only deterministic argument-validation errors.
- **The working directory's own absolute path.** `realpath`'s plain output
  is the absolute path of a fresh temp directory, unique to every recording
  and every check run. Its corpus uses `--relative-to` throughout instead,
  which reports the same resolution relative to the working directory.
- **The GNU version banner.** No utility's corpus tests `--version`; the Go
  port is not expected to reproduce GNU's literal banner text, and Gate B
  does not require it.

`--help` is required by Gate B for every utility but is not part of these
corpora, for the same reason as `--version`: its exact wording is not a
behavior the Go port needs to reproduce byte for byte, only its presence and
exit code, which a dedicated non-corpus test can check more directly.

## Design gaps this corpus surfaces (for the orchestrator)

- `gnucorpus.Check(t, utilityName, utility, cases)` registers only
  `utilityName` for `Invocation.Run`. A case that runs another program
  through the utility under test (`find -exec`, `xargs`, and `xargs`'s own
  default command when none is given) can only be checked this way when the
  invoked program's name equals `utilityName` itself. The corpora for
  `find` and `xargs` are recorded against real external programs (`echo`,
  `cat`, `false`) as agents actually use them, since recording only needs
  those binaries on `PATH`; running these cases against the Go
  implementations will need `Check` (or a variant of it) to accept a full
  program registry, not just the one utility. This needs an orchestrator
  decision before the work package that implements `find`/`xargs` builds
  its own tests on top of this corpus.

## Core options per utility

Core count is of the recorded cases; a utility can have more core options
than distinct flags shown here when an option takes a form (e.g. `-n 3`
vs. `-3`) covered by more than one case.

| Utility | Core cases | Core options |
| --- | --- | --- |
| `cat` | 7/12 | `-n`, `-b`, `-s` |
| `head` | 7/11 | `-n`, `-c` |
| `tail` | 7/10 | `-n`, `-c` |
| `wc` | 7/10 | `-l`, `-w`, `-c` |
| `tee` | 4/6 | `-a` |
| `sort` | 8/12 | `-r`, `-u`, `-n`, `-t`, `-k`, `-c` |
| `uniq` | 6/8 | `-c`, `-d`, `-u`, `-i` |
| `cut` | 5/8 | `-d`, `-f`, `-c`, `-b` |
| `tr` | 4/7 | `-d`, `-s` |
| `grep` | 14/20 | `-i`, `-n`, `-v`, `-c`, `-l`, `-L`, `-r`, `-E`, `-F` |
| `rg` | 11/16 | `-i`, `-n`, `-v`, `-c`, `--sort`, `-l`, `-F` |
| `find` | 10/14 | `-name`, `-iname`, `-type`, `-maxdepth`, `-mindepth`, `-empty`, `-print0` |
| `xargs` | 6/8 | `-n`, `-I` |
| `sed` | 8/12 | `-n`, `-e`, `-i` |
| `jq` | 12/16 | `-r`, `-c`, `-n`, `--arg`, `-s` |
| `ls` | 9/13 | `-A`, `-a`, `-R`, `-F`, `-d` |
| `cp` | 7/11 | `-r`, `-n` |
| `mv` | 6/6 | `-n` |
| `rm` | 6/7 | `-f`, `-r`, `-rf` |
| `mkdir` | 5/6 | `-p` |
| `rmdir` | 3/4 | (operands only) |
| `touch` | 5/7 | `-t`, `-c`, `-r` |
| `stat` | 5/7 | `-c` |
| `du` | 4/5 | `-b`, `-sb`, `-ab` |
| `df` | 1/2 | (operands only; see "What is out of scope") |
| `chmod` | 5/7 | `-R` (plus octal and symbolic mode operands) |
| `chown` | 4/6 | (numeric `uid[:gid]` operands only) |
| `realpath` | 4/5 | `--relative-to`, `-e` |
| `mktemp` | 2/3 | `-u` (argument-validation errors only; see "What is out of scope") |
| `basename` | 4/5 | `-s` |
| `dirname` | 3/4 | (operands only) |
| `env` | 4/5 | `-i`, `-u` |
| `seq` | 6/8 | `-s` (plus first/increment/last operands) |
| `date` | 6/7 | `-u`, `-d`, `--iso-8601`, `--rfc-3339` |
| `sleep` | 5/5 | (operands only) |
| `paste` | 4/5 | `-d`, `-s` |
| `nl` | 5/7 | `-ba`, `-s`, `-w` |
| `tac` | 4/5 | (operands only) |
| `od` | 6/7 | `-An`, `-tx1`, `-c`, `-j`, `-N` |
| `diff` | 7/9 | `-u`, `-c`, `-q`, `-s` |
| `cmp` | 6/8 | `-s`, `-l` |
