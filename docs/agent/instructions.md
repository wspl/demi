# Instructions

The user's personal instructions and a project's `AGENTS.md` or `CLAUDE.md`
files reach every node's model as one `context` block, the instructions block.
The composer's context card lists what the block holds, and each entry leads
to where it is written. This document owns which files are read, what the
model receives, and what the card lists. The context blocks themselves are the
[agent runtime](transcript.md#context)'s; the Instructions settings section is
[the product's](../product/web-application.md#web-app-scope).

## What the model receives

For example, the user writes "Keep answers brief. Run the tests before you
finish." in Settings › Instructions, and opens a conversation on their Mac in
`~/Projects/shop/packages/web`, inside the git repository `~/Projects/shop`.
The repository has `AGENTS.md` at its root, `CLAUDE.md` beside it as a
symbolic link to it, and `packages/web/AGENTS.md`. Before the first request
of the root node, the model receives:

```text
These are the user's personal instructions and the project's instruction
files. Follow them in this conversation together with your other
instructions; this block replaces any earlier one. Where they conflict, the
user's messages come first, then a file nearer the working directory over
one further up, then the personal instructions.

<personal_instructions>
Keep answers brief. Run the tests before you finish.
</personal_instructions>

<project_instructions path="/Users/alex/Projects/shop/AGENTS.md">
…the file's text…
</project_instructions>

<project_instructions path="/Users/alex/Projects/shop/packages/web/AGENTS.md">
…the file's text…
</project_instructions>
```

The block opens with that paragraph, then holds the personal instructions,
when the user has any, then each project file, from the repository's root
down to the working directory. Each text is written as it is, whole, and
the attribute is XML-escaped. A block with no personal instructions and no
project file is not written ([When it is sent](#when-it-is-sent)).

## Which files are read

The files are searched in the node's working directory, a child's own
(`crates/agent-server/src/node/mod.rs`), and each directory
above it up to the root of its git repository, the nearest directory that
holds `.git`, as [project skills](skills.md#project-skills) are. Outside a git
repository, only the working directory is searched.

- In each directory, `AGENTS.md` is read when it is a file, and otherwise
  `CLAUDE.md`. One directory gives at most one file, so a `CLAUDE.md` that
  links to the `AGENTS.md` beside it, or copies it, reaches the model once.
- In each directory, `.demi/instructions.md` is read too, after that
  directory's `AGENTS.md` or `CLAUDE.md`: instructions for Demi alone, which
  other agents reading `AGENTS.md` should not follow, such as which
  workflow to use or how to use Demi's browser for this project. It
  reaches the model as a `project_instructions` element of its own path.
- Names are matched exactly. A symbolic link is followed.
- A file that is empty or holds only white space is left out.
- A file the Host cannot read is left out and logged.
- A file's bytes are decoded as UTF-8, an invalid sequence replaced by U+FFFD.

There is no size limit and nothing is cut: every file is read whole, its
size first and then its bytes at that size, read again when it grew in
between, and the replay bound does not apply to the instructions block
([Text bounds](compaction.md#text-bounds)). A large file costs its size in
every request, which the context usage above the card's list includes.

Files in directories below the working directory are not read. Home
directories are not read either: `~/.claude/CLAUDE.md` or `~/.codex/AGENTS.md`
would differ between the Cloud and each device the conversation moves to,
so instructions for every project are the personal instructions, which follow
the user.

## Personal instructions

The user writes them in Settings › Instructions, one text of at most 65,536
characters, which the backend stores per user
([Control records](../backend/storage.md#control-records)). Saving an empty
text removes them. A change reaches the next request of every node, open
conversations included, as a new instructions block.

## When it is sent

The instructions source is the product's, asked before each provider request
after the execution context and before the plugins
([Context](transcript.md#context)). Every node asks it with its own working
directory, a subagent too, a profile's included.

- The project files are kept current by the runner, and a request hardly
  waits for the Host. For each Host and working directory a loaded
  conversation's nodes use, the source searches once, through the
  conversation's host access in the form that never wakes a Host, and
  subscribes with the runner's [file watch](../execution/runner.md#watching-files)
  to each file the search reads or would read, whether it exists yet or not;
  when a subscription says one changed, it searches again in the background.
  A request uses the result held at that moment. Only the first search of a
  directory holds a request, so that a new conversation's first request
  carries its instructions, and for at most 5 seconds; past that, or while
  the Host is stopped or offline, the request uses the last result or none,
  and the block that follows the search reaches the next request. An
  edit to a file therefore reaches the model at the next request. A Host that
  comes online again is searched again; a failed search or subscription is
  logged and the last result kept. The subscriptions are released when no
  loaded conversation uses that directory on that Host. Skills are kept the
  same way ([Project skills](skills.md#project-skills)).
- The personal instructions are read at every request.
- The source answers only when the block it would write differs from the
  newest of its blocks the model receives, so an unchanged block is sent
  once and the prompt cache keeps it. After a compaction its earlier blocks
  are no longer given to it, so it writes the block again.
- When the newest block held something and there is now nothing, it answers
  `There are no longer any personal instructions or project instruction
  files.`

## What the card lists

The composer's context card lists what the newest instructions block of the
root node holds, in a section of its own under the heading Instructions,
the card's last, below the context window's part and the usage limits,
whether the list of rows is open or closed. Each entry is one row in the block's order:

| Entry | Shows | A click |
| --- | --- | --- |
| Personal instructions | `Personal instructions` | Opens Settings › Instructions |
| A project file | Its path from the directory of the block's outermost file, as `AGENTS.md` and `web/CLAUDE.md`; its full path as a tip | Shows the file in the work panel, through the `file` intent ([Files named in messages](../product/file-previews.md#files-named-in-messages)) |
| The conversation's workflow | `Workflow: Review Team`; the workflow file's path as a tip | Shows the workflow file in the work panel, as a project file does |

The card shows no token count for an entry: the Instructions row of the
context window carries their estimate
([What fills the context](compaction.md#what-fills-the-context)). A click
closes the card. Before the root's first request, and when the newest block
says there is nothing, the card has no Instructions section at all: it is as
it was before instructions existed, rather than a line about what is missing.

The card reads what the page already holds: the instructions block
carries, beside its text, the list of its entries (`instructions`, each
`{ kind: "personal" }` or `{ kind: "file", path }`), so the page never parses the model's text and
never asks the backend for anything more. The page holds the newest block
when its latest window does; otherwise a page of the transcript names the
newest block's entries ([Pages](../product/web-api-conversations.md#pages)). The block itself stays hidden from
the transcript, as every context block is.

## Rationale

- **A context block, not the system prompt.** The system prompt stays the
  same for every user and project so its prefix is cached
  ([System prompt](system-prompt.md#the-layers)); OpenCode and Crush put these
  files in the system prompt, and an edit, or a random order, costs the cache.
  Codex, Claude Code and Gemini CLI send them as a user message, as here.
- **One file per directory.** Codex and pi read the first name that exists in
  each directory; reading both, as Crush does, sends a repository whose
  `CLAUDE.md` links to its `AGENTS.md` twice.
- **The git root as the bound.** Codex, Gemini CLI and Goose stop at it.
  Claude Code and pi walk to the file system's root and read unrelated files
  above the project; OpenCode does outside a repository.
- **No cut.** Codex cuts the files at 32 KiB in total, and the others send
  them whole. A cut that drops the end of a project's rules without telling
  anyone is worse than a large request the context usage shows, and a file
  left out for its size is the same loss. A file is the project's to keep
  small.
