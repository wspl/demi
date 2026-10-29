---
status: accepted
---

# A transcript times every part of a turn

## Problem

Alex's turn took 2 minutes 14 seconds, and Alex wants to know where the time
went: waiting for the model, its thinking, the commands, or Demi itself
between them. The transcript cannot say.

Each block keeps one time, `created_at`, when it entered the transcript, and
only a tool call also keeps when it returned (`ended_at`):

- A thinking or text block keeps when its first piece came, not its last.
  The page guesses the end of thinking from the start of the block after it.
- A request keeps neither when Demi sent it nor when the model's first output
  came. The session measures both while the request streams (`StreamedOutput`
  in `agent-session`) and drops them when it ends.
- A tool call enters the transcript when its input is complete, so the time
  the model spent writing a long call's input counts as the step before it.
- A compaction keeps when it ended, not when it began.

Some of these times reach the backend's log (`provider_call.ended`,
`compaction.ended`, Claude Code's `claude_code_run`), but the log rotates and
cannot be joined into one conversation's timeline.

## Decision

The transcript keeps when each part of a turn started and ended, as
timestamps on the blocks that already stand for those parts. Durations are
derived from them, never stored.

| Block         | Starts                    | Ends                          |
| ------------- | ------------------------- | ----------------------------- |
| `thinking`    | `created_at`, first piece | `ended_at`, last piece (new)  |
| `text`        | `created_at`, first piece | `ended_at`, last piece (new)  |
| `tool_call`   | `writing_started_at` (new), the model began writing it; `created_at`, its input was complete | `ended_at`, it returned |
| `response`    | `first_sent_at` (new), its first attempt left, when it was tried again; `sent_at` (new), the request left Demi; `first_output_at` (new), the first thinking, text or call came | `created_at`, the response ended |
| `compaction_marker` | `started_at` (new), the pass began | `created_at`, the pass ended |

`ended_at` of thinking and text moves with each piece that comes, so it is
right however the stream ends: done, stopped, failed or cut by a crash. A
call runs once its response ended, so its run counts from the later of its
`created_at` and its response's end. An attempt that failed and was tried
again leaves no block, so the response that answered keeps when the first
attempt left.

`demi conversation history timing <range>` prints where the time of each turn
that starts in the range went, one row per turn, in milliseconds, as every
`demi` command prints a list (`crates/shared-types/src/output.rs`), and
`--json` prints the same rows:

```text
$ demi conversation history timing 4213..
TURN  STARTED               TIME(MS)  REQUESTS  PREPARING(MS)  WAITING(MS)  THINKING(MS)  TEXT(MS)  CALLS(MS)  TOOLS(MS)  COMPACTING(MS)  RETRYING(MS)  OTHER(MS)
4213  2026-10-14T09:12:03Z  134200    6         2100           9800         31000         18200     6500       62300      0               4100          200
```

- **preparing:** Demi's own time before a request left, after what ended
  last.
- **waiting:** from a request leaving to its first output.
- **thinking**, **text** and **calls:** the model writing each.
- **tools:** the calls running, from their response's end.
- **compacting**
- **retrying:** failed attempts and the waits after them.
- **other:** the time no part covers.

Time two parts share, such as parallel calls, counts once, so the parts add
up to the turn's time and a gap in the timing shows as other instead of
hiding. A turn is the blocks of one turn id, read to its end when that lies
after the range; what ran outside it, a compaction while the conversation
was idle or a failure outside any turn, is not its time. Fifty turns print
at a time.

The page reads the recorded ends: a thinking row says how long it thought,
and a run says its time, from `ended_at` instead of the next block's start.

### Design

- `shared-types`: the new fields, each `Option<Timestamp>`. Transcript block
  format 17.
- `agent-transcript`: `TranscriptLog` stamps `ended_at` as it appends a piece
  of thinking or text, and takes the other new times as arguments.
- `agent-session`:
  - `WritingCall` keeps when its first piece came, which the call's block
    gets as `writing_started_at`.
  - `StreamedOutput` gives `sent_at`, `first_sent_at` and `first_output_at`
    to the response block. The speed keeps counting from the first output
    that adds to the count, `first_counted_at`.
  - A compaction pass gives the marker when it began.
- `conversation-lane-protocol`: an `append_text` patch carries when its
  latest text came, which the page's copy of the block takes as its
  `ended_at`.
- `backend-database`: reads a format 16 record with the new fields null. A
  tool call of format 16 entered the transcript when it ran, as today, so its
  times mean what they did. The conversation schema names format 17, by a step that
  changes no row.
- `backend-user-shard`: the `timing` command of `demi conversation history`
  derives the parts from a range of blocks.
- `web-ui`: `ThinkingFace` and `runTime` read `ended_at`; a thinking block
  that does not record its end says Thought, with no time.

A request Demi did not send itself, such as a later model call Claude Code
makes within one run, left once the response before it ended and Demi handed
the CLI the last result of that response's calls: that is its `sent_at`.

Steps inside a request that only one provider has, such as starting the
Claude Code process, stay in that provider's log.

A block written before this change shows no thinking time.

## Spike

None.

## Prior art

- opencode keeps times on each part of a model step
  ([`session-message.ts`](https://github.com/sst/opencode/blob/dev/packages/schema/src/session-message.ts)):
  a step's `created`, `streamed` and `completed`, a tool's `created`
  (writing began), `ran` and `completed`, and reasoning's `created` and
  `completed`. Text keeps none.
- Codex records `started_at_ms` and `completed_at_ms` for every item of a
  turn, and a turn's `duration_ms` and `time_to_first_token_ms`
  ([`protocol.rs`](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/protocol.rs)).
- Langfuse keeps a model call's `start_time`, `completion_start_time` and
  `end_time`
  ([observations](https://langfuse.com/docs/observability/data-model)).
- OpenTelemetry's generative AI conventions measure a call's
  `gen_ai.client.operation.duration` and
  `gen_ai.client.operation.time_to_first_chunk`
  ([metrics](https://opentelemetry.io/docs/specs/semconv/gen-ai/gen-ai-metrics/)).
  The Vercel AI SDK records `msToFirstChunk` and `msToFinish`.

Demi keeps the times on the blocks it has, as opencode and Codex keep them on
their items, rather than adding a container per model call: the `response`
block closes a request and takes that request's times. Unlike opencode it
also times text, and unlike Claude Code's `thinkingDurationMs` it stores
timestamps, from which the gaps between parts can be derived too.

## Alternatives

- **A separate table of spans, as a tracing system keeps them.** It would be a
  second record of what the transcript already holds, and would have to
  follow the transcript through every fork, edit and share.
- **The log only, as today.** It rotates, and joining a turn's lines is a
  search, not a reading.
- **Durations instead of timestamps.** A duration loses where a part sits, so
  the gaps between parts, Demi's own time among them, cannot be found.
- **A `response` block for a failed or stopped request too.** It would time
  those requests, but change what a response block means to every reader. A
  failed request keeps its failure's time on its `error` block, and the rest
  in the log.
- **Timing the parts in the usage ledger.** The ledger counts a vendor's
  calls for billing, across conversations; it does not know the thinking,
  text and calls of a turn.

## Open questions

None. The page shows no breakdown of a turn: the command is the only reader
of the parts, and a block written before this change shows no thinking time.
