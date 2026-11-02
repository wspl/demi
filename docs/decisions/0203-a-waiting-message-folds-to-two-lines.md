---
status: proposed
---

# A waiting message folds to two lines

Supersedes: 0186 (A waiting message holds its controls), the part that has a
long waiting message fold where a sent one does: it folds at two lines now.

## Problem

While the agent works, a schedule queues an eight-line brief: "Watch the
build log and report only failed steps…". The queued bubble folds as a sent
message does, at about 200 px, so it takes nine lines of the transcript,
with its row of controls under them, and the agent's work above it scrolls
away. A waiting message is there to be sent or recalled; the user needs to
know which message it is, not to read all of it.

## Decision

A message that waits to be sent, a steer or a queued one, folds once its
text runs past two lines: its first line shows whole, its second fades out,
and Show More, first in the row of controls it already has, opens it whole.
A message of one or two lines shows whole. The lines count from its first
text, so a comment on a picture shows the picture whole above them. As it is
sent, it folds by the sent message's rule, which shows a message under
300 px whole.

```text
                    ┌──────────────────────────────────────────┐
                    │ Watch the build log and report only the  │  whole
                    │ failed steps, with the line that failed. │  fading
                    │ Show More | Send  Recall        = Queued │
                    └──────────────────────────────────────────┘
```

`UserBlock` measures the fold as before; for a `pending` message, the mark
is two line heights below the top of its first line of text instead of
`FOLDED_PX`, and a line across it still shows whole.

## Spike

None.

## Prior art

- Codex's terminal UI previews each pending message in at most three lines,
  then an ellipsis
  ([`pending_input_preview.rs`](https://github.com/openai/codex/blob/67f0994b532b8740f14b5b738af7b7b2a915b1d6/codex-rs/tui/src/bottom_pane/pending_input_preview.rs)).
  Demi keeps two, since its bubble adds a row of controls the terminal does
  not have, and fades the last line as its sent messages do, rather than an
  ellipsis.
- Mail on macOS and iOS previews a message in the list in two lines, and
  opens it whole on demand.

## Alternatives

- One line, cut with an ellipsis: a short brief's start often says too
  little to tell two waiting messages apart.
- Three lines, as Codex: with the row of controls, a bubble of four rows.
- The sent message's fold: what this record replaces.
- Two lines from the bubble's top: a comment on a picture showed a strip of
  the picture and none of its words.

## Open questions

None.
