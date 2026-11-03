---
status: proposed
---

# A turn goes on after a filtered reply, and the model knows why

## Problem

A model writes a mockup page with `cat > menu.html <<'EOF' ...`, and the
vendor's safety classifier stops the reply there: a refusal, in Anthropic's
words (`stop_reason: "refusal"`), a content filter in OpenAI's, a safety block
in Google's. Demi calls such a reply filtered. What follows depends on the
provider:

- **The Anthropic API, OpenAI and Google** end the turn with a failure record,
  "The response ended early: The provider's filter stopped the reply
  (refusal).", or the vendor's own reason in place of `refusal`. The user
  presses Resume, and the model reads only "Continue from where you left
  off.": it never learns that its reply was filtered. A long task stops at
  every filtered reply until the user comes back.
- **Claude Code** never tells Demi. Demi hands the agent the message's calls
  at its `message_stop` without reading how the message ended, and the agent
  runs them. Meanwhile the CLI goes on by itself: it gives the call the result
  "Interrupted: ... it may have partially or fully completed" and tells the
  model "Do not produce that content again, even reworded". Demi's transcript
  holds the call's real result instead, so a later process, which resumes
  from Demi's transcript, gives the model a history it never saw. Nothing
  limits how many filtered replies a turn takes. The usage ledger records the
  filtered request as answered.

A filtered reply is rare and usually passes: in a long conversation, the
requests between two filtered replies succeed.

## Decision

A filtered reply ends the request, whatever the provider, and Demi resumes
the turn at once, as if the user had pressed Resume, telling the model why.
The fourth filtered reply of one turn ends the turn instead, and Resume then
goes on.

What the model reads after a filtered reply, in a request of the same turn:

```text
assistant  Writing the mockup.
           [tool_use shell: cat > menu.html <<'EOF' ...]
user       [tool_result, error] Tool call aborted: The provider's filter
           stopped the reply (refusal).
user       The provider's safety filters stopped your previous reply.
           Continue from where you left off.
```

What the user sees: the reply's text and its aborted calls, as they streamed,
then Requesting while the next request waits, and the turn going on. After
the fourth filtered reply, the failure record:

```text
  ╭──────────────────────────────────────────────────────────╮
  │ ⊗ The provider’s safety filters stopped this reply.      │
  │   › Provider Response                              Copy  │
  ╰──────────────────────────────────────────────────────────╯
```

- **A new error code, `filtered`.** `EarlyEnd::Filter`, every provider's
  refusal, content filter or safety block, gives `ErrorCode::Filtered` instead
  of `Incomplete`. It is not retried, as `incomplete` is not, since the same
  request would be filtered again.
- **The turn resumes.** When a request fails with `filtered` and the turn has
  gone on after fewer than three filtered replies, the session completes the
  calls the reply did not run as aborted, with the failure's message, appends
  a `resume` block whose reason is "The provider's safety filters stopped your
  previous reply.", and goes round the turn's loop again, as after a step: it
  waits as Requesting, lands a pending model switch, writes the waiting input,
  steers included, and requests. It records no failure. The limit exists
  because a history the classifier objects to is often filtered again, and
  each filtered reply is a paid request; three is as many as a long turn
  meets.
- **The fourth filtered reply** ends the turn with the failure record. Resume
  unwinds the record, as for any failure, and its `resume` block carries the
  same reason, so the model knows of that one too.
- **A session copy**, which sends one request for a compaction summary, does
  not go on: its filtered summary fails the compaction, as before.
- **Claude Code.** The provider reads the stop reason of every message: from
  `message_delta` while one streams, and from the whole `assistant` line of a
  message the CLI fetched without streaming, as it does after three failed
  streams. When a message ends with `refusal`, the provider hands over no
  call, gives the session the calls it read, closes the process, so the CLI
  does not go on by itself, and ends the run with
  `ProviderFailure::ended_early(EarlyEnd::Filter, "refusal", frame)`. The
  request's ledger row is refused with `filtered`. The next request starts a
  process that resumes from Demi's transcript. A message fetched without
  streaming comes as one line per block, and the provider stops at its first
  line, so its later blocks, calls among them, are not kept: the model still
  reads why its reply stopped.
- **The record's headline**, `errors.filtered`, says the sentence above, and
  the vendor's own words, such as its category, are under Provider Response.
  An older page has no words for the code: it shows "The turn failed." and
  the response.
- The gallery's failure records gain a specimen of it.

## Spike

- **Streaming.** A local stand-in for the Messages API answered a Claude Code
  2.1.296 process with a text block, a `tool_use`, `content_block_stop`, then
  `message_delta` with `stop_reason: "refusal"` and `stop_details`, and
  `message_stop`. The CLI started the call at `content_block_stop`, then wrote
  the "Interrupted" result and its note, and asked the model again.
  `--include-partial-messages` prints each frame, so the provider sees the
  stop reason before `message_stop`.
- **Without streaming.** The stand-in broke three streams, and the CLI fetched
  the message without streaming. It printed the message as whole `assistant`
  lines, one per block, each with `stop_reason: "refusal"`, and no
  `message_delta`. With an MCP tool, the CLI sent `tools/call` only after
  those lines, filtered or not, so the provider reads how the message ended
  before any call of it opens.
- **The API path.** An Anthropic-protocol Compatible API entry, pointed at a
  stand-in that stops replies inside a shell call, ran no command; the call
  showed "Tool call aborted: The provider's filter stopped the reply
  (refusal)."

## Prior art

- **Claude Code** goes on by itself after a refusal, with no limit, and tells
  the model not to write that content again. Demi departs in the words: the
  model is told what happened and decides.
- **Anthropic's documentation**
  ([Streaming refusals](https://platform.claude.com/docs/en/test-and-evaluate/strengthen-guardrails/handle-streaming-refusals))
  says the vendor adds no message of its own, so the client tells the user,
  and that refusals can keep coming until the history changes, which is why
  the turn stops after a few.

## Alternatives

- **Let the Claude Code CLI go on by itself.** Its calls would run while the
  CLI tells the model they may not have, and Demi's transcript would differ
  from what the model read.
- **End the turn at every filtered reply.** A long task would wait for the
  user each time, though the next request usually succeeds.
- **No limit.** A history the classifier keeps filtering would spend requests
  until the user stops it.

## Open questions

None.
