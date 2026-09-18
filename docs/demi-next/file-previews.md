# File previews

The work panel's File view shows one file of the conversation's Host, and its
Change view shows one changed file. Code and other text open in the code
editor. This document owns what the two views show for everything else: which
kinds of file they preview, how a view picks one, how the bytes reach the
page, when a transfer ends, and what file content may do in the page. The
routes belong to [Web API](web-api.md#file-text-and-working-tree-changes), the
runner's byte transport to [Runner](runner.md#file-contents), and Host access
to [Sessions and targets](sessions-and-targets.md#host-operations). The gallery
shows what each preview looks like.

## What the user sees

For example, the agent records a demo video, edits the README and redraws the
logo. In the File view:

- `README.md` renders as a document, the way GitHub shows a repository file.
  Source switches to its text.
- `assets/logo.svg` shows the picture. Source switches to its XML.
- `docs/demo.mp4` plays in the browser's own player. Seeking to minute five
  fetches only the bytes from there on.
- `dist/app.bin` shows a card with its type, size and modification time, and
  Download.

In the Change view, the logo shows its text diff, and Preview puts the
committed picture beside the current one. The new video shows only its
current side.

| Kind | Extensions | File view | Change view |
| --- | --- | --- | --- |
| Image | `png`, `jpg`, `jpeg`, `gif`, `webp`, `avif`, `bmp`, `ico`, `svg` | The picture, scaled down to fit and never enlarged; a click toggles actual size. Transparency shows over a checkerboard, with the pixel size and file size beside it. | Committed and current side by side, each with its pixel size and file size |
| Video | `mp4`, `m4v`, `webm`, `mov` | The browser's player | Side by side |
| Audio | `mp3`, `wav`, `ogg`, `oga`, `opus`, `m4a`, `aac`, `flac` | The browser's player | Side by side |
| PDF | `pdf` | The browser's PDF viewer | Side by side |
| Markdown | `md`, `markdown` | Rendered, see [Markdown](#markdown) | Text diff |
| Text | Any other file the text route reads | Code editor | Text diff |
| Other | Anything else | A card: type, size, modification time, Download | A card per side |

Markdown and SVG are text as well. The File view offers Preview and Source and
starts on Preview; the Change view offers Diff and Preview and starts on Diff.
The choice holds for the next files until the user changes it, for the page's
lifetime. Every file in the File view can be downloaded, whatever its kind.

These show the card: HEIC, TIFF and JPEG XL images, which only Safari
displays; MKV, AVI and WMV video, which browsers do not play reliably; Office
documents. HTML and MDX show as source: a page needs its scripts and relative
files in an origin of its own, which an [expose](expose.md) of a server on the
Host gives it. Mermaid diagrams, CSV tables, Jupyter notebooks and image
comparison modes such as swipe and onion skin are not designed yet.

## Choosing a view

One table maps a file extension, ignoring case, to a media type, and says
which media types the page shows in place. `@demicodes/core` owns it: `web-ui`
picks the viewer from it, and the backend takes the type it serves from it.
The choice goes by extension, not by content, for three reasons: the page must
choose an element before any byte arrives, since a `<video>` fetches its own
URL; the type the backend serves must match the viewer the page chose; and
browsers must not guess a type on their own (`nosniff`).

A file whose content does not match its extension fails in its viewer and
shows the card. So does media the browser cannot decode, such as HEVC video in
a browser without that codec; the card then says the browser cannot play it.
A file the table does not name is read as text, and a `not_text` or
`file_too_large` answer shows the card.

## Markdown

Markdown renders as GitHub renders a repository file: GitHub Flavored
Markdown, with tables, task lists, strikethrough and autolinks, highlighted
code blocks and KaTeX math. A single line break inside a paragraph joins the
lines, unlike in messages. Task list checkboxes are read-only. Leading YAML
front matter shows as a YAML code block. A file over 2 MiB shows only its
source, since rendering that much stalls the page.

HTML inside the Markdown renders after it is sanitized
([Keeping file content inert](#keeping-file-content-inert)), so README layouts
such as a centered logo or a `<details>` block look as their authors meant.
Heading anchors carry a `user-content-` prefix, as on GitHub, so a document's
ids never collide with the page's; `#` links follow the prefix.

Links and images resolve against the file:

| Target | A link | An image |
| --- | --- | --- |
| Relative path | Resolved against the Markdown file's directory; the file opens in the File view, and Back returns | Resolved the same way and loaded from the Host |
| Path starting with `/` | Resolved against the workspace root, as GitHub resolves it against the repository root | Same |
| `#fragment` | Scrolls to the heading | Not applicable |
| `http` or `https` URL | Opens in a new browser tab | Loaded from that URL |
| Anything else | Shown as text | Dropped |

In the Change view's Preview, each side renders from its own text, and
relative images load as the Host has them now.

## Changes

In Uncommitted mode, a file with a preview shows its committed version beside
its working-tree version. An added file has only the working-tree side, a
deleted file only the committed side, and a renamed file takes its committed
side from the old path. The two columns stack when the panel is narrow. A
committed version over 8 MiB shows a card in its column saying it is too large
to show; the working-tree side is unaffected. A file without a preview shows a
card per side, each with Download.

Conversation mode is unchanged. A call's retained edits are text only, and a
binary edit keeps no contents ([Edit tracking](edit-tracking.md#rationale)).
Open file shows the file as it is now.

## Getting the bytes

Text, including Markdown and SVG source, comes from the text route as before:
at most 8 MiB of UTF-8. Everything else comes from the raw route as a stream:

```text
Browser                        Backend                           Runner
<video> GET fs/raw             Host access: wake, file gate
  Range: bytes=150M-   ──────▶ stat: size, modification time
                               read [150M, end) into a pipe ───▶ open, seek, read
                     ◀──────── relay ◀────────────────────────── stream into the pipe
```

- Nothing holds a whole file. Each hop forwards what the next one accepts:
  when the browser stops reading, as a player with a full buffer does, the
  backend stops pulling, the runner's upload waits and the file read pauses.
  So a working-tree file has no size limit.
- A range request reads only its range. That is how a player seeks and how a
  PDF viewer fetches pages on demand.
- The committed version of a change comes from git, which stores it
  compressed and often as the difference from another version. The runner
  must decode it whole into memory before sending it, so it is limited to
  8 MiB, the limit text diffs already have.

A preview names the version of the file it opened, the ETag a `HEAD` of the
raw route reported. When the file changes under a playing video, the next
range request answers 412 and the player stops with an error instead of
splicing old and new bytes; players retry without validators of their own.
The view then offers to open the new version.

## Ending a transfer

The page ends a transfer as soon as its preview stops being shown: the user
picks another file or tab, closes the panel, opens another conversation or
leaves the page. A player or an image drops its source, because removing the
element does not reliably stop the browser's fetch; a text read aborts its
request. The end travels to the file: the request's end fails the pipe, the
runner stops reading and closes the file, and the backend releases its Host
access.

A transfer is a Host operation, so while it lasts it keeps a Cloud awake and
holds the conversation's file gate. A paused player can keep its connection
open without reading, so a transfer the browser accepts nothing from for 60
seconds ends, and an archive or a target switch ends open transfers instead of
waiting for them ([Host operations](sessions-and-targets.md#host-operations)).
A player asks for the range again when it needs more.

## Keeping file content inert

Files on a Host are untrusted input. A cloned repository, a downloaded file or
a file the model wrote on someone else's instructions can contain script, and
script running in the product's origin acts as the user: it can instruct the
agent to run commands on every device the user has. So file content never runs
in the product's origin:

- The raw route serves the table's in-place media types as themselves and
  everything else as a download: `application/octet-stream` with
  `Content-Disposition: attachment`. Every answer carries
  `X-Content-Type-Options: nosniff`.
- An image served in place also carries
  `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox`,
  the policy GitHub and Gitea put on raw files, so an SVG opened directly runs
  in an opaque origin, without script and without fetching anything.
- Audio, video and PDF carry no policy: Chrome refuses to play an audio file
  opened directly under one, and by Gitea's account Safari refuses to show a
  PDF under `sandbox`.
  None of them runs script in the page; a PDF's script stays inside the
  browser's viewer. The page's PDF frame never carries the `sandbox`
  attribute, which every browser's PDF viewer refuses.
- The page shows SVG only through `<img>`, which never runs its script.
- Rendered Markdown passes DOMPurify with an explicit allowlist modeled on
  GitHub's: headings, paragraphs, lists, tables with `align`, code, links,
  images, `details` and `summary`, `picture` and `source`, `kbd`, `sub` and
  `sup`. Script, event handlers, `style`, forms, frames and embedded objects
  do not pass, and an input passes only as a task list's disabled checkbox.
  The same pass rewrites link and image targets as [Markdown](#markdown)
  describes. Math renders after sanitizing, from its TeX text, because
  KaTeX's output depends on the inline styles sanitizing removes.
- Uploaded blobs follow the same table and headers
  ([Media by reference](backend.md#media-by-reference)).

## Responsibilities

| Where | Responsibility |
| --- | --- |
| `crates/runner`, `packages/runner-protocol` | Move file contents through pipes ([Runner](runner.md#file-contents)). |
| `packages/shell`, `packages/host-remote` | Streamed reads, whole or by range, and writes in the Host contract, over pipes. |
| `packages/backend` | The raw routes, their headers and ranges, and ending transfers. |
| `packages/core` | The extension table. |
| `packages/web-ui` | Choosing and showing previews, Markdown rendering and sanitizing, the side-by-side comparison, releasing transfers. |
| `packages/web`, `packages/web-gallery` | Raw URLs from the product's routes; a fixture file for every kind. |

## Rationale

The previews use what browsers already do well: their image decoders, media
players and PDF viewers. Bundling a PDF renderer or a media stack would add
weight and still differ from what the user's browser can play; the card and
Download cover the rest.
