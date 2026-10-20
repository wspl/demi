---
status: accepted
---

# An attachment opens the file it was uploaded from while that file is there

## Context and Problem Statement

An agent gives the user a file as an attachment, a copy taken when it was
uploaded. When the agent changes the file and gives it again, the
conversation holds a new attachment for each version, and an older one opens
a version that is no longer the file. Choosing between an attachment and a
link to the file is no answer: whether a file is delivered, or where it lies,
does not tell which one the user needs.

This record covers what opening an attachment shows. It leaves out how an
attachment looks in a reply and what the agent is told about attachments.

## Considered Options

* An attachment keeps where it came from, a Host and a path, and opens that
  file while it is there, its copy otherwise.
* Tell the agent to link a file of the workspace and attach only other files.
* Upload a file again in place of its earlier attachment when it changes.

## Decision Outcome

Chosen option: an attachment keeps its source, because the agent then gives
files one way, and the user always opens the file as it is now while it can
be read.

* An upload by `demi attachment upload` records the Host it ran on and the
  file's full path with the copy. The agent does nothing more.
* Opening an attachment whose Host is the conversation's Host, online, and
  whose file is still there, opens the file in the work panel, as a link to
  it does: the file as it is now, with no note that it changed.
* Otherwise the attachment's copy opens, as today: when the Host is offline,
  when its Cloud is stopped, which opening an attachment never wakes, when the
  conversation moved to another Host, when the file is gone, and for an
  attachment uploaded before this change.
* A picture's preview in a reply stays the copy, so a reply never changes
  after it is sent. Opening it follows the rules above.

```text
reply:  [▣ report.pdf]          ← the attachment, as today
           │ open
           ├─ Host online, file there ─▶ work panel: report.pdf, as it is now
           └─ otherwise ───────────────▶ the copy, as uploaded
```

### Consequences

* Good, because the agent attaches files one way, and the user never opens
  an outdated copy while the file can be read.
* Bad, because one attachment can open different content at different times.
* Modules touched: the attachments' storage in `backend-database`, which
  gains a source; the upload route and `demi attachment upload`; the
  attachment's protocol; and opening an attachment in `web` and `web-ui`.
  Data to migrate: none; attachments without a source open their copy.
