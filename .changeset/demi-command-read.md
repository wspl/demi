---
"@demicodes/coding-agent": minor
---

Rename the `editor` command to `demi` and give it a content-aware `read`.

`createEditorCommand` is now `createDemiCommand`, and the registered command is
`demi` (`demi create` / `demi edit` / `demi patch`) — a single namespace for the
framework's built-in workspace tools rather than an edit-only "editor". The new
`demi read <path>` reads a file: text is returned as text, and images
(png/jpeg/webp/gif) are returned as a viewable image block via `CommandIO.asset`,
so the model can actually see images a read surfaces. The `coding-harness`
option `editorHost` is now `demiHost`, and file-diff metadata is `file_diffs`.
