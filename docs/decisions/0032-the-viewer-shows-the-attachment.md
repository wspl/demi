---
status: accepted
---

# The viewer shows the attachment

Supersedes, in part: 0018 (the file cache owns a file's version), where an
attachment opens as its file's latest version.

## Context and Problem Statement

A reply's attachment shows in the transcript as a thumbnail of its own copy,
which the backend keeps and never changes, so the browser caches it for good
and it shows at once. Opening it in the media viewer did something else: the
viewer showed the Host file the attachment came from, as it is now, and fell
back to the copy only when that file was gone.

That one choice cost a request to the backend and a hash of the file on the
Host at every selection, before anything showed: 332 ms to go back to a
picture already shown, with 300 ms of network delay. It also needed a rule to
tell when the copy may stand for the file (0018). Nobody asked for the
viewer to show anything but the attachment.

## Considered Options

* Keep showing the file's latest version, and cache the answers and the bytes.
* Show the attachment's own copy, as the thumbnail does.

## Decision Outcome

Chosen option: the attachment's own copy, because the viewer is where an
attachment is looked at, and the attachment is the copy the reply delivered.

* Opening an attachment, from its thumbnail or with Previous and Next, shows
  its own copy, the same bytes the thumbnail shows, with no request about the
  file it came from. A file that changed or went since does not change it.
* The Host file is what the Files panel shows; the viewer does not link the
  two.
* What only served opening the latest version goes, with its tests: the
  backend's answer about an attachment's file, the hash on the Host, the
  viewer's link between the copy and a version, and its waiting and Loading
  for that answer. The change removes code; it adds none beyond pointing the
  viewer at the copy.

### Performance

Measured before the change, with a 2 MB photo, median of three:

| Going back to | Local | 300 ms delay |
|---|---|---|
| A reply's attachment | 43 ms | 332 ms |
| A picture from a tool's result | 33 ms | 31 ms |

Expected, not measured:

* An attachment opened the first time loads its copy, as its thumbnail did,
  usually from the browser's cache; no Host is read.
* Going back to an attachment costs what going back to a tool's picture costs
  today, about 33 ms whatever the network, since both show bytes the browser
  holds.
* Memory and storage: unchanged.

### Consequences

* Good, because opening an attachment no longer depends on its Host, which
  may be asleep or gone, and costs no request after the first.
* Good, because a whole path and its rule go, not only their cost.
* Bad, because an attachment shows what the reply delivered even when the
  file changed since; the Files panel shows the file as it is.
