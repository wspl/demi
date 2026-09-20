---
'@demicodes/backend': minor
'@demicodes/web-ui': minor
---

A conversation's title can be generated again on request. `POST /api/conversations/:id/title` asks the conversation's model for a title from every message the user has sent, each cut to 400 characters and the whole to 4,000, with no attachments. The summary carries `titleCurrent` (no message is newer than the last generated title) and `titleGenerating`. `ChatSession` takes `retitle` (`available`, `running`) and emits `retitle`: the Rename button opens a menu of Detect title and Rename, is busy while the title is written, and Detect title is disabled until the user's next message. A double-click on the title renames it in place. A generated title now replaces the title its request started from, so a rename in flight still wins and a title the user typed can be replaced by asking.
