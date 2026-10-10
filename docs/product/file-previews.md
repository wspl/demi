# File previews

The work panel's File view shows one file of the conversation's Host, and its
Change view shows one changed file; they are the pinned `file` and `change`
kinds of the `file-browser` and `changes` plugins
([Work panel](web-application.md#work-panel)). Code and other text open in the code
editor. This document owns what the two views show for everything else: which
kinds of file they preview, how a view picks one, how the bytes reach the
page, when a transfer ends, and what file content may do in the page. It also
owns how a message shows the files it names, and how a tool call shows the
images and videos its result carries. The routes belong to
[Web API](web-api.md#file-text-and-working-tree-changes), the runner's byte
transport to [Runner](../execution/runner.md#file-contents), and Host access
to [Sessions and targets](../execution/sessions-and-targets.md#host-operations).
The gallery shows what each preview looks like.

## What the user sees

For example, the agent records a demo video, edits the README and redraws the
logo. In the File view:

- `README.md` renders as a document, the way GitHub shows a repository file.
  Source switches to its text.
- `assets/logo.svg` shows the picture. Source switches to its XML.
- `docs/demo.mp4` plays in the player built into the user's browser. Seeking to
  minute five fetches only the bytes from there on.
- `dist/app.bin` shows a card with its type, size and modification time, and
  Download.

In the Change view, the logo shows its text diff, and Preview puts the
committed picture beside the current one. The new video shows only its
current side.

| Kind | Extensions | File view | Change view |
| --- | --- | --- | --- |
| Image | `png`, `jpg`, `jpeg`, `gif`, `webp`, `avif`, `bmp`, `ico`, `svg` | The picture, scaled down to fit and never enlarged; a click toggles actual size. Transparency shows over a checkerboard, with the pixel size and file size beside it. | Committed and current side by side, each with its pixel size and file size |
| Video | `mp4`, `m4v`, `webm`, `mov` | The player in the user's browser | Side by side |
| Audio | `mp3`, `wav`, `ogg`, `oga`, `opus`, `m4a`, `aac`, `flac` | The player in the user's browser | Side by side |
| PDF | `pdf` | The PDF viewer in the user's browser | Side by side |
| Markdown | `md`, `markdown` | Rendered, see [Markdown](#markdown) | Text diff |
| Text | Any other file the text route reads | Code editor | Text diff |
| Other | Anything else | A card: type, size, modification time, Download | A card per side |

Markdown and SVG are text as well. The File view offers Preview and Source and
starts on Preview; the Change view offers Diff and Preview and starts on Diff.
The choice holds for the next files until the user changes it, for the page's
lifetime. Every file in the File view can be downloaded, whatever its kind.

These show the card: HEIC, TIFF and JPEG XL images, which only Safari
displays; MKV, AVI and WMV video, which web browsers play unreliably; Office
documents. HTML and MDX show as source: a page needs its scripts and relative
files in an origin of its own, which the File view does not give it. Mermaid diagrams, CSV tables, Jupyter notebooks
and image comparison modes such as swipe and onion skin are not designed yet.

## Choosing a view

One table maps a file extension, ignoring case, to a media type, and says
which media types the page shows in place. The `shared-types` contract crate owns the
table and its lookup, and the web app receives both generated into
`@demicodes/protocol`
([Logic the web app and backend share](../architecture/contracts.md#logic-the-web-app-and-backend-share)):
`web-ui` picks the viewer from it, and the backend takes the type it serves
from the same definition. The choice goes by extension, not by content, for
three reasons: the page must choose an element before any byte arrives, since
a `<video>` fetches its own URL; the type the backend serves must match the
viewer the page chose; and the user's browser must not guess a type on its
own (`nosniff`).

The table also names every type a model reads natively
([Media the model views](../agent/runtime.md#media-the-model-views)), and shows
each in place: a tool's images and videos, and a message's, are served from
their blobs by the same table
([Media a tool returned](#media-a-tool-returned)), and one it did not show in
place would download, so a player would play it only where the user's browser
guessed its format. So `m4v` is `video/x-m4v`, the type a model receives M4V
bytes as.

A file whose content does not match its extension fails in its viewer and
shows the card. So does media that the user's browser cannot decode, such as
HEVC video when it lacks that codec; the card then says the user's browser
cannot play it.
A file the table does not name is read as text, and a `not_text` or
`file_too_large` answer shows the card.

## Markdown

Markdown renders as GitHub renders a repository file: GitHub Flavored
Markdown, with tables, task lists, strikethrough and autolinks, highlighted
code blocks and KaTeX math. A single line break inside a paragraph joins the
lines, unlike in messages. Task list checkboxes are read-only. Leading YAML
front matter shows as a YAML code block. A file over 2 MiB shows only its
source, since rendering that much stalls the page.

Math follows Pandoc's rule for dollar signs, the same in a file and in the
agent's replies, which render through the same Markdown renderer. `$$…$$` is
display math. `$…$` is inline math only when the opening `$` has a
character other than a space right after it, and the closing `$` has one
other than a space right before it and no digit right after it. So prices
stay text: in `Notion is $10 vs $12`, the `$` before `12` cannot close,
since a digit follows it, and both dollar signs show as typed. Inline math
may touch the text around it, as Chinese writing does: `面积为$x^2$平方米`
renders `x^2` as math.

HTML inside the Markdown renders after it is sanitized
([Keeping file content inert](#keeping-file-content-inert)), so README layouts
such as a centered logo or a `<details>` block look as their authors meant.
Heading ids and `<a name>` anchors carry a `user-content-` prefix, as on
GitHub, so a document's names never collide with the page's; `#` links follow
the prefix. An image is scaled down to the document's width with its
proportions kept; a tall image makes the document longer, as on GitHub.

Links and images resolve against the file:

| Target | A link | An image |
| --- | --- | --- |
| Relative path | Resolved against the Markdown file's directory; the file opens in the File view, and Back returns | Resolved the same way and loaded from the Host |
| Path starting with `/` | Resolved against the workspace root, as GitHub resolves it against the repository root | Same |
| `#fragment` | Scrolls to the heading | Not applicable |
| `http` or `https` URL | Opens in a new tab of the user's browser | Loaded from that URL |
| Anything else | Shown as text | Dropped |

In the Change view's Preview, each side renders from its own text, and
relative images load as the Host has them now.

## Files named in messages

A message, the agent's or the user's, can name files on the conversation's
Host. For example, the agent's reply contains an image and a link:

```markdown
![Weekly load](out/chart.png)
[plot.py](scripts/plot.py)
```

The chart loads from the Host the way a document's image does, and a click on
`plot.py` opens it in the File view.

| Target | A link | An image |
| --- | --- | --- |
| Relative path | Resolved against the conversation's working directory, where the agent's commands run; the file opens in the File view | Resolved the same way and loaded from the Host |
| Absolute path, or a `file://` URL | The file at that path, opened in the File view | Loaded from the Host |
| `http` or `https` URL | Opens in a new tab of the user's browser | Loaded from that URL |
| `attachment:a3`, an attachment of the conversation ([Attachment commands](../execution/commands.md#attachment-commands)) | Opens it: an image or a video large, in the viewer, any other file as a download | An image shows; a video shows its first frame with a play mark, at an image's bounds, and a click plays it in the viewer |
| `data:` URL | Shown as text | Shown as it is |
| Anything else | Shown as text | Its alt text |

A Host path names the file as it is now: the image changes when the file
does, and is gone with it, with the Host out of reach, with an archive or with
a target change. A file the agent gives the user is therefore an attachment,
a copy that stays; a Host path suits a file of the workspace the user should
see as it is, such as a chart the repository keeps. An attachment number the
conversation does not have shows its alt text with *No attachment a9 in this
conversation*. The product's instructions tell the model that its messages
render as Markdown, which of the two to use, and that a path in code or plain
text stays text.

An image is scaled down, its proportions kept, to at most the message's width
and 60 percent of the conversation's visible height, the part the composer
does not cover; a smaller image keeps its size. For example, where 800
pixels of the conversation are visible, a 1000 × 8000 screenshot shows 480
pixels tall and 60 wide, instead of 5600 pixels tall at a 700-pixel message
width. A click shows the image whole: a Host image in the File
view, a web image in a new tab of the user's browser. An image inside a link
follows the link.

Images that follow each other show side by side. For example, the agent
replies with four screenshots, one per line, and nothing else between them:
they stand in a row, left to right in their order, as many as the message's
width holds, and the rest wrap onto the next row. A run is two or more
images or videos with nothing but white space between them, in one
paragraph or in paragraphs that follow each other; any text, list or heading
ends it. In a run, each image and each video is a thumbnail, as a tool's are
([Media a tool returned](#media-a-tool-returned)); they stand 8 pixels apart
in both directions. A lone image, and a lone video's first frame, keep the
rule above.

A link to a file opens the `file` intent, and so does a click on a Host image;
while no plugin the user has on opens it, a file link is shown as text and a
Host image only shows. A `:line` suffix on a path is dropped, since the File view opens a whole file.
Only a Markdown link or image names a file: a path in code or plain text stays
text, because the page cannot tell which paths exist without asking the Host
for each. Paths name files on the conversation's Host; a file on another Host
the conversation reaches does not show. Loading an image is a Host operation,
so it wakes a stopped Cloud like any other
([Sessions and targets](../execution/sessions-and-targets.md#host-operations)).

## Media a tool returned

For example, the agent runs `demi browser screenshot t1 | demi file view`.
The call's result attaches the PNG as an image the model reads
([Media the model views](../agent/runtime.md#media-the-model-views)). The page
shows the same picture under the call's row, whether the call is folded or
open, and a click opens it large:

```text
▸ Take a screenshot of the login page    the call's row; its fold holds the
                                         script and [image 1: ...]
  ┌──────────────────────┐
  │    the screenshot    │               what the model read; a click
  │                      │               opens it large
  └──────────────────────┘
```

- **Which calls.** A call shows the images and videos its own result carries,
  so a picture shows where the model saw it. A `shell` call whose command
  exits within the call's window carries the media the job viewed, several
  of them when it viewed several. When the command exits after the
  call returned, the report or the `demi shell status` that tells the end
  carries them instead. The generic tool card shows its result's media the
  same way.
- **Where.** Under the call's row, above the files the call changed, side
  by side in the order of the result and wrapping onto the next row when the
  transcript's width runs out, as a run of images in a message does
  ([Files named in messages](#files-named-in-messages)). Every medium takes its box, a thumbnail's or a player's, before its
  bytes arrive, so the transcript does not move when they load. The fold
  still shows the command's own output, whose lines
  `[image 1: image/png, 1280 × 720 px, 412000 bytes]` stands for the
  original file the job viewed, and `<binary stdout: 412000 bytes>` for bytes
  the model did not see.
- **An image** is a thumbnail: 80 pixels tall, as wide as its proportions
  make it within 64 and 200 pixels, never enlarged. An image whose
  proportions fall outside that range is cropped to it: a tall one, such as
  a whole page's screenshot, keeps its top, and a wide one its middle. For
  example, a 1280 × 720 screenshot is 142 × 80, and a 360 × 2400 page shows
  the top 64 × 80 of it scaled down. Never enlarged wins over the range: the
  box is no larger than the image either way, so a 50 × 50 icon stays
  50 × 50 and a 300 × 40 strip shows its middle 200 × 40. The medium's
  reference, and an attachment's record, carry an image's pixel size, so its
  box is known before its bytes arrive. It is small because it is a step of the
  work, not something the agent chose to show. A click opens it large over the dimmed page, as the File view shows an image:
  scaled down to fit and never enlarged, a click toggles actual size,
  transparency shows over a checkerboard, and its pixel size shows beneath
  it. Escape, the close control or a click on the dimmed page closes it; it
  stays open while the transcript changes beneath it.
- **A video** never plays in place. Its thumbnail is its first frame, sized
  as an image's, with a play mark over its middle. A click opens it in the
  viewer over the dimmed page, scaled down to fit and never enlarged, where it
  plays at once in the player built into the user's browser, whose controls
  and full-screen control it keeps; closing the viewer stops it. A video's
  reference carries its pixel size where the backend reads it from the
  container's header (MP4 and QuickTime, WebM), and a thumbnail of unknown
  size is 16:9 until its first frame arrives.
- **On a phone**, the preview fits the conversation's width. An image opened
  large fills the screen: a tap toggles actual size, at actual size a drag
  moves the picture, a pinch zooms as anywhere on the page, and the close
  control closes it. A video opened from its thumbnail plays in the player of
  the user's browser, which an iPhone shows full screen.
- **A medium that is gone.** A result can hold, in a medium's place, a part
  that says its bytes could not be stored ([Media](../agent/runtime.md#media)).
  The page shows one line where the medium was, with the store's reason, such
  as *Video not stored: the object store refused the write*. The model reads
  the same fact in a text of its own. A medium that was stored stays.
- **A medium that cannot be shown.** When the page cannot show a medium,
  because its blob is missing, the request failed or the user's browser cannot
  decode it, *Could not show this image.* (or *video*) shows in its place, at
  the same height, and a reload of the page tries again. A blob that a block
  references is never deleted ([Media](../agent/runtime.md#media)), so a
  missing one is a fault; a request that replays the medium then carries
  `[missing image blob <ref>]` in its place.
- **The model's picture.** The page loads the blob the result references,
  the same bytes the model receives, so it shows exactly the picture the
  model saw: the image as it was fitted when it entered the transcript
  ([Images in the transcript](../agent/runtime.md#images-in-the-transcript)),
  and a blob is named by the hash of its bytes, and nothing changes them. The
  original bytes stay in the command's whole output, which the model reads
  with `demi shell output`
  ([The whole output](../agent/runtime.md#the-whole-output)).

The bytes come from the blob route
([Media by reference](../backend/backend.md#media-by-reference)). The user's
browser keeps its answers, so opening an image large loads nothing again. A
player reads a video by byte range
([Uploads and media](web-api.md#uploads-and-media)): Safari plays a video only
from a server that answers ranges, and every player seeks with them.

## Changes

A text change shows as a unified diff of the whole file, as GitHub's and VS
Code's do: each changed stretch with three unchanged lines around it, and a
longer unchanged stretch between two changes folded into one line that says
how many lines it holds, which a click unfolds in place. For example, an
edit of line 10 in a 400-line file shows lines 7 to 13, then one line for
the 387 below; the stretch at the top folds only once it is longer than
three lines. The diff is exact whatever the file's size, within 500 ms of
work: a 30 KB file with twelve edits shows twelve changes, never the whole
file removed and added again. When the file changes, the folds follow the
new changes, so a change never hides inside a fold, and a stretch the user
unfolded stays unfolded. The view keeps its place through the update
([What the service keeps](../architecture/plugin-pages.md#what-the-service-keeps)).

The Change tab with no file picked shows the first file it lists, and keeps
showing it when another changed file later sorts above it.

A working directory outside a git repository has no changes to show: the
Change view says so, and the page never lists changes for it, however often
its files change.

In Uncommitted mode, a file with a preview shows its committed version beside
its working-tree version. An added file has only the working-tree side, a
deleted file only the committed side, and a renamed file takes its committed
side from the old path. The two columns stack when the panel is narrow. A
file without a preview shows a card per side, each with Download. A committed
version over 8 MiB, which the committed side's route cannot serve, shows a card
in its column saying it is too large to show, without Download; the
working-tree side is unaffected.

Conversation mode has no media previews: a call's retained edits are text
only, and a binary edit keeps no contents
([Edit tracking](../execution/edit-tracking.md#rationale)). Open file shows the
file as it is now.

## Getting the bytes

Text, including Markdown and SVG source, comes from the text route: at most
8 MiB of UTF-8. A larger text file shows its first 8 MiB, read from the raw route with a
byte range and cut before a character the range splits, with a line above
saying how much of how much it shows and Download, as VS Code opens a large
file rather than refusing it. Everything else comes from the raw route as a stream:

```text
Web app                        Backend                           Runner
<video> GET fs/raw             Host access: wake, file gate
  Range: bytes=150M-   ──────▶ stat: size, modification time
                               read [150M, end) into a pipe ───▶ open, seek, read
                     ◀──────── relay ◀────────────────────────── stream into the pipe
```

- Nothing holds a whole file. Each hop forwards what the next one accepts:
  when the user's browser stops reading, as a player with a full buffer does,
  the backend stops pulling, the runner's upload waits and the file read pauses.
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
element does not reliably stop the user's browser from fetching; a text read
aborts its request. The end travels to the file: the request's end fails the
pipe, the runner stops reading and closes the file, and the backend releases its
Host access.

A transfer is a Host operation, so while it lasts it keeps a Cloud awake and
holds the conversation's file gate. A paused player can keep its connection
open without reading, so a transfer that the user's browser accepts nothing from
for 60 seconds lets go of the Host, and an archive or a target switch ends open
transfers instead of waiting for them
([Host operations](../execution/sessions-and-targets.md#host-operations)).
Either way the page sees the response cut short, never complete, and a
player asks for the range again when it needs more.

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
  PDF viewer of the user's browser. The page's PDF frame never carries the
  `sandbox` attribute, which the PDF viewer of every web browser refuses.
- The page shows SVG only through `<img>`, which never runs its script.
- Rendered Markdown passes DOMPurify with an explicit allowlist modeled on
  GitHub's: headings, paragraphs, lists, tables with `align`, code, links,
  images, `details` and `summary`, `picture` and `source`, `kbd`, `sub` and
  `sup`. Script, event handlers, `style`, forms, frames and embedded objects
  do not pass, and an input passes only as a task list's disabled checkbox.
  The same pass rewrites link and image targets as [Markdown](#markdown)
  describes. Math renders after sanitizing, from its TeX text, because
  KaTeX's output depends on the inline styles sanitizing removes.
- Blobs, a message's uploads and a tool's media alike, follow the same table
  and headers ([Media by reference](../backend/backend.md#media-by-reference)).

## Responsibilities

| Where | Responsibility |
| --- | --- |
| The runner | Moves file contents through pipes ([Runner](../execution/runner.md#file-contents)). |
| The Host contract and the backend's remote Host | Streamed reads, whole or by range, and writes, over pipes. |
| The backend | The raw routes, their headers and ranges, and ending transfers. |
| The `shared-types` contract crate | The file-type table and its lookup, generated for the web app into `@demicodes/protocol`. |
| `web-ui` | The previews as primitives: choosing and showing them, Markdown rendering and sanitizing, the side-by-side comparison, releasing transfers; resolving the files a message names and opening them through the `file` intent ([Intents](../architecture/plugin-pages.md#intents)), showing the media a tool returned and opening an image large. |
| `plugin-file-browser`, `plugin-changes` | The File view and the Change view, composed from the previews over the conversation files service. |
| `web`, `web-gallery` | Raw and blob URLs from the product's routes and the working directory messages resolve against; a fixture file for every kind, and fixture blobs for a tool's media. |

[Crates and packages](../architecture/crates-and-packages.md) names the crate
behind each role.

## Rationale

The previews use what the user's browser already does well: its image
decoders, media players and PDF viewers. Bundling a PDF renderer or a media
stack would add weight and still differ from what the user's browser can play;
the card and Download cover the rest.
