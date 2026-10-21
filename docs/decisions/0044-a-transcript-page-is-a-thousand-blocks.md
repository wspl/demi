---
status: accepted
---

# A transcript page is a thousand blocks

Supersedes, in part: 0009 (agent sessions and sync), where a page of a
transcript is bounded by its size, and 0025 (a transcript is shown from a
bounded window), whose window holds a limited number of blocks on each
side.

## Context and Problem Statement

A transcript reaches the page in pages: the latest when a conversation opens,
then one before another as the user scrolls up. Each page holds as many blocks
as fit in 64 KB of their light form. The list then folds a run of steps into
one row, such as "Ran 12 commands", so a page of steps shows as a few rows.

In a long subagent, a page of several dozen blocks showed as a handful of
rows. The first screen was not full, and the user scrolled up and waited again
and again: reaching the last hundred rows took dozens of pages. Across the
largest conversations and the subagents with the most steps, the first page
showed about one screen in the middle case, and less than one in about half
of them.

The window the page holds around what is shown (0025) keeps a few hundred
blocks on each side, and the local copy is read a few hundred blocks at a
time, so a larger page would not fit either.

## Considered Options

* Keep a byte budget and raise it.
* Count shown rows on the server.
* Count blocks: 200, 500 or 1,000 a page.

## Decision Outcome

Chosen option: a page is 1,000 blocks, because a block count is what the
server can cut without knowing how the list folds, and 1,000 fills a screen
even in the most folded subagent measured, at a size that still opens a
conversation quickly.

* Every page of every transcript, the root's, a subagent's or a side
  question's, is 1,000 blocks: the latest, the one before or after a block,
  and the one around a block, which takes half on each side. A page is
  shorter only where the transcript or the session's window ends: the first
  and the last page, a page the local copy holds only in part, and the latest
  page of a conversation compacted less than a page ago, which starts at the
  compaction. There is no byte budget; a large block travels whole in its
  light form.
* Opening a conversation sends the latest page, as every reconnect does.
* The page holds one page on each side of the block shown, and the latest
  page, where 0025's window held a few hundred blocks. The page a read has just
  brought stays whole until the view tells which block it shows next, so a
  window never cuts the page the user is about to reach, or the one they are
  on.
* The local copy is read a page at a time. It is still let go in small
  batches, apart from the page size, since one large deletion holds every
  read back (0009).
* The list asks for the page before when the user is within three screens of
  the top, and for the page after within three screens of the bottom, instead
  of within one. It first tells which block is in view, read from the rows as
  drawn, so the window the page holds follows the reader, not the place they
  were a moment before. Only a window that holds the transcript whole, from
  its start to its end, is told as the latest; any other is held around the
  block in view, even at its end, or the page read above a few folded rows
  would be let go as it arrives.
* The page size is one number, owned by the protocol (`PAGE_BLOCKS`); the
  page's window and the local copy's reads take it from there.
* A loading row shows at the top or the bottom of the window only while a
  page is being read for that edge, never because a page could be read. The
  list keeps its scroll position by the content, never by that row: a page
  that arrives above it moves the rows in view down by its height and the
  view with them, so the reader's place does not change and the top is not
  reached again.

```text
  A subagent of a few thousand steps, opened

  before                                 after
  +-----------------------------+        +-----------------------------+
  |        (loading...)         |        | Work through the long list  |
  | Ran 12 commands           > |        | Ran 2,480 commands        > |
  | Ran 9 commands            > |        |                             |
  |        (loading...)         |        |                             |
  +-----------------------------+        +-----------------------------+
  read after read at the top             a few reads, then still; the
                                         loading row only while reading
```

### Performance

Measured by replaying the light form and the list's folding over the largest
conversations and the subagents with the most steps on a server in use, in
orders of magnitude:

| | Today (64 KB) | 1,000 blocks |
|---|---|---|
| Latest page, light form | tens of KB | about 1 MB |
| Screens of rows on the first page, middle case | about one | about ten |
| Pages to the last hundred rows, worst | dozens | one |

Measured on a made-up conversation of about 3,000 steps with long outputs,
about 16 KB a block, in a development build:

* The server reads a page of 1,000 whole blocks and answers it in its light
  form in about half a second, from a third to three quarters of one; the
  answer is about 0.8 MB. The time is the reading and the trimming of the
  whole blocks, which grows with what each block holds.
* The page reads a page of 1,000 blocks from its local copy, decoding
  included, in about 25 ms.

Estimated, not measured:

* Pages are sent uncompressed, so opening a long conversation sends about
  1 MB where it sent about 70 KB: about 0.8 s at 10 Mbit/s and 0.1 s at
  100 Mbit/s. A short conversation is unchanged, since it is one page either
  way.
* The page holds up to 3,000 light blocks for each open transcript, about
  3 MB, against 900 blocks, about 0.9 MB, today. With the 3 conversations
  kept connected and 8 subagent windows kept, the worst case is about 23 MB
  more memory.
* Rows are drawn virtually, so the rows on screen do not grow; folding is
  computed over up to 3,000 blocks instead of 900 on each change.
* The server reads up to 1,000 blocks from its database for a page instead
  of a few dozen, once per page instead of more than ten times.

### Consequences

* Good, because the first page fills many screens even where steps fold most,
  and scrolling up reads ahead of the user, so the wait between pages is gone
  in the common case.
* Good, because a page is one count, the same for the server, the page's
  window and the local copy.
* Bad, because opening a long conversation takes longer before the first
  screen shows: about half a second on the server for blocks with long
  outputs, and about 0.7 s more at 10 Mbit/s.
* Bad, because the page's memory for a long transcript grows to about three
  times what it is.
