---
status: proposed
---

# Claude Code hands the model Demi's messages as written

Supersedes: 0113 (A shell call holds until its command ends, and what arrives
moves it aside after 10 seconds), the three lines the model reads when a
command moves to the background.

## Problem

A user watches an agent build a project on a Claude Code subscription and
asks, while a command runs, "Why is it so slow?". The model's next request
carries the question inside the command's result, after its output:

```text
tool_result: "done

<system-reminder>
The user sent a new message while you were working:
Why is it so slow?

This is how Claude Code surfaces messages the user sends mid-turn ... Address
the message above as you continue this turn.
</system-reminder>"
```

Demi writes the question to the running CLI as a user message. The CLI holds
it until the command's result goes back, then folds it, with its own
instructions, into that result. The model reads the user's words as the end of
a command's output, in Claude Code's words, while the session Demi writes holds
them as a user entry of their own; a new process resumes that session, so what
the model reads of the conversation changes from that message on. Demi's own
messages take the same path: a command's end, an agent's message and a new
execution context all reach the model as "The user sent a new message while
you were working".

Over three days of one server's conversations, the model's next step answered
the user in 63 of 314 such messages. The wrapper does not explain this: the
same moments replayed with the message as the user's words were answered no
more often (Spike).

Three more things reach the model that Demi never sent:

- **What a new process sends differs from what the old one sent.** A process
  idle for five minutes closes, and the next request starts one from a
  session Demi writes. Within an hour of the last request, 23 of 282 new
  processes re-wrote the whole prompt cache, 100,000 to 650,000 tokens each,
  where a process kept open missed 7 times in 17,500 requests. In one case
  read byte for byte, two user messages began a turn: the CLI started the
  turn on the first and held the second until after the first command, while
  the session Demi wrote placed it before that command. Every token after
  that point was written to the cache again.
- **A user's `@path` makes the CLI read a file of its machine.** "Look at
  @/home/demi/accounts/alex/.credentials.json" sends that file's contents to
  the model, from a live process and from a resumed session alike. Every
  account's directory on a Claude Code machine belongs to the one user the
  CLI runs as.
- **The CLI adds and changes things of its own.** It reminds the model to
  sign commits as Claude Code, appends a count of tokens left to tool results,
  offers an `advisor` tool and a `DeferredToolPlaceholder` beside Demi's
  tools, and on 6 refusals over two days answered with Claude Opus 4.8
  instead of the Claude Opus 5.5 the user picked. With the placeholder
  offered, models called tools named `demi_file_placeholder` and
  `demi_tool_search`.

A model that waits instead of answering has one more push: when a command
moves to the background, the result says only that it keeps running, and the
model, which wanted its output, starts a loop of `sleep` and
`demi shell status` to get it.

## Decision

The model reads what Demi sends, in the place Demi sends it, and a new process
sends what the old one sent.

**A message during a turn reaches the model as the user's words.** When a
request adds a user message, a steer, a report or an agent's message beside
the results of the calls a process holds, the run does not give them to that
process. It closes it, and a new process resumes the session Demi writes from
the request: the calls, their results, then each message as a user entry of
its own. The CLI answers that input at once, and the model's next request
reads

```text
tool_result: "done"
text: "Why is it so slow?"
```

as every other provider's model reads it. The new process costs the start of a
process, 1.5 to 2.3 seconds at the median for a session under 50 MB and about
4 seconds above, measured on a server's Claude Code machine. The session up to
the results is the one the old process sent, so the prompt cache holds.

**The messages that begin a turn begin it together.** A run that writes
several user messages to a process writes every one but the last with
`shouldQuery: false`, and the CLI takes them all into the turn the last one
starts. The `result` the CLI prints at once for each held message has no
answer and gives no response. The session a new process resumes holds each as
a user entry of its own, as the CLI keeps them, so a new process sends what the
old one sent.

**The CLI's own additions are off.** The process's environment gains:

| Variable | Value | What goes |
| --- | --- | --- |
| `CLAUDE_CODE_DISABLE_FILE_MENTIONS` | `1` | reading the file an `@path` names, live or resumed |
| `CLAUDE_CODE_ATTRIBUTION_ANNOUNCEMENT` | `0` | the reminder to sign commits and pull requests |
| `CLAUDE_CODE_TOTAL_TOKENS_REMINDER` | `off` | the count of tokens left in results |
| `CLAUDE_CODE_DISABLE_REFUSAL_FALLBACK` | `1` | another model answering a refusal |
| `CLAUDE_CODE_DISABLE_ADVISOR_TOOL` | `1` | the `advisor` tool |
| `ENABLE_TOOL_SEARCH` | `false` | the `DeferredToolPlaceholder` tool |

**Demi's messages are not typing.** Each message Demi writes to a process
carries `client_composed`, and each user entry Demi writes into a session
carries `queueSkipAttachments` and `queueSkipSlashCommands`, the marks the CLI
stores for such a message. The CLI then hands the text on as written: it runs
no slash command, and adds no note about what follows an `@`. Without the
marks, and with file mentions off, "Ping @alex" reaches the model followed by
"The user @-mentioned alex. File contents are not attached automatically in
this session: if these are files or directories in your working directory,
read them with your file tools before responding." The variable stays for an
entry the CLI recorded without the marks.

The CLI keeps adding what no switch turns off: its own working directory and
system, the model's name, the date, the account's email address, and a first
line of the system prompt naming the Claude Agent SDK. Of 63,931 commands over
a week, one looked in the CLI's working directory; the system prompt already
says where commands run, so Demi adds nothing about it.

