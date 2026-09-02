# Demi Next: tinybash

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Implemented (M8) |
| Scope | The tiny shell hostless conversations run in: the boundary and why it sits there, grammar, builtins, semantics, refusals, interface, verification |

## Role

tinybash is the in-process counterpart of real bash on an execution
target. A conversation without a machine (`sessions-and-targets.md`) still
has a `bash` tool; what runs its tool calls is tinybash: a parser and
executor for a fixed subset of bash, with a fixed set of builtin commands
implemented over the conversation's store-backed Host, and the command
manifest's root commands (`commands.md`) dispatched through a loader. It
lives in `@demicodes/tinybash`, pure JS, and knows nothing about the
backend; any embedder with a Host and a loader can use it.

The one guarantee everything below serves:

> **Acceptance implies bash-equivalence.** Any script tinybash accepts
> means exactly what it means in bash with GNU coreutils, given the same
> root commands. Anything bash or a GNU tool would do differently is
> refused, never approximated.

GNU is the reference because managed hosts are Linux: what the model sees
from `ls` or `grep` on a real target is GNU output, and hostless must show
the same. There is no divergence catalogue; the grammar, the builtin table
and the refusal table below are the whole contract.

## Where the boundary sits, and why

The boundary was placed with a corpus of 6 120 `bash` tool calls written
by a coding agent across 107 projects (methodology, full tables and bias
notes in `progress.md`). Each call was classified by the shell constructs
it uses; the cumulative share of calls that fall entirely within a growing
subset of bash is:

| Subset | Cumulative share | Marginal cost |
|---|---|---|
| one simple command, quoting | 9 % | tokenizer |
| + newline, `;`, `&&`, `\|\|` | 16 % | trivial |
| + heredoc, here-string | 18 % | small |
| + `cd` | 20 % | cwd becomes session state |
| + pipes between builtins and roots | 38 % | stream plumbing + the builtins |
| + `2>&1`, `/dev/null` | 63 % | trivial |
| + `~/` | 78 % | trivial |
| + `$NAME`, assignments | 83 % | small |
| + globs | 90 % | small |
| + `> file`, `>> file`, `< file` | 94 % | small |
| + `$( )`, backticks | 96 % | nested execution, semantic risk |
| + control flow, subshells, `&` | 100 % | a full interpreter |

Excluding the sessions of this repository (whose operations work skews
the corpus) the ladder is within two points at every step. The knee is
plain: everything through file redirections costs a few hundred lines
each and buys 94 %; the last 6 % costs an interpreter. tinybash stops
before command substitution.

The programs the calls invoke set the other axis. 58 % of calls run at
least one real tool (`bun`, `git`, `python3`, …), which no hostless shell
can serve — those calls are what auto-provisioning is for. The remainder
is concentrated: `grep` in 48 % of calls, `head` 41 %, `tail` 24 %, `echo`
22 %, `sed` 16 %, `ls` 10 %, `cat` 6 %; of pipes, 97 % feed `head`, `tail`
or `grep`. A builtin set of a dozen commands with whitelisted flags covers
what the model habitually reaches for; `sed` is admitted only for printing
line ranges, because its `s///` dialect was the largest source of
divergence in the previous portable-command effort.

## Grammar

