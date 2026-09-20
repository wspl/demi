---
'@demicodes/web-ui': minor
---

The session header has a Rename button beside the title: the title becomes an input where it stands, and `ChatSession` emits `rename` with the new title. `ui/TitleInput` is the one in-place title editor, used by the header and the sidebar row: Enter or leaving it submits the trimmed title, Escape, an empty or an unchanged title cancels.
