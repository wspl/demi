---
'@demicodes/web-ui': minor
---

The session header cuts its title short last: the directory button and then the host button become their icons, names in tooltips, while the title lacks width, and name themselves again when it fits. `ui/label-room` (`provideLabelRoom`, `useRoomLabel`) is the one mechanism for a label giving way, and the composer's model chip now uses it instead of a `room` prop. The header's Archive button is removed, with `ChatSession`'s `archive` event.
