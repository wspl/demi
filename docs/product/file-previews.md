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
| Image | `png`, `jpg`, `jpeg`, `gif`, `webp`, `avif`, `bmp`, `ico`, `svg` | The picture, fitted, zoomed and moved as in the [media viewer](#the-media-viewer), by the same component. Transparency shows over a checkerboard, with the pixel size, the zoom and the file size beside it. | Committed and current side by side, each zoomed and moved as in the File view, with its pixel size, zoom and file size |
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

### The File view's bars

The File view has two bars, as a browser has an address bar over a page and
GitHub a file's own bar over its contents:

```text
 ← →  [ / › Users › zan › Projects › demi › docs › guide.md ]  [tree]   the path bar
 ┌──────────────────────────────────────────────────┬──────────┐
 │ Markdown · 47 lines · 1.1 KB   [Preview|Source] ⧉ ⤓ │ demi     │   the file's bar
 │                                                  │  docs    │
 │   the document                                   │   guide… │
```

- **The path bar shows the whole path**, always: its crumbs start at the
  Host's root, the home and the root wearing their glyphs, and its text field
  holds the absolute path. The tree beside the file is the workspace's, and
  only the tree: the path bar says where the file is on the Host, wherever
  that is. A path typed relative to nothing still starts from the
  workspace, and `~` names the home. A bar too narrow for every name turns the
  crumbs farthest from the file into their glyphs first, as it does now.
- **The file's bar** stands over what the file shows, the width of the file's
  side, not over the tree, 36 pixels tall. On its left, what the file is: its
  kind's full name, the language for text as the editor knows it,
  *TypeScript*, *Markdown*, *Plain Text* for one it does not, then the facts
  that kind has, so a picture's facts move up here from under it:

  | Kind | The bar says |
  | --- | --- |
  | Text and Markdown | *TypeScript · 20 lines · 476 B* |
  | Image | *PNG Image · 1280 × 720 · 150% · 412 KB*, the zoom following the picture |
  | Video | *MP4 Video · 1280 × 720 · 4.2 MB* |
  | Audio, PDF | *M4A Audio · 3.1 MB*, *PDF Document · 812 KB* |
  | Other | Its kind and size, as its card says |

  On its right, what the user can do with the file: Preview and Source for
  Markdown and SVG, Copy for any file shown as text, which puts its whole
  text on the clipboard, Markdown's source and not its rendering, and
  Download for every file. A file shown as only its first 8 MiB has no Copy,
  since the page does not hold the rest; Download gives all of it. Nothing of
  the file stands in the path bar.

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

## Code

Code shows in one set of colors wherever it shows: the File view's source,
the Change view's diff, a code block in a message or a rendered document,
the script of a `shell` call's row and of its terminal. One highlighter
reads it, the code editor's, with one theme, One Dark Pro or One Light by
the page's mode, so the same line of Rust is the same colors in a reply and
in the file it came from, as VS Code shows a Markdown preview's code in its
editor's colors. A language the highlighter does not know shows as plain
text in the theme's text color.

The File view's source and the Change view's diff are read-only, but they
are text the user reads with the keyboard as well as the mouse, as a
read-only file is in VS Code or Xcode: a click puts a blinking caret there,
in the theme's caret color in either mode, and the arrow keys move it,
Shift extends the selection from it, and ⌘F finds from it. Without the
focus the view shows no caret.

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
`plot.py` opens it in the File view. A link to a Host file shows the file's
icon before its name, the icon the file tree shows for it, so a file reads as
a file and not as a page on the web. A link that starts with an icon, a
file's icon or an agent's mark ([Linking an agent](../agent/subagents.md#linking-an-agent)),
is a small object in the sentence, as a mention is in Slack or Notion, so it
keeps 0.2 em of space on each side, inside its hover fill where it has one
and outside its underline: in "已经告诉[负责的 Agent](agent:57)：", the character before the
mark and the colon after the name no longer touch it. A link without an
icon is words and is spaced as words.

| Target | A link | An image |
| --- | --- | --- |
| Relative path | Resolved against the conversation's working directory, where the agent's commands run; the file opens in the File view, and a `#L42` or `#L42-L50` after the path, or `:42`, opens it at line 42, scrolled there and marked for a moment as a search result is, in the file's source even where it has a preview, as GitHub's line links show the plain text | Resolved the same way and loaded from the Host |
| Absolute path, or a `file://` URL | The file at that path, opened in the File view | Loaded from the Host |
| `http` or `https` URL | Opens in a new tab of the user's browser | Loaded from that URL |
| `agent:4`, an agent of the conversation ([Linking an agent](../agent/subagents.md#linking-an-agent)) | Opens the Agents window on that agent's tab | Its alt text |
| `attachment:a3`, an attachment of the conversation ([Attachment commands](../execution/commands.md#attachment-commands)) | Opens it: an image or a video in the [media viewer](#the-media-viewer), any other file as a download | An image shows; a video shows its first frame with a play mark, at an image's bounds, and a click plays it in the viewer |
| `data:` URL | Shown as text | Shown as it is |
| Anything else | Shown as text | Its alt text |

A Host path names the file as it is now: the image changes when the file
does, and is gone with it, with the Host out of reach, with an archive or with
a target change. A file the agent gives the user is therefore an attachment,
a copy that stays; a Host path suits a file of the workspace the user should
see as it is, such as a chart the repository keeps. An attachment number the
conversation does not have shows its alt text with *No attachment a9 in this
conversation*. The product's instructions tell the model that its messages
render as Markdown, which of the two to use, and that a file it names is a
link ([System prompt](../agent/system-prompt.md)).

A path the agent writes as code rather than as a link opens too, when it
names a file that is there. For example, the reply says
`` `docs/execution/runner.md` `` or `` `src/app.ts:42` ``: the page asks the
conversation's Host, once for the reply's code spans together, which of them
are files, and those show as links, `src/app.ts:42` opening at line 42; the
others stay text. Only inline code that reads as a path is asked about, one
with a `/` or a name with an extension, and an optional `:line`: a path is
relative to the working directory or absolute, and a bare name such as
`runner.md`, which may name several files, is never guessed. Plain text is
never turned into a link. The answer is kept with the reply; a stopped Cloud
is not woken for it, and its code stays text until the Host runs.

An image is a picture in the message, not the message: it is scaled down,
its proportions kept, to at most 480 pixels wide and 320 tall, and the
message's width where that is less; a smaller image keeps its size. For
example, a 2560 × 1720 screenshot shows 476 × 320, a 1280 × 720 one
480 × 270, and a 1000 × 8000 page 40 × 320, as chat apps show a picture in
a message and leave the detail to the click. An image always stands on a
line of its own, with 12 pixels between it and the text above and below it,
even where the Markdown puts it in the same paragraph as the text, such as
on the line after a list item's words. A click shows the image whole: a Host image in the File
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
in both directions, and the run stands apart from the text as a lone image
does. Text before the first image in the same paragraph does not stop the
run: a list item whose words are followed by two images, one per line, shows
its words, then the two side by side. A lone image, and a lone video's first frame, keep the
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
open, and a click opens it in the [media viewer](#the-media-viewer):

```text
▸ Take a screenshot of the login page    the call's row; its fold holds the
                                         script and [image 1: ...]
  ┌──────────────────────┐
  │    the screenshot    │               what the model read; a click
  │                      │               opens it large
  └──────────────────────┘
```

- **Which calls.** A call shows the images and videos its own result carries,
  and a command's notice those its report carries, so a picture shows
  where the model saw it; a folded run shows those of its calls and notices. A `shell` call whose command
  exits within the call's window carries the media the job viewed, several
  of them when it viewed several. When the command exits after the
  call returned, the end report
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
  box is known before its bytes arrive. A row of thumbnails is as tall as
  its tallest box, not 80 pixels whatever they are: four 171 × 40 frames
  stand in a row 40 pixels tall, with no empty band under them before the
  next row. Only a medium whose size is unknown until it loads, such as a
  video before its first frame, keeps the 80-pixel box, so the transcript
  never shrinks when it arrives. It is small because it is a step of the
  work, not something the agent chose to show. A click opens it in the
  [media viewer](#the-media-viewer).
- **A video** never plays in place. Its thumbnail is its first frame, sized
  as an image's, with a play mark over its middle. A click opens it in the
  [media viewer](#the-media-viewer), scaled down to fit and never enlarged, where it
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

### The media viewer

The user clicks a screenshot's thumbnail in the transcript. A small window
opens over the page, as Quick Look and Preview open a picture on a Mac: the
screenshot fits in it, its title bar names it, and the page behind stays as
it was, not dimmed, so the user can go on reading and typing beside it.

```text
 ┌─ login-page.png ── 1280 × 720 ─────────[×]┐  title bar: drag to move
 │                                            │
 │              the screenshot                │  click: actual size
 │                                            │
 └────────────────────────────────────────────┘◢ edges and corners resize
```

- **A window, not a layer.** It does not block the page: the transcript
  scrolls, the composer takes typing and other controls work while it is
  open, and it stays open while the transcript changes beneath it.
- **Move and resize.** Dragging its title bar moves it, and dragging any edge
  or corner resizes it, as a window on the desktop does; it stays at least
  240 × 180 and no larger than the page, and its title bar never leaves the
  page. The first time it opens, it is as large as the picture at its actual
  size, at most 60% of the page's width and 70% of its height, centred over
  the conversation; after that it opens where the user last left it, at the
  size they gave it, for as long as the page is open.
- **One viewer.** Opening another picture or video shows it in the same
  window, in place of the one it showed.
- **What it shows.** An image opens fitted to the window, scaled down and
  never enlarged, over a checkerboard where it is transparent; its pixel
  size and the zoom show in the title bar, *1280 × 720 · 150%*.
- **Zoom and move.** As in an image viewer, the wheel, and a pinch on a
  trackpad, zoom the image about the pointer, so the spot under it stays
  under it: from the fitted size, or the actual size when that is smaller,
  up to eight times the actual size. While the image is larger than the
  window, dragging it moves it, with the grab hand as the pointer, and it
  never leaves an empty band on a side where it still has pixels to show. A
  click without a drag toggles between the fitted size and the actual size
  about the clicked spot. Resizing the window keeps a fitted image fitted,
  and a zoomed one at its zoom; another picture opens fitted. A video plays at once in the player built into the user's
  browser, whose controls and full-screen control it keeps.
- **Close.** The close control in the title bar closes it; so does a press
  outside every window while it is the newest one, by the rule the
  session's windows share ([Recovering an unfinished turn](product.md#recovering-an-unfinished-turn)); and so does
  Escape, unless the focus is in a text field or in another layer that takes
  Escape, such as a menu or a dialog. Opening moves the focus into the
  window, so Escape right after a click closes it; closing returns the focus
  to the thumbnail that opened it. Closing stops a video.

The window is a primitive of `web-ui` that any floating window of the
product uses, not the viewer's own code.

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
8 MiB of text, a byte that is no UTF-8 shown as U+FFFD, and a file that is
binary by the one rule ([What `demi file view` shows](../agent/runtime.md#what-demi-file-view-shows))
shows its card instead. A larger text file shows its first 8 MiB, read from the raw route with a
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
