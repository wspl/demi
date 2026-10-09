# System prompt

What a node's model reads before the conversation, who writes each part, and
how a capability gets the model's attention without its whole manual. The
transcript, context blocks and replay are the [agent runtime](runtime.md)'s;
the command declarations and `--help` are [Commands](../execution/commands.md)'s;
this document owns what the system prompt holds.

## What the model reads

For example, a user asks a conversation on their Mac to check the sign-in
page of the site they are building. The model has never been told about
`demi browser`'s 49 operations, only that it exists and when it serves:

```text
You are Demi, an agent that does work for the user on their computers …     identity
How Demi works: Hosts and devices, the workspace, files for the user, …      harness guide
The shell tool: its window, command reports, …                     tool rules
Capabilities                                                                 capability index
  demi browser  Drives a real browser on the Host: opens pages, reads …
                Use it when a task needs a live page: a site the user is
                building, a page that needs JavaScript or a sign-in, or
                showing the user how a page looks. Not for fetching a
                static URL, where curl is enough.
                Operations: open show tabs info goto back … screenshot …
                Details: demi browser --help; one operation: demi browser <operation> --help
  demi file     …
This conversation runs on Claude Opus 5.5 (anthropic, claude-opus-5-5). …     model identity
```

It reads the index, runs `demi browser --help`, then `demi browser open --help`,
and opens the page. The help it read is an ordinary command result in the
transcript, so it stays there and costs nothing more when replayed.

## The layers