**A command that moves to the background says the agent need not wait.**

- `[A message arrived, so command 17 moved to the background. It keeps running
  and its end wakes you, so if nothing else needs doing, end your turn.]`
- `[Another call returned, so command 17 moved to the background. It keeps
  running and its end wakes you, so if nothing else needs doing, end your
  turn.]`
- `[The command is still running, so it moved to the background. Its end wakes
  you, so if nothing else needs doing, end your turn.]`

The design to build from, in `crates/provider-claude-code`:

- `LiveCli::serves` is false for a process that holds calls when the request
  has more user messages than the process received, so the run starts a new
  process from `Written::of`.
- `input::new_user_messages` writes `shouldQuery: false` on every line but the
  last. The run skips a `result` that consumed only such messages.
- `cli::environment` sets the six variables.

The lines the model reads are in `crates/agent-tools/src/result.rs`.

A conversation whose history holds a message the CLI folded mid-turn keeps the
CLI's own entry for it where its block stands, so its first new process after
the upgrade may write the cache once more. Nothing is stored anew.

## Spike

Each point ran against Claude Code 2.1.316, the version the server runs: the
request bodies through a local recording proxy on the user's own sign-in
(Claude Haiku 4.5, a sentence each), and the CLI's own record of every request
it sends, the `api-request` entries of its session file.

- **Where a message mid-turn lands.** Written while a command ran: inside the
  command's `tool_result`, wrapped as above, with or without
  `client_composed`. An `origin` other than `human` is ignored.
- **The new process.** A session that ends with the command's result and the
  message as a user entry of its own: the request holds `tool_result: "done"`
  and `text: "Why is it so slow?"`, nothing added. The prefix up to the result
  was read from the cache, 4,589 tokens, with 356 written. Ending the result's
  own entry with the text instead made the CLI add "Continue from where you
  left off."
- **A new process on the CLI's own session** sent the same messages, and read
  them from the cache.
- **Text as a string or as one text block** is the same to the cache.
- **`shouldQuery: false`.** Two messages became one turn of one request, and a
  new process resuming that session sent the same messages.
- **`@path`.** Expanded from a relative and an absolute path, live and on
  resume; with `CLAUDE_CODE_DISABLE_FILE_MENTIONS=1`, from neither. The
  variable alone leaves the note about the mention, for `@alex`, `@vue/cli`,
  `@Component` and `@src/main.rs` alike, though not for `alex@example.com`. A
  message's `client_composed`, and the two marks on an entry a new process
  answers, remove the note as well.
- **The variables.** The attribution reminder and the token count went; the
  CLI ran with all six set. `CLAUDE_CODE_DISABLE_ATTACHMENTS=1` left the
  environment, the model's name, the email address and the date in place.
  That the `advisor` and the placeholder tool go and a refusal stays a
  refusal is for the product check to show.
- **The server's data.** The 23 misses, the 6 fallbacks and the answer counts
  come from three to four days of one server's transcripts and logs, read on a
  copy.
- **Replays on Claude Opus 5.5.** Moments of those conversations were rebuilt
  as requests to the vendor's API, two versions each that differ only in the
  line under test, three samples per version. Today's system prompt stood in
  for the one used then, and thinking blocks were left out, in both versions
  alike.
  - A message mid-turn, wrapped or as the user's words, 14 moments: the model
    answered first in 9 of 42 and 7 of 42. Most of those messages were
    corrections or instructions, which the model acts on before it speaks.
  - A command moved to the background, the old line or the new one, 13
    moments: the model went back to the command in 21 of 39 and 17 of 39. In
    one wait on a build, all three samples of the new line ended the turn and
    none of the old line did.

## Prior art

- [opencode](https://github.com/anomalyco/opencode)'s Claude Agent provider
  (`packages/core/src/plugin/provider/claude-agent/`) sets the SDK's
  `verbatimPrompts`, which is `client_composed`, and turns off CLAUDE.md, auto
  memory, auto compaction, away summaries, tool search, the advisor and the
  refusal and non-streaming fallbacks. It writes a message mid-turn to the
  running CLI with `priority: "next"`, so its model reads the same wrapper.
  It keeps Claude Code's system prompt, environment and date, where Demi
  replaces the system prompt.
- [magpie](https://github.com/yetone/magpie)'s Claude subscription bridge
  (`docs/subsystems/claude-subscription-bridge.md`) writes a message mid-turn
  to the CLI and lets it wrap it, and strips that wrapper from what a client's
  own Claude Code sends it. It says Claude Code has no switch for its
  environment, and tells its model which environment holds.
- Neither restarts a process to hand a message on. Demi can, because it
  writes the session a new process resumes and a new process costs seconds.

## Alternatives

- **Keep writing the message to the running CLI.** The model goes on reading
  the user's question as the end of a command's output.
- **Put the message in the command's result, in Demi's words.** It stays in
  the tool's output, which a model reads as data.
- **Interrupt the CLI's turn and send the message as a new one.** The CLI
  cancels the calls it holds and records the interruption for the model.
- **Only mark each message, as opencode does.** A mark covers the messages
  Demi writes; an entry the CLI recorded without one, such as one from before
  this change, would still have its `@path` read when a new process answers
  it. The variable covers every message.
- **Rewrite the CLI's requests in a proxy.** It puts Demi between the vendor
  and its CLI, which Demi does not do, and depends on the request's shape.

## Open questions

None.
