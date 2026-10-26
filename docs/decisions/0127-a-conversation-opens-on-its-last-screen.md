---
status: proposed
---

# A conversation opens on its last screen

## Problem

Alex opens a long conversation that the page holds nothing of. The page
reads the root's latest two pages at once, 2,000 blocks, before it shows
anything, though the first screen shows only the last few dozen rows of
them. On a server 370 ms away, that one read takes 3.6 to 4.3 s: the
server reads 2,000 blocks from its store in 1.3 to 2.1 s, and the answer,
megabytes of JSON, takes about 2 s more to arrive and be read. Loading
conversation… stays for all of it.

The two pages come together (`earlier=true`) so that the older page does
not wait for the latest when the reader scrolls up. The reader who scrolls
up pays nothing for it; the reader who only looks at the end pays for both.

## Decision

The first read of a root brings its last 500 blocks, `FIRST_BLOCKS`, and
the page shows them at once. The page then reads the page before them in
the background, as the reader would ask for it, so scrolling up finds it
held as today.

```text
  before:  [ earlier page 1000 | latest page 1000 ] ──── shown after both
  after:   [ last 500 ] ── shown
           [ the 1000 before ] ── read once shown
```

* **The query.** `TranscriptQuery` loses `earlier` and gains `blocks`:
  `?latest=true&blocks=500` reads the last 500 blocks, from the live
  window when the node runs, as `latest` does today. `blocks` takes 1 to
  `PAGE_BLOCKS`; without it the latest page is whole. `FIRST_BLOCKS` is a
  table of `crates/conversation-lane-protocol`, generated to TypeScript
  beside `PAGE_BLOCKS`.
* **The page.** The client's first read of the root, holding none of it,
  asks for `FIRST_BLOCKS`. In the frame after its rows show, the page
  reads the page before them (`before` its start, with its first block as
  the edge), unless the read reached the transcript's start. A
  subagent's read, the offline copy and every later read stay as they are.
* **The log.** The page's `read.answered` gains `bytes`, the size of the
  answer's body, so a slow read tells whether the server or the network
  took the time.
* No stored data changes. A page of the release before reloads into the new
  web app as the backend starts, as after any release; a read it sends with
  `earlier` before then is refused as a field the query does not know, and
  its opening tries again after the reload.

## Spike

* **How many blocks a screen needs.** A row of the transcript groups
  blocks: a work group of 30 commands is one row. In a server's page log
  over three days, the placed lists held 11.6 blocks per row at the median,
  23.3 at the 90th percentile and 43.5 at the 99th. So 500 blocks are 43
  rows at the median and 11 at the 99th percentile: a full screen in nearly
  every case, where most of those rows are one line tall.
* **What a read costs.** On that server the store's reads of 101 to 500
  blocks answered in 160 ms on average, and those of more than 1,000 blocks
  in 2.0 s; the same 2,000-block read takes 172 ms on a developer's laptop.

## Prior art

* Element Web loads 30 events when a room opens and 50 at each step back
  (`INITIAL_SIZE` and `PAGINATE_SIZE` in
  [TimelinePanel.tsx](https://github.com/element-hq/element-web/blob/develop/apps/web/src/components/structures/TimelinePanel.tsx)).
* Zulip fetches 60 messages before the anchor of a narrow, then backfills
  in batches of 1,000 in the background
  ([message_fetch.ts](https://github.com/zulip/zulip/blob/main/web/src/message_fetch.ts)).
* Mattermost loads 60 posts at a time (`POST_CHUNK_SIZE` in
  [constants.tsx](https://github.com/mattermost/mattermost/blob/master/webapp/channels/src/utils/constants.tsx)).

Demi counts blocks rather than messages, since a reply is many blocks, and
reads the page before at once in the background, as Zulip does, rather than
when the reader reaches it.

## Alternatives

* **The latest page alone, without the one before.** Half the read, still
  about 2 s on a far server.
* **A tail cut by bytes.** The server would stop at a byte budget; a block
  count is what the page's windows and the copy already measure.
* **Compressing the answers.** Worth measuring once `bytes` is in the log;
  it would not shorten the server's read of 2,000 blocks.

## Open questions

None.
