---
status: accepted
---

# A reply keeps the pictures it shows

## Context and Problem Statement

A reply can show a picture from its Host by naming the file's path, as
`![chart](out/chart.png)`. The page reads the file from the Host each time
it shows the reply: first its description, then its bytes, through the
backend. So the picture shows late, after a placeholder a fifth of its
height; it is missing when the Host is offline or the conversation is
archived; it wakes a stopped Cloud; it changes or disappears when the file
does, though the reply was written about the picture as it was; and a shared
page never shows it.

An attachment the agent uploads with `demi attachment upload` has none of
these problems: the backend keeps a copy, with its size, and the reply shows
the copy.

## Considered Options

* Keep reading the Host, and keep a size once it is known.
* Copy each picture a reply names when the reply ends, as an attachment.

## Decision Outcome

Chosen option: copy it when the reply ends, because a reply then shows what
the agent saw when it wrote it, from the backend alone, the way an
attachment already does.

* When an agent's message is written to its end, the backend copies each
  Host picture its text shows, from the Host the agent ran on, into the
  conversation's attachments, with its size, as `demi attachment upload`
  would. The message shows the copies; the text keeps the path the agent
  wrote.
* The copies are made in the background: the agent's turn and the next
  request never wait for them. Until a copy is in, its picture shows as a
  placeholder, since its size is known only once it is read. Each copy is an
  attachment of its own; its bytes are stored once whatever names them.
* A picture that cannot be copied, because the file is missing, too large
  for an attachment, or the Host went away, shows as a link to its path with
  why, and is not read again.
* Opening a copy shows the copy in the viewer, as opening any attachment
  does (0032).
* A reply written before this change, or a message that is not written by
  the agent as it ends, such as a subagent's brief or a model's thinking, has
  no copies and none are made for it: its Host pictures show as links to
  their paths. A reply shown again, as when the user switches to another
  version of a message, is not copied again.
* The page and the backend find a reply's pictures by one rule, so a copy is
  always found for the picture it was made of.

```text
  A reply that shows three pictures, one of them missing

  | Here is the chart, and the page as it looked:                 |
  | +-------------+  +-------------+                              |
  | |  chart.png  |  |  page.png   |   (copies, at their size)    |
  | +-------------+  +-------------+                              |
  | out/old.png (The file isn't on the host)                      |
```

### Performance

* Measured on a server in use: pictures named by replies are rare, a few
  dozen in the largest conversation and none in most; copying them would
  store a few megabytes a month.
* Measured on a paired Host: copying a 1 MB or a 10 MB picture took about
  70 ms; one already stored took about 30 ms.
* Risk: a Host on a slow link sends a 10 MB picture in about two minutes at
  750 Kbps, during which the reply shows its placeholder; nothing else waits.
* Removed: a description and a byte read from the Host each time a reply is
  shown, and every wake of a stopped Cloud to show one.

### Consequences

* Good, because a reply's pictures show with the reply, at their size, on a
  shared page and in an archived conversation, and never change afterwards.
* Bad, because each picture a reply shows is stored once more, counted as an
  attachment.
