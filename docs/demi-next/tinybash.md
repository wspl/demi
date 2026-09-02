# Demi Next: tinybash

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (M8) |
| Scope | The tiny shell hostless conversations run in: grammar, semantics, refusals, interface, verification |

## Role

tinybash is the in-process counterpart of real bash on an execution
target. A conversation without a machine (`sessions-and-targets.md`) still
has a `bash` tool; what runs its tool calls is tinybash: a parser and
executor for a fixed, small subset of bash whose only executables are the
command manifest's root commands (`commands.md`), dispatched through a
loader. It lives in `@demicodes/tinybash`, pure JS, with no Host of its own
and no knowledge of the backend; any embedder with a loader can use it.

The one guarantee everything below serves:

> **Acceptance implies bash-equivalence.** Any script tinybash accepts
> means exactly the same thing in bash, given the same commands. Anything
> that bash would expand, rewrite or run differently is refused, never
> approximated.

So there is no divergence catalogue: the grammar below is the whole
contract, and a script that falls outside it is the signal to provision a
machine.

## Grammar

```
script      := list ( ( ";" | NEWLINE )+ list )* ( ";" | NEWLINE )*
list        := command ( ( "&&" | "||" ) command )*
command     := word+ heredoc?
heredoc     := "<<" ["-"] delimiter          body follows on the next lines, ends at a line equal to delimiter
             | "<<<" word                    here-string: word plus a trailing newline
delimiter   := word                          quoted ('EOF', "EOF") or bare (EOF)
word        := ( bare | single | double | escape )+
bare        := any char except whitespace, quotes, "\", "#", ";", "&", "|", "<", ">", "(", ")", "`", "$", "*", "?", "[", "~"
single      := "'" any except "'" "'"
double      := '"' ( any except '"' "\\" "$" "`" | "\\" ( '"' | "\\" ) )* '"'
escape      := "\\" any                      outside quotes: the next char literally; "\\" NEWLINE joins lines
comment     := "#" to end of line            only where a word may start
```

Rules the grammar does not show:

- A word is the concatenation of its parts (`a'b'"c"` is `abc`), as in
  bash.
- `&&` and `||` are left-associative with equal precedence, as in bash.
- One heredoc or here-string per command, attached to the command's stdin;
  it may appear anywhere after the first word. A bare delimiter's body is
  taken literally only if it contains no `$` or backtick (see refusals);
  `<<-` strips leading tabs from body lines and from the terminator.
- The terminator must be a whole line; a script that ends inside a heredoc
  is refused.
- Whitespace is space and tab; `\r` is refused.

## Semantics

**Parse first, then run.** The whole script is parsed before anything
executes. If any statement is refused, nothing runs and the tool result is
the refusal. If every statement's first word is a root command, the script
runs; if any first word is not, the script is not tinybash's — the backend
provisions a machine and runs the **entire script** there, intact
(`sessions-and-targets.md`). Hostless execution therefore never leaves a
script half-done.

Execution:

- Commands run strictly in order, one at a time. Each command is
  `loader.dispatch(root, argv, io)` with `argv` the words after the root,
  stdin the heredoc body (or empty), stdout and stderr streamed to the tool
  result as produced.
- `a && b` runs `b` only if `a` exited 0; `a || b` only if `a` exited
  non-zero; `;` and newline run the next command regardless.
- The script's exit code is the exit code of the last command that ran; an
  empty script exits 0.
- `ctx.cwd` and `ctx.env` for every command are the conversation's current
  cwd and environment as the backend tracks them. No command changes them;
  there are no assignments and no builtins.
- Cancellation: the tool's `AbortSignal` aborts the running command and
  skips the rest; the exit code is the aborted command's.
- Output limits are the backend's existing bounded capture; tinybash
  imposes none.

There are **no builtins**. `cd`, `echo`, `export`, `exit`, `true` and the
like are refused like any non-root word; the refusal message for the common
ones names the alternative (paths relative to the cwd, `demi file` for
writing, a machine for everything else).

## Refusals

A refusal exits 2 with one line on stderr: what was found, why it is not
available here, and the way out. Refusals are decided at parse time; the
message names the offending token and its line.

| Found | Why refused | Way out named |
|---|---|---|
| `\|`, `>`, `>>`, `<` (not `<<`/`<<<`), `&>`, `2>` | pipes and redirections need processes | a machine; `demi file create` for writing output to a file |
| `&` (background), `;;`, `(`, `)`, `{`, `}` as words | job control and grouping | a machine |
| `$name`, `${…}`, `$(…)`, backtick — outside single quotes | bash would expand; tinybash never expands | write the value literally, or quote it in single quotes if it is text |
| `$` or backtick in a **bare**-delimiter heredoc body | bash would expand the body | quote the delimiter: `<<'EOF'` |
| `*`, `?`, `[` in an unquoted word | globbing | quote the word, or name the files |
| `~` at the start of an unquoted word | tilde expansion | an absolute or cwd-relative path |
| `NAME=value` before a command | assignments | a machine |
| `if`, `for`, `while`, `case`, `function`, `!` as a first word | control flow | a machine |
| a first word that is not a root command | no executables here | not a refusal when managed hosts are configured — the whole script runs on a provisioned machine; otherwise "this conversation has no machine; pair a device and move it there" |
| unterminated quote or heredoc, `\r` | not a script | fix the script |

Everything bash accepts that is not in the grammar and not in this table
is still refused — the table lists what needs a tailored message, not the
boundary. The boundary is the grammar.

## Interface

```ts
runTinybash(input: {
  script: string
  roots: ReadonlySet<string>        // the manifest's root command names
  dispatch: (root: string, argv: string[], io: CommandIO) => Promise<number>
  cwd: string
  env: Readonly<Record<string, string>>
  io: CommandIO                      // stdout/stderr sinks for the whole script
  signal?: AbortSignal
}): Promise<
  | { kind: 'ran'; exitCode: number }
  | { kind: 'refused'; exitCode: 2; message: string }
  | { kind: 'not-tinybash'; firstWord: string }   // hand the script to a machine
>
```

`parseTinybash(script, roots)` is exported separately and returns the
statement list or the refusal; the backend uses it to decide the
`not-tinybash` case before touching the loader, and tests use it for the
grammar table.

## What tinybash is not

- Not a bash implementation, not a "portable bash", not a place for
  builtins or coreutils. When a hostless conversation needs `ls`, it needs
  a machine, and the design gives it one on the first such command.
- Not a sandbox. Its safety property is the bash-equivalence guarantee,
  which is about meaning, not isolation; the store-backed Host is the
  boundary for what a hostless command can touch.
- Not used on real hosts. A target runs real bash; tinybash never runs
  inside the runner or the shell.

## Verification

- **Grammar table**: every production and every refusal row has positive
  and negative cases: words with each quoting form and concatenation; each
  heredoc form including `<<-` tab stripping, a bare delimiter with and
  without `$` in the body, an unterminated body; `;`, `&&`, `||`, newline
  and mixed chains with the exit-status semantics; comments in every
  position; line continuation; each refusal token in quoted (accepted) and
  unquoted (refused) positions.
- **Equivalence**: the accepted corpus is run through real bash with the
  roots stubbed as scripts that record `argv` and stdin; tinybash's
  dispatch log must match bash's byte for byte. This is the guarantee's
  test, and it runs in CI.
- **Parse-first**: a script with a refusal in its last statement executes
  nothing; a script with a non-root first word in its last statement is
  reported `not-tinybash` before any dispatch.
- **Cancellation** aborts mid-command and skips the rest.
- The backend's hostless integration (`roadmap.md` M8) runs `demi file`
  and `demi todo` scripts through tinybash end to end.
