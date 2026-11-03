---
status: accepted
---

# File pills fold to one line

Supersedes: 0196 (File pills show the rest in place, newest first), the part
where the folded pills take two lines.

## Problem

The agent edits nine files in one run. Under the run's folded row, two lines
of pills name them, `auth.ts +12 −3`, `session.ts +4` and so on, before the
next message starts. Alex scrolls the transcript to read what the agent
said; every run that changed files pushes it down by two lines of names
Alex has not asked to see.

## Decision

**Folded, the pills take one line: the files that do not fit count in its
last pill, "+N Files", which shows the rest in place as before. The newest
file always shows, its name cut when it leaves the count no room.**

```text
  auth.ts +12 −3   session.ts +4   login.vue +30 −8   +6 Files
                    ─ click ─▶
  auth.ts +12 −3   session.ts +4   login.vue +30 −8   …
  cookie.ts +1   index.ts +2 −2   …   Show Less
```

- On a phone, a long newest name and "+8 Files" share the line:
  `ConversationList… +120   +8 Files`.
- Show Less shows only while the pills need more than one line.
- Everything else 0196 decided stays: newest first, in place, a pill opens
  its file.

### Design

- `web-ui` `ui/fit-more.ts`: `fitMore` fits one line; its `rows` goes, since
  no caller takes more.
- `web-ui` `FileChangePills.vue`: when not even the first pill fits beside
  the count, it shows alone with the count, as wide as the count leaves it.
- `web-gallery`'s file-change specimens: the one that fits exactly fills one
  line of the phone width.

## Spike

None.

## Prior art

- **Thunderbird**'s message header lists the recipients on one row and ends
  it with "N more", which shows the rest in place
  ([bug 511745](https://bugzilla.mozilla.org/show_bug.cgi?id=511745)).

## Alternatives

- **Keep two lines.** Each run that changed files costs a line more of the
  transcript, and the newest files show first either way.
- **One pill, "9 Files".** The file the agent changed last, which a reader
  most often opens, would take a click more.
- **Only the count when the first pill does not fit beside it.** On a phone
  a long newest name would leave the line naming no file at all.

## Open questions

None.
