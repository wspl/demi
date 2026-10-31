---
status: proposed
---

# A binary file's pill opens the file

## Problem

The agent copies a new `logo.png` into the project. Under the command, a pill
names `logo.png`. A click on it opens the Change view, which has no diff to
show for a binary file and says so: "Demi keeps the text of the changes a
conversation makes, not pictures or other binary files.", with Open File
under it. The user clicks twice to see the picture, and reads a sentence
about what is missing on the way.

## Decision

A file pill whose edit is recorded as binary, which edit tracking keeps no
contents of, opens the file as it is now through the `file` intent, in the
File view, where an image, a video or a PDF shows as itself, as the Change
view's Open File does. Every other pill opens its edit in the Change view
as before, a text file's diff among them.

The pill is the same; only where its click goes changes. A binary file's
pill stays a control while a plugin opens files, even with none on that
opens changes. While no plugin opens a file, the pill opens the edit as
before, and the Change view says why it has no diff. The "+N Files" pill
still lists its files in the Change view, where a binary one says why it has
no diff, since it asks for the list and not one file. Opening the Change
view from its own list still shows that sentence too, since there the user
asked for the change.

The pills under a command and under a folded run ask one function what a
click does (`packages/web-ui/src/agent/edit-selection.ts`):

```ts
/**
 * Opens the file as it is now for a binary edit, through `files.open`; the
 * edit through `select` otherwise; null while neither opens it.
 */
function pillAction(
  select: EditSelectionHandler | undefined,
  requests: TranscriptRequests,
  files: MessageFiles | undefined,
  selection: RequestEditSelection,
): (() => void) | null
```

It reads whether the edit is binary from the transcript's requests
(`selectsBinary`), and resolves the file's path as the Host writes it, a
Windows path included, against the directory the call ran in. A subagent's
file link keeps a directory it names, so the File view opens there.

## Spike

None.

## Prior art

- GitHub's pull request diff shows a binary file it cannot render as
  "Binary file not shown"
  ([an example](https://stackoverflow.com/questions/58241262/how-do-i-comment-on-a-github-pull-request-for-a-binary-file-that-isnt-shown)),
  and renders an image as images instead.
- VS Code opens a binary file in its binary editor, which says the file is
  not displayed because it is binary and offers to open it anyway
  ([`binaryEditor.ts`](https://github.com/microsoft/vscode/blob/e8db8ed8/src/vs/workbench/browser/parts/editor/binaryEditor.ts)).

Both show the binary file's place in the diff and say why it has no text.
Demi departs for its pill: the pill already names one file and asks to see
it, and a diff with nothing to compare would only say so, so it opens the
file itself, as GitHub shows an image.

## Alternatives

- Show the picture's before and after side by side: edit tracking keeps no
  copy of a binary file, so there is no before.
- Leave the pill and make the Change view open the file by itself: the
  Change view opened from its own list would then leave it too, where the
  user asked for the change.

## Open questions

None.