```
script      := list ( ( ";" | NEWLINE )+ list )* ( ";" | NEWLINE )*
list        := pipeline ( ( "&&" | "||" ) pipeline )*
pipeline    := command ( "|" command )*
command     := assignment* word+ redirect*  |  assignment
assignment  := NAME "=" word
redirect    := ">" word | ">>" word | "<" word | "2>" word | "2>>" word | "2>&1" | "&>" word
             | "<<" ["-"] delimiter        heredoc body on the following lines, ends at a line equal to delimiter
             | "<<<" word                  here-string: the word plus a trailing newline
word        := ( bare | single | double | escape | "$" NAME | "${" NAME "}" | "~" )+
bare        := any char except whitespace, quotes, "\", "#", ";", "&", "|", "<", ">", "(", ")", "`", "$", "{", "}"
single      := "'" any except "'" "'"
double      := '"' ( any except '"' "\\" "`" | "\\" ( '"' | "\\" | "$" ) | "$" NAME | "${" NAME "}" )* '"'
escape      := "\\" any                    outside quotes: the next char literally; "\\" NEWLINE joins lines
comment     := "#" to end of line          only where a word may start
```

Expansions, in bash's order, on words outside single quotes:

- **Tilde**: a leading `~` or `~/` becomes the session's home.
- **Parameter**: `$NAME` and `${NAME}` become the session variable or
  the empty string; `$PWD` is the cwd. Nothing else after `$` is accepted
  (see refusals). An unquoted expansion is split into words on blanks, as
  bash does with the default `IFS`, and an unquoted expansion to nothing
  yields no word; a quoted one is always exactly one word.
- **Globbing**: `*`, `?` and `[…]` in an unquoted word match against the
  Host filesystem relative to the cwd, sorted, with bash's default of
  leaving the word literal when nothing matches. `**` is not accepted.
- Words concatenate (`a'b'"c"` is `abc`); a bare-delimiter heredoc body
  expands `$NAME` like bash and a quoted-delimiter body is literal.

Whitespace is space and tab; `\r` is refused. `{` and `}` are literal
except in the forms bash expands (`{a,b}`, `{1..3}`), which are refused;
`~user` is refused.

Text is bytes throughout, as in the C locale a managed host runs with:
`sort` orders bytes, `grep` matches bytes, `wc -w` splits on ASCII blanks,
`ls -l` prints English month names, and the tools quote names with plain
`'…'`. tinybash is the C locale; there is no other.

## Builtins

Every builtin is GNU coreutils behaviour for the listed flags and refuses
any other flag or form with a message. Output formats are GNU's.

| Command | Accepted | Notes |
|---|---|---|
| `grep` | `-n -i -v -c -l -r -E -F -A N -B N -C N`, files or stdin | patterns are translated from ERE (or BRE without `\(` `\{`) to JS regex, with `-i` folding ASCII letters only, as the C locale does; a pattern that has no faithful translation is refused; `-r` visits entries in name order |
| `head`, `tail` | `-n N`, `-N`, `-c N`, `tail -n +N` | |
| `cat` | `-n` | |
| `echo` | `-n -e` | |
| `printf` | `%s %c %d %i %u %x %X %o %%` with `-` `+` space flags, width, precision; format escapes incl. `\NNN` `\xHH` | bash's builtin: the format is reused while arguments remain, a non-number is reported and counts as the parsed prefix; `-v`, `*`, `%N$`, `%b`, `%q`, floating conversions, `#`, `'` and `0` with `%s` are refused |
| `ls` | `-l -a -1 -R`, paths | GNU column and long formats |
| `find` | paths, `-name`, `-iname`, `-type f\|d`, `-maxdepth N` | no other predicate, no `-exec`; entries come in name order, where GNU's order is whatever the filesystem returns |
| `wc` | `-l -w -c` | |
| `sort` | `-r -n -u -k N` | |
| `uniq` | `-c` | |
| `cut` | `-d X -f LIST` | |
| `tr` | two sets, `-d` | |
| `sed` | `-n` with `Np`, `N,Mp`, `$p` | printing line ranges only; `-i` and `s///` refused with `demi file edit` as the way out |
| `mkdir` | `-p` | |
| `rm` | `-r -f` | |
| `mv`, `cp` | `cp -r` | |
| `touch` | | |
| `pwd`, `true`, `false` | | |
| `test`, `[` | `-e -f -d -s`, `=`, `!=`, `-z`, `-n` | |
| `cd` | one path or none | changes the session cwd for subsequent tool calls |

Refused outright, with the way out named: `awk`, `jq`, `xargs`, `perl`,
`python`, `sed -i`, `sed s///`, `export`, `source`, `eval`, `exit`,
`sleep`, `kill`.

## Semantics

**Parse first, then run.** The whole script is parsed before anything
executes. A script is either entirely inside the subset and runs here, or
it is **outside**, and nothing runs: the backend provisions a machine and
runs the entire script there, intact and silently
(`sessions-and-targets.md`). Inside means all of: every construct in the
grammar; every command word a builtin or a root; every flag on its
whitelist; every absolute path — builtin arguments, redirection targets,
`cd` targets, glob expansions, and root-command arguments the manifest
marks as paths — inside the namespace the embedder declares (`/home/demi`
and `/tmp` in the product). Hostless execution never leaves a script
half-done, and the model never learns where the line is. Grammar,
programs, flags and paths outside the subset are the same case; the
distinction below exists only for the refusal messages an embedder shows
when it has no machine to hand the script to.

The one run-time failure that is not an upgrade is the storage quota: a
write that exceeds it fails with an `ENOSPC`-class error, as a full disk
would on a machine.

Execution:

- Statements run in order. A pipeline runs its commands concurrently with
  byte streams between them; its exit code is the last command's.
- `a && b` runs `b` only if `a` exited 0; `a || b` only if `a` exited
  non-zero; `;` and newline run the next statement regardless.
- The script's exit code is the last statement's; an empty script exits 0.
- Redirections open paths on the Host filesystem relative to the cwd;
  `/dev/null` is a sink and an empty source.
- The **session shell state** is the cwd and the variables: `cd` and
  assignments change it for the rest of the script and for later tool
  calls, as in a real shell session. A prefix assignment
  (`NAME=value cmd`) applies to that command only. Root commands receive
  the cwd and variables as `ctx.cwd` and `ctx.env`.
- Root commands are `loader.dispatch(root, argv, io)` with stdin from the
  pipeline, heredoc or `<` and stdout/stderr into the pipeline or
  redirections.
- Cancellation: the tool's `AbortSignal` aborts the running statement and
  skips the rest.
- Output limits are the backend's existing bounded capture.

## Refusals

Refusals are what an embedder **without a machine** shows for a script
outside the subset; in the product they surface only when a deployment has
no machine to upgrade to. A refusal exits 2 with one line on stderr: what
was found, why it is not available here, and the way out; it names the
offending token and line.

| Found | Why refused | Way out named |
|---|---|---|
| `$(…)`, backtick, `$((…))` | command and arithmetic substitution | write the value literally; a machine for the rest |
| `${NAME:-…}` and every other `${…}` operator, `$?`, `$1`, `$@` | parameter operators and positionals | a plain `$NAME` or the value literally |
| `{a,b}` brace expansion, `**` | not expanded | list the names |
| `~user` | tilde with a user name | the home path written out |
| `if`, `for`, `while`, `case`, `function`, `!`, `(`, `{` as words | control flow and grouping | a machine |
| `&` (background), `;;`, `<(…)`, `>(…)` | job control, process substitution | a machine |
| `\|` into a non-builtin, non-root command | no such program here | a machine |
| an absolute path outside the namespace | no such place here | a path under the home, or a machine |
| a builtin with a flag outside its whitelist | not implemented faithfully | the listed flags, or a machine |
| `grep` pattern with no faithful translation | dialect | `-F`, or a simpler pattern |
| unterminated quote or heredoc, `\r` | not a script | fix the script |

## Interface

```ts
runTinybash(input: {
  script: string
  roots: ReadonlyMap<string, RootPaths>   // root names → which of an argv's arguments are paths
  namespace: readonly string[]            // absolute prefixes the script may touch, e.g. ['/home/demi', '/tmp']
  dispatch: (root: string, argv: string[], io: DispatchIO) => Promise<number>
  fs: HostFileSystem                      // the builtins' filesystem
  state: { cwd: string; home: string; vars: Record<string, string> }   // mutated by cd and assignments
  io: CommandIO                           // the script's stdout and stderr
  identity: { user: string; group: string }   // owner names `ls -l` shows; every hostless file is the session user's
  stdin?: AsyncIterable<Uint8Array>       // the script's own stdin: what the caller writes while the script runs
  signal?: AbortSignal
}): Promise<
  | { kind: 'ran'; exitCode: number }
  | { kind: 'outside'; reason: OutsideReason; message: string }   // hand the whole script to a machine
>

type RootPaths = (argv: readonly string[]) => readonly string[]   // the path-typed arguments of this invocation

interface DispatchIO {
  stdin: AsyncIterable<Uint8Array>     // the pipeline, heredoc, `<` file, or empty; finite
  stdinStream?: AsyncIterable<Uint8Array>   // the script's stdin, when this command's stdin is not redirected; live
  stdout: CommandIO['stdout']
  stderr: CommandIO['stderr']
  cwd: string
  env: Record<string, string>
  signal?: AbortSignal
}
```

`RootPaths` is a function because which argument is a path depends on the
leaf the argv selects; the loader builds it from the manifest's path marks
(`commands.md`) so tinybash never parses a root's arguments itself. Tests
hand in a table. `DispatchIO` is what a root command's `ctx` is built
from: the loader adds `args` and `fs`. The script's `stdin` is what real
bash would give a job: one stream every command whose stdin is not
redirected reads in turn (a shared iterator, never closed by a reader
that stops early). Builtins never read it — a builtin with no input reads
nothing — so it only reaches root commands, which is how `shell_write`
steers a running `demi agent spawn` in a hostless conversation.

`OutsideReason` names the construct, program or flag that put the script
outside the subset, and `message` is the refusal line for embedders with
nowhere to hand the script. `parseTinybash(script, roots, namespace, state)`
is exported separately and returns the statement list or the `outside`
result; the backend uses it to decide before touching the loader or the
Host, and tests use it for the grammar table. Path conditions that need
the filesystem — a glob whose expansion leaves the namespace — are decided
by `runTinybash` before the first statement runs, still under parse-first.

## Packages

`@demicodes/tinybash` depends on `@demicodes/shell` for `HostFileSystem`
and `CommandIO` and on `@demicodes/utils`; nothing else. Its directories
mirror this document: `grammar/` (lexer, parser, AST, expansions),
`outside/` (the refusal table and the parse-first checks), `exec/`
(statements, pipelines, byte streams, redirections, session state) and
`builtins/` (one file per builtin over a shared flag parser, the whitelist
declared as a table).

## What tinybash is not

- Not a bash implementation and not a coreutils implementation: the
  builtin table is closed, the flag lists are closed, and nothing outside
  them is approximated. When a hostless conversation needs more, it needs a
  machine, and the design gives it one on the first such command without
  telling the model.
- Not something the model knows about. The tool is bash; tinybash is the
  backend's business.
- Not a sandbox. Its safety property is the equivalence guarantee, which
  is about meaning, not isolation; the store-backed Host bounds what a
  hostless command can touch.
- Not used on real hosts. A target runs real bash and GNU tools; tinybash
  never runs inside the runner or the shell.

## Verification

- **Grammar table**: every production, expansion and refusal row has
  positive and negative cases: each quoting form and concatenation; each
  heredoc form including `<<-` tab stripping and expansion in bare bodies;
  chains and pipelines with exit-status semantics; each redirection;
  tilde, parameter and glob expansion including the no-match case;
  comments and line continuation; each refusal token quoted (accepted)
  and unquoted (refused).
- **Builtin table**: per builtin, every accepted flag against GNU
  behaviour, and every refused flag.
- **Equivalence corpus**: the accepted scripts, with roots stubbed as
  programs that record `argv` and stdin, run through real bash with GNU
  coreutils on Linux; tinybash's output, exit codes and dispatch log must
  match byte for byte. The scripts live in
  `packages/tinybash/src/__tests__/corpus/cases.ts` over a fixed fixture
  tree; bash's results are committed as goldens
  (`corpus/goldens/*.json`, home path normalized) so every platform's
  `bun test` compares against them, and on Linux
  `TINYBASH_CHECK_GOLDENS=1` re-derives them from bash and fails on drift.
  Cases whose output depends on the filesystem underneath (`ls -l` block
  totals and directory sizes) are compared on Linux only. This is the
  guarantee's test and runs in CI on Linux.
- **Parse-first**: a script whose last statement is outside the subset
  executes nothing, whether the cause is grammar, a program, a flag or a
  path; path cases cover builtin arguments, redirections, `cd`, globs and
  path-typed root arguments, absolute and relative.
- **Split equivalence** (`sessions-and-targets.md`): tool-call sequences
  run hostless-then-machine at every split point match the all-machine
  run byte for byte.
- **Reference suites**: the just-bash fork's compatibility tests filtered
  to the subset; the oils spec tests (cross-shell, table-driven, with
  bash's expected output per case) filtered to the subset; GNU bash's and
  coreutils' own test cases for the accepted builtins and flags.
- **Session state**: `cd` and assignments persist across tool calls;
  prefix assignments do not.
- The backend's hostless integration (`roadmap.md` M8) runs `demi file`,
  `demi todo` and builtin pipelines through tinybash end to end.
