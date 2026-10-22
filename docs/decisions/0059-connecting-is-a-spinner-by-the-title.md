---
status: accepted
---

# Connecting is a spinner in place of the title's Rename button

Supersedes, in part: the tail row's Connecting (`activity-slot.ts`), where a
conversation's opening and a lost socket add a row at the transcript's end;
and the connection banner (`ConnectionBanner.vue`), the strip across the top
of the app while the backend cannot be reached.

## Summary

While a conversation's socket is not open, the Rename button beside its
title becomes a spinner of the same size; no row is added to the transcript
and no words are shown. Requesting stays the transcript's tail row. The
banner across the top of the app is removed.

## Motivation

A user opens a conversation. The transcript shows the page's copy with a
Connecting row at its end; the socket opens, the row goes, and every row
moves by its height. It happens at every opening and every reconnect.

## Guide-level explanation

```text
  open:        Fix the cart total  ✎
  connecting:  Fix the cart total  ◌
```

* While the conversation's socket is not open for two seconds or more, the
  Rename button shows a spinner in its place, and is not a button; when the
  socket opens, the button comes back. A shorter wait shows nothing.
* The transcript never says Connecting; nothing in it moves for the socket.
* On a phone, where the title opens a sheet, the spinner stands where the
  title's chevron is.
* No banner crosses the top of the app when the backend cannot be reached,
  the browser being offline included; the spinner by the title is the one
  sign, as the conversation's socket cannot open then either.

## Reference-level explanation

* `ActivityKind` loses `connecting`; `activitySlotKind` answers `requesting`
  or null, by the actions sent and the server's wait alone.
* `ChatSession` reads the runtime's socket state and shows the spinner in
  the Rename button's box, the same width and height, so the title does not
  move; its label for assistive technology is the existing Connecting…
  string.
* `ConnectionBanner`, its height in the app bar's layout and its gallery
  specimens are removed; the page's state keeps why it cannot reach the
  backend only where something else reads it.

## Upgrade and version skew

None: the change is in the page alone.

## Performance

None.

## Drawbacks

* A spinner by the title is easy to miss.

## Rationale and alternatives

* Keep the row and reserve its height: an empty row at the end of every
  transcript.
* Words by the title: more to read for a state that passes on its own.

## Prior art

* Telegram and WeChat tell a connection being made in the chat's title bar;
  Slack in a banner. None adds it to the messages.

## Unresolved questions

None.