A node's system prompt is rendered once, when the node is assembled, from
five layers in this order, and holds no time, Host or other state
([Prompt cache](../providers/providers.md#prompt-cache)):

| Layer | Written by | Holds |
| --- | --- | --- |
| Identity | The product; a [subagent profile](subagents.md#profiles)'s instructions replace it | What Demi is and how it works for the user |
| Harness guide | The product | The facts of Demi's world the model cannot infer and that hold in every turn |
| Tool rules | The agent runtime | How the `shell` tool behaves |
| Capability index | Each command group's declaration | One entry per command group of the node, sorted by group |
| Model identity | The agent runtime, from the node's model selection | Which model serves the node |

Everything that changes, or that differs by Host, user or project, reaches
the model as a `context` block appended to the transcript, never as a change
to these layers ([Context](runtime.md#context)): the date, the current Host's
system and working directory, the skills catalog, the user's and the
project's [instructions](instructions.md). Details reach it when it
asks for them: a command's `--help` is a command result.

The whole system prompt stays near 3,000 tokens: the identity a few
sentences, the harness guide at most 800 tokens, the index at most 1,500.
The tools are fixed, so the prefix of every request changes only when the
node's command set changes, with a Demi release, or on a model switch, as
the prompt cache rule allows.

## Identity

The identity says what Demi is, not one job it does: an agent that does work
for the user on their computers, a Cloud machine or the user's own devices,
and reaches the user through its messages and the work panel. Coding is one
kind of that work. Guidance that only holds for software, such as running
the project's tests before calling a change done, is written as working
habits that apply to any work: check the result the way the user would.

The product's instructions are the identity layer. A subagent profile's
instructions replace only this layer; the other four stay, so a profile's
node knows the same harness, tools, capabilities and model.

## Harness guide

The harness guide tells the model how Demi's world works, in the facts it
would otherwise get wrong or spend turns discovering. It covers, in this
order:

- **Hosts.** The conversation works on one Host at a time, a Cloud machine
  or one of the user's paired devices, which can change while it lives; the
  current one's system and directory arrive in a context block before the
  first turn and after each change
  ([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)). Other devices
  of the user are reached with `demi host`.
- **The workspace.** The working directory is the task's workspace, which
  persists across turns. What the model makes, a new app, a download, notes,
  goes in it, as `./signin-app` rather than `~/signin-app` or `/tmp`, unless
  the user names another place.
- **Demi's capabilities first.** A task a `demi` command serves is done with
  it. When one fails, the model takes the step its error names, such as
  installing what it lacks, rather than stopping to report it; only when
  nothing it can do fixes the failure does it tell the user why, and it asks
  before reaching for an outside service that would put the user's work on
  the internet, such as a public tunnel.
- **Parallel work.** A task with several independent parts, each taking more
  than a quick step, such as several sites, pages, libraries or files to look
  into, goes to helper agents with `demi agent`, one part each, run at the
  same time; the model then combines their reports. It does the parts itself
  only when each is a single quick step. The capability index entry says what
  the group does; this rule says when the work is split, because a model
  reading only the entry weighs the split against doing the work itself and
  keeps choosing to do it alone: the five-site pricing comparison took 140
  requests and six million input tokens in one context.
- **Files for the user.** Replies render as Markdown. A file the user should
  keep goes out as an attachment
  ([Attachment commands](../execution/commands.md#attachment-commands)); a
  Host path in a link or image shows the file as it is now; a path in code
  stays text. Images and videos are embedded, other files linked.
- **What the user sees.** The work panel beside the conversation shows the
  conversation's changes, files and browser tabs; the user watches and
  operates the browser there.
- **Permissions.** A command refused for the user's permission has asked the
  user in the app. The model says what it needs the permission for, goes on
  with what it can do without it, and does not run the command again until
  the user grants it.
- **Context blocks and attachments.** A `context` block is a fact the
  application supplies, not the user's words; the instructions block is the
  one that holds instructions to follow, the user's personal ones and the
  project's `AGENTS.md` or `CLAUDE.md` files
  ([Instructions](instructions.md)). A file the user attaches arrives as an
  `<attachment>` tag naming its path on the Host.

A rule belongs here only when its moment arises during ordinary work rather
than while the model reads one command's help; otherwise it belongs in that
command's help. For example, "a file the user should keep goes out as an
attachment" arises while working, so it is here; "a screenshot's coordinates
are CSS pixels" arises while reading `demi browser screenshot --help`, so it
is there.

## Capability index

Each top-level command group's declaration carries its index entry, and
registration refuses a group without one, since a group the index does not
name is one the model never finds. The index
lists every group of the node's command set, sorted by group path, each with
its entry, the names of its operations, and where its details are. It opens
with the paragraph of conventions every command follows unless its help says
otherwise, such as how stdin bodies are passed and where output goes
([Help](../execution/commands.md#help)). The
renderer writes the operations' names and the pointer to `--help`; the entry
is the declaration's text, at most 600 characters, refused at registration
when longer.

An entry is the most important text a capability has: a model that does not
see from it that the capability fits the task never asks for its help. It is
written in three sentences or fewer, in this order:

1. **What it does,** in the third person, in the words of the task, not of
   the implementation: "Drives a real browser on the Host", not "A CDP
   client".
2. **When to use it,** as the situations a user would describe: "a site the
   user is building, a page that needs JavaScript or a sign-in, showing the
   user how a page looks".
3. **When not to,** where a common alternative is better: "Not for fetching a
   static URL, where curl is enough."

The use comes first because it is what decides. An entry gives reasons and
situations, never capital letters or "you must": a newer model follows
emphasis too eagerly and reaches for the capability where it does not fit.

For example, `demi attachment`'s entry: "Gives the user files from the Host
as attachments of the conversation, which stay viewable whatever becomes of
the file. Use it to deliver a screenshot, a recording, a download or a file
you made; embed it in your reply as `![…](attachment:a3)`. A file the user
should see as it changes on the Host is linked by its path instead."

A group whose whole help is under about 1,000 tokens and that nearly every
conversation uses may list its operations' usage lines in place of their
names; no group does today.

## Details on demand

`demi <group> --help` lists the group's operations, each with its one-line
summary, and `demi <group> <operation> --help` gives the operation's full
usage, as [Help](../execution/commands.md#help) renders them. Help is
deterministic, so the same read replays the same bytes. A model that needs a
group reads its help once; after a compaction it reads it again when it
needs it.

A command whose result is large writes it to a file on the Host and prints
the path, as `demi browser screenshot --output` does, rather than returning
it whole.

## Model identity

The last line of the system prompt names the model that serves the node,
generated from the node's model selection: its display name in the catalog,
its provider family and its id, and that this is the answer when the user
asks which model it is. It is never written by hand: a name that does not
follow the configuration makes the model claim to be another. A model switch
renders it anew, which the prompt cache rule already allows.

## What a plugin contributes

A plugin contributes to what the model reads only through its declarations:

| Contribution | Where it goes | Limit |
| --- | --- | --- |
| An index entry for each command group it declares | The capability index | 600 characters, refused at registration |
| Its commands' help | `--help`, on demand | None beyond the declaration's |
| A context source | Appended `context` blocks ([Prompt text and context](../architecture/plugins.md#prompt-text-and-context)) | Its own block's size |
| Skills | The skills catalog, a context block ([Skills](skills.md)) | The catalog's |

A plugin never writes free text into the system prompt, and it never orders
or places its entries: the renderer does. A plugin's design states what its
index entry says and why the model would reach for it from that sentence.

## Acceptance

Automated tests never call a model, so the index is accepted against the
running product. A fixed set of tasks, each needing one command group and
worded as a user would ask without naming the command, runs on each model
family the product offers; for each it records whether the model used the
right group, the turns and input tokens, and whether the task succeeded. An
entry is rewritten until its group is found; the set includes tasks for
every group in the index.

The tests check the rendering: the layers' order, the index sorted and
listing every group of the node, an over-long entry refused at registration,
the model line following the node's selection, and the same bytes for the
same node.

## Rationale

- **An index, not the manuals.** The browser group's help alone was 64 KB,
  about 16,000 tokens, in every request. With fewer definitions in front of
  it a model chooses better: Anthropic's deferred tool search raised tool
  selection from 49% to 74% on Opus 4 with 85% fewer definition tokens, and it
  recommends loading details on demand past about 10,000 tokens
  ([Advanced tool use](https://www.anthropic.com/engineering/advanced-tool-use)).
  But a capability the model must think to look up is often never used: in
  Vercel's evaluation an on-demand skill was not invoked in 56% of runs, while
  an always-present index of what exists and where reached 100%
  ([Vercel](https://vercel.com/blog/agents-md-outperforms-skills-in-our-agent-evals)).
- **Rules in code, not prose.** A rule that must hold, such as a permission,
  an output cap or a timeout, is enforced by the command or the shell tools;
  prose only explains it. Measured harness changes gained from tools and
  middleware, not from prompt text alone.
- **The model's own name.** Without a system prompt most models misname
  themselves at least once, and a hard-coded name made a harness's model
  claim to be another; the line is generated from the configuration.
