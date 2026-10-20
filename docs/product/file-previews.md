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
[Web API](web-api-hosts.md#file-text-and-working-tree-changes), the runner's byte
transport to `crates/runner-host/src/files.rs`, and Host access
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
 ← →  [ / › Users › alex › Projects › shop › docs › guide.md ]  [tree]   the path bar
 ┌──────────────────────────────────────────────────┬──────────┐
 │ Markdown · 47 lines · 1.1 KB   [Preview|Source] ⧉ ⤓ │ demi     │   the file's bar
 │                                                  │  docs    │
 │   the document                                   │   guide… │
```

- **The path bar shows the whole path**, always: its crumbs start at the
  Host's root, the home and the root wearing their glyphs, and its text field
  holds the absolute path. The tree beside the file is the directory's
  the view was opened for: the working directory of the agent whose link,
  file change or folder opened it, the conversation's when the user opened
  it; and only the tree: the path bar says where the file is on the Host, wherever
  that is. A path typed relative to nothing still starts from the
  workspace, and `~` names the home. A bar too narrow for every name turns the
  crumbs farthest from the file into their glyphs first; when even the
  glyphs do not fit, the middle folders go behind one ellipsis crumb,
  **Enclosing Folders**, whose menu lists them and opens the one picked, so
  the first crumb and the file stay whole, as Finder's path bar keeps its
  ends; then the file's name is cut short, then shown as its glyph. No crumb
  is ever cut in half.
- **The tree follows the file.** Whenever the File view shows a file, opened
  by a link, a file change, an attachment, the tree itself, the path bar,
  Back or Forward, the tree expands the folders above it, selects its row
  and scrolls it into view, as VS Code's explorer and Xcode's navigator
  follow the file being edited; the folders the user opened or closed
  elsewhere stay as they are. A file outside the tree's directory selects
  no row, and the selection the tree had is cleared. A hidden tree stays
  hidden and follows when it is shown again; only *Show in Files* shows
  it.
- **The file's bar** stands over what the file shows, the width of the file's
  side, not over the tree, 36 pixels tall. On its left, what the file is: its
  kind's full name, the language for text as the editor knows it,
  *TypeScript*, *Markdown*, *Plain Text* for one it does not, then the facts
  that kind has, so a picture's facts move up here from under it:

  | Kind | The bar says |
  | --- | --- |
  | Text and Markdown | *TypeScript · 19 lines · 476 B*, counting lines as GitHub does: the line break that ends the last line starts no line of its own, so a file of `a⏎b⏎` has 2 |
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

### Symbolic links

A symbolic link shows as Visual Studio Code's Explorer shows one: as what it
points at, with a link mark at the end of its row. For example, a project
holds `vendor → ../shared/vendor` and `config.json → ~/.config/app.json`:

```text
▸ 📁 src
▸ 📁 vendor                 ↪
  📄 config.json            ↪
  📄 old.log                ↪      its target is gone
```

- **It is what its target is.** Its icon is its target's, a folder's or the
  file type's. A link to a directory opens as a folder, its entries the
  target's, at any depth: a link back to a folder above it opens again as
  the Explorer's does, since each level is listed only when opened. A link
  to a file opens the target's contents, under the link's own path in the
  path bar.
- **The mark says it is a link.** A small arrow at the end of the row, in
  the secondary text color, where the Explorer puts its decorations; its
  tooltip names the target as written, *Symbolic link to ../shared/vendor*.
- **It sorts as what its target is**: a link to a directory among the
  folders, as the Explorer sorts it. A link to something that is neither a
  file nor a directory, such as a device or a pipe, shows and opens as a
  file does.
- **A link whose target is missing** shows as a file with the mark, its
  tooltip *Symbolic link to /tmp/gone.log, which does not exist*; opening it
  shows that sentence where the file would be. Missing means nothing is
  there. A target that is there but cannot be reached, because a folder on
  the way is closed to the user, the links go round in a loop, or the disk
  fails, is *unreadable* instead, and says why: *Symbolic link to
  /root/notes, which cannot be read: permission denied*. Opening either
  shows its sentence from the listing, whatever a read of the target
  answers; the File view asks for the folder's listing and the file at
  once, so a file opened from a message link costs no extra round trip.
- **What a row says about its target is as of its listing.** The folder's
  watch sees the folder's own entries, not where each link leads, so a
  target created or removed elsewhere does not change the row by itself, as
  the Explorer's rows do not. Expanding or opening a link reads its target
  again, and the row takes what that read found: a link that showed as
  missing and now leads to a folder expands, and one that showed as a
  folder whose target is gone says it does not exist.

The listing says what each link's target is
(`crates/runner-host/src/lib.rs`), so the tree shows all of this
without a request per link. The attachment picker and a device's folder
dialog show links the same way, and act on what a link leads to when it is
chosen, after reading it again, as the tree does: a link to a folder opens
as one, and choosing a link whose target is missing or unreadable attaches
nothing and says its sentence where the picker shows its errors, as Finder
says an alias's original cannot be found. In the folder dialog a link to a
file is dimmed like a file, and opening it on a double-click reads it again,
so one whose target has become a folder opens.

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
(`crates/command-package-file/src/files.rs`), and shows
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

The editor and its highlighter are not part of the page that opens: the
first code that shows, and the first File or Change view, load them, about
600 KB the first screen would not use. KaTeX and the terminal are loaded the
same way, by the first math ([Markdown](#markdown)) and the first terminal
that opens, and a test of the build keeps all of them out of the files the
page loads first. Until the editor arrives, code shows as
plain text in the theme's text color, laid out as it will be, and a code
block's language label keeps its row, empty; the colors and the label then
fill in without moving anything. A File or Change view says *Reading…* until
its editor is there, and *Couldn't load the code editor.* with Retry when it
cannot load; a terminal says *Loading terminal…*, and *Couldn't load the
terminal.* with Retry. Retry loads the page again, as a page of another
build does ([A page of another build](web-application.md#a-page-of-another-build)):
a browser keeps a module that failed to load as failed for the page's life,
and a drafted message and the place the reader was are kept across a
reload.

The File view's source and the Change view's diff are read-only, but they
are text the user reads with the keyboard as well as the mouse, as a
read-only file is in VS Code or Xcode: a click puts a blinking caret there,
in the theme's caret color in either mode, and the arrow keys move it,
Shift extends the selection from it, and ⌘F finds from it. Without the
focus the view shows no caret.

## Markdown

Markdown renders as GitHub renders a repository file: GitHub Flavored
Markdown, with tables, task lists, strikethrough and autolinks, alerts
(`> [!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, `[!CAUTION]`, titled
in the reader's language), highlighted code blocks and KaTeX math. An image
whose file is still being read, or is missing, shows the quiet placeholder
box with its alt text, never the browser's broken-image icon. A single line break inside a paragraph joins the
lines, unlike in messages. Task list checkboxes are read-only. Leading YAML
front matter shows as a YAML code block. A file over 2 MiB shows only its
source, since rendering that much stalls the page.

Until KaTeX arrives, math shows its TeX source, without the dollar signs,
in the code font and the muted color, where the math will stand: inline in
its line, a display centered on its own line and scrolling sideways, as
ChatGPT and Claude show math before it is typeset. A KaTeX that cannot load
leaves the source and says once *Math Formatting Is Unavailable*.

Math follows Pandoc's rule for dollar signs, the same in a file and in the
agent's replies, which render through the same Markdown renderer. `$$…$$` is
display math. `$…$` is inline math only when the opening `$` has a
character other than a space right after it, and the closing `$` has one
other than a space right before it and no digit right after it. So prices
stay text: in `Notion is $10 vs $12`, the `$` before `12` cannot close,
since a digit follows it, and both dollar signs show as typed. Inline math
may touch the text around it, as Chinese writing does: `面积为$x^2$平方米`
renders `x^2` as math.

A reply that streams renders a block at a time, so the work of each update
is the reply's last block, not the whole reply. The reply's source splits
into the top-level blocks the Markdown renderer's own block parser finds,
read from the reply's finished lines only, since the line still being
written can change what the lines above it are, as a half-written row
turns the lines above it into a table; a block is settled once a block
after it begins in those finished lines, so a second item that joins a
numbered list joins it before the list settles. A display math block not yet closed, or
a raw HTML block, keeps itself and everything after it unsettled, since
only what follows decides what it holds. A settled block is rendered once,
sanitized once and kept in the page unchanged, together with the files and images it
names, and only the last block renders again as text arrives. A settled
block is never rendered again while the reply streams, so its elements stay
the same nodes and the page lays out only what changed. When the reply
ends, it renders once whole, which catches what only the whole reply
decides, such as a link whose reference is defined further down; a
difference that render finds replaces only the blocks it changes. For
example, an essay of forty paragraphs streams with thirty-nine settled
paragraphs on the page and one that grows.

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
Host. A user's message, in the composer and as a sent bubble, shows every
image it names as an attachment capsule, as a file the user attached, never
as a picture in its text: the user's message is text and attachments. What
follows of pictures in text is the agent's. For example, the agent's reply
contains an image and a link:

```markdown
![Weekly load](out/chart.png)
[plot.py](scripts/plot.py)
```

The chart loads from the Host the way a document's image does, and shows the
file as it is now: the page keeps each shown image's description, its size,
time and version, follows the file on the conversation's file watch, and
names that version in the image's address, so a picture the file replaces
shows in every message that names it, and one the page already decoded never
stands for a newer file. While the description is first read, and when the
file is missing, the image shows its alt text. An image follows its file
while it is on screen; one scrolled away reads its description again when it
is shown. A Markdown document in the File view and the Change view
follows its Host images the same way. A `srcset`, of an `<img>` or of a
`<picture>`'s `<source>`, resolves each Host candidate as an image's
target is resolved, and each follows its file under its own versioned
address, so a README's dark logo, a `<source>` for
`(prefers-color-scheme: dark)`, shows in a dark theme as on GitHub; a
candidate whose file is missing drops out of the set. A `<source>`'s
`media` that asks `prefers-color-scheme` is judged by the theme Demi shows,
not the system's, since a user may pick Dark on a light system: the source
that matches it stays, and the other goes. An image whose address ends in
`#gh-dark-mode-only` or `#gh-light-mode-only`, GitHub's older way to say the
same, shows only in that theme. A click on `plot.py` opens it in
the File view. A link to a Host file shows the file's
icon before its name, the icon the file tree shows for it, so a file reads as
a file and not as a page on the web, and a commit link shows the commit
icon, Lucide's `git-commit-horizontal`, in orange as Git's own mark is, before its text.
Their color says where they lead: a link to a file or a commit leads into the
conversation's own work and reads as part of the sentence, as an agent's link
does, in the prose's color over a faint underline, while a web page's link
keeps the accent color a link has on the web. One fixed color for each kind
would meet the accent the user picks: with an orange accent, a web link would
read as a commit. A link that starts with an icon, a
file's icon, a commit's or an agent's mark,
is a small object in the sentence, as a mention is in Slack or Notion, so it
keeps 0.2 em of space on each side, inside its hover fill where it has one
and outside its underline: in "已经告诉[负责的 Agent](agent:57)：", the character before the
mark and the colon after the name no longer touch it. Latin punctuation
right after it, `. , ; : ! ? ) ] }`, takes no space before it, as after any
word: "the main agent;" never reads "the main agent ;". A link without an
icon is words and is spaced as words.

In a message every link wears an icon, so each reads as the kind of thing it
opens: a Host file its file's icon, a commit the commit icon, an agent its
mark, an attachment its file's icon, and a web page Lucide's `link` icon. A
link whose text is an image, as a badge is, wears none, and a Markdown
document's links wear none either, since a document renders as its author
laid it out, as on GitHub.

A link's text may be code, as `` [`runner.md`](docs/execution/runner.md) ``
or `` [`docs`](https://example.com) `` writes it. A link is one color: its
code takes the link's color, never code's own, so the text and its underline
never differ. In a message, where the icon already says what the link is,
the code's look goes and the text reads as `[runner.md](docs/execution/runner.md)`
does; in a document, code in a link keeps its monospace and fill, in the
link's color, as GitHub shows it.

| Target | A link | An image |
| --- | --- | --- |
| Relative path | Resolved against the working directory of the agent that wrote it, where its commands ran when it wrote it, the conversation's for the main agent (`crates/agent-server/src/node/mod.rs`); the file opens in the File view, and a `#L42` or `#L42-L50` after the path, or `:42`, opens it at line 42, scrolled there and marked for a moment as a search result is, in the file's source even where it has a preview, as GitHub's line links show the plain text | Resolved the same way and loaded from the Host |
| Absolute path, a path starting `~/` from the Host's home, or a `file://` URL | The file at that path, opened in the File view | Loaded from the Host |
| `http` or `https` URL | Opens in a new tab of the user's browser | Loaded from that URL |
| `commit:3f2a9c1`, a commit of the repository the agent that wrote it worked in, by its id or at least 7 of its first hex digits; `commit:src/v8@86229da`, a commit of the repository in that folder, relative or absolute | Opens the Change view in Commits mode on that commit, in that repository ([Commits mode](../execution/edit-tracking.md#commits-mode)) | Its alt text |
| `agent:4`, an agent of the conversation | Opens the Agents window on that agent's tab | Its alt text |
| `data:` URL | Shown as text | Shown as it is |
| Anything else | Shown as text | Its alt text |

A Host path names the file as it is now: the image changes when the file
does, and is gone with it, with the Host out of reach, with an archive or with
a target change. A file the agent gives the user is therefore an attachment,
a copy that stays, delivered as its block; a Host path suits a file of the workspace the user should
see as it is, such as a chart the repository keeps. An attachment block whose number the
conversation does not have shows *No attachment a9 in this
conversation* in its place. A number above the conversation's latest names one still to
come: for example, a reply names `a9` before the agent uploads it, and shows
it missing; the upload stores `a9`, the conversation's summary carries 9 as
its latest attachment ([Sidebar mutations and read state](web-api-conversations.md#sidebar-mutations-and-read-state)),
and that reply and every later one that names `a9` show the image. A
number up to the latest that has no attachment never gets one. The product's instructions tell the model that its messages
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

An image is a picture in the message, not the message: it is a tile, scaled
down with its proportions kept to fit a box a third of the width it stands
in, the message's or a list item's,
less the two 8-pixel gaps between three tiles, and 240 pixels tall, never
enlarged, so three pictures fill a line. For example, in a message 760
pixels wide a tile's box is 248 × 240: a 2560 × 1720 screenshot shows
248 × 167, and a 1000 × 8000 page 30 × 240. An image always stands on a
line of its own, with 12 pixels between it and the text above and below it,
even where the Markdown puts it in the same paragraph as the text, such as
on the line after a list item's words. A tile whose picture's size is
known before it loads, as an attachment's is, takes its box at once; one
whose size is not, as a Host path's or a web image's, takes no room until
it loads and then grows once to its tile, since a box held at the largest
size would leave a blank band under every smaller picture. Resting the pointer on it shows it
large ([A picture's preview on hover](#a-pictures-preview-on-hover)). A click shows the image whole in the media viewer, a
Host image and a web image alike. An image inside a link follows the link.

Images that follow each other show side by side. For example, the agent
replies with four screenshots, one per line, and nothing else between them:
they stand in a row, left to right in their order, as many as the message's
width holds, and the rest wrap onto the next row. A run is two or more
images or videos with nothing but white space between them, in one
paragraph or in paragraphs that follow each other; any text, list or heading
ends it. In a run, each image and each video's first frame is a tile, as above
([Media a tool returned](#media-a-tool-returned)); they stand 8 pixels apart
in both directions, and the run stands apart from the text as a lone image
does. Text before the first image in the same paragraph does not stop the
run: a list item whose words are followed by two images, one per line, shows
its words, then the two side by side. 

A link to a file opens the `file` intent; a click on an image a message
shows, a Host image as any other, opens it in the [media viewer](#the-media-viewer),
as a tool's screenshot does, since a picture in the reading is looked at
where it is read. While no plugin the user has on opens files, a file link
is shown as text. A `:line` suffix on a path is dropped, since the File view opens a whole file.
Only a Markdown link or image names a file: a path in code or plain text stays
text, because the page cannot tell which paths exist without asking the Host
for each. Paths name files on the conversation's Host; a file on another Host
the conversation reaches does not show. Loading an image is a Host operation,
so it wakes a stopped Cloud like any other
([Sessions and targets](../execution/sessions-and-targets.md#host-operations)).

### Attachment blocks

A file the agent delivers to the user stands in its reply as a block of its
own, as a file stands in a Notion or Feishu document, not as a link in a
sentence. The agent writes `<attachment id="a3" />` on a line of its own,
naming an attachment it uploaded ([Attachment commands](../execution/commands.md#attachment-commands)),
and the reply shows it by what it is:

- **An image or a video** as the reply shows one today, at an image's
  bounds, a click opening it in the [media viewer](#the-media-viewer).
- **Audio** as its inline player.
- **Any other file** as a card the reply's width at most: the file's icon,
  the one the file tree shows for it, at the left; its name, and its kind
  and size in secondary text, *PDF Document · 2.4 MB*, the kind as the File
  view names it, beside it; and
  Download at the right end. A click on the card opens the file in the work
  panel, previewed as the File view previews a file but from the
  attachment's copy, with its name, its size and Download in the preview's
  bar; only Download downloads.

Images and videos on consecutive lines sit side by side as a reply's images
do; cards stack, each its own. In a shared conversation a card shows its
name, kind and size and Download, and a click on it does nothing else, as a
share has no work panel.

What is a block is decided once, by one rule both the page that renders a
reply and the backend that lists a reply's attachments for a share follow,
so a share serves exactly the files its reply shows and no other: a line
whose whole content, after the Markdown that places it (a list item's
marker, a quote's `>`), is the tag. The tag is `<attachment id="a3" />`, the
space before `/>` optional and nothing else varying: single quotes, a
closing `</attachment>` or another attribute make it text. A tag in code
(fenced, indented or inline), in an HTML comment or block, or in a line
with anything else on it, emphasis included, is text, and names no
attachment the share serves. The block is the only way
a reply shows an attachment: a link or an image whose address is
`attachment:a3` is an address the reply does not open, shown as its text, as
any other one is. The tag in the middle of a sentence is not a block and
shows as its text, as Markdown shows a tag it does not render. A number the conversation does not have
shows *No attachment a9 in this conversation* in the block's place, and so
does a number in an agent's reply that its own uploads never printed, the
user's files and another agent's included: a block shows only what the agent
that wrote it uploaded, never a file it guessed the number of. A side
question's agent is an agent of its own here, and a subagent forked from
another wrote, in the history it copied, what that agent wrote, so a copied
reply still shows what it showed, and a
number still to come shows once the upload stores it, as below.

## Media a tool returned

For example, the agent runs `demi browser screenshot t1 | demi file view`.
The call's result attaches the PNG as an image the model reads
(`crates/command-package-file/src/files.rs`). The page
shows the same picture small in the call's row, after its title, whether
the call is folded or open, and a click opens it in the
[media viewer](#the-media-viewer):

```text
▣ Take screenshots of the login page ›  [▭][▭][▭][▭] +5
  the call's row: its title, then what the model read, in one line;
  a click on a picture opens it large; +5 counts those that do not fit
```

- **Which calls.** A call shows the images and videos its own result carries,
  and a command's notice those its report carries, so a picture shows
  where the model saw it; a folded run shows those of its calls and notices. A `shell` call whose command
  exits within the call's window carries the media the job viewed, several
  of them when it viewed several. When the command exits after the
  call returned, the end report
  carries them instead. The generic tool card shows its result's media the
  same way.
- **Where.** In the row's own line, at its end, right-aligned, side by side
  4 pixels apart in the order of the result, at least 8 pixels after the
  last part of its face, the chevron or a tag. Each is 20 pixels tall, as wide as its proportions make it
  within 16 and 48 pixels, cropped as below, with 4-pixel
  corners. They never wrap and the row stays one line: as many show as fit
  in the width the title leaves, and the title gives way, truncating, only
  down to half the row; the rest are counted after the last one shown, *+5*,
  in the row's faint text, which is not a control. A run's row folded shows
  those of its steps the same way. A row's pictures are a glance at what the
  step saw, not the place to look at them, which the viewer is. Every medium
  takes its box before its bytes arrive, so the row does not move when they
  load. The fold
  still shows the command's own output, whose lines
  `[image 1: image/png, 1280 × 720 px, 412000 bytes]` stands for the
  original file the job viewed, and `<binary stdout: 412000 bytes>` for bytes
  the model did not see.
- **An image** in a row, whose proportions fall outside its box's range, is
  cropped, not squeezed: a tall one, such as a whole page's screenshot,
  keeps its top, and a wide one its middle. It is never enlarged, so a
  16 × 16 icon stays 16 × 16. The medium's reference, and an attachment's
  record, carry an image's pixel size, so its box is known before its bytes
  arrive; only a medium whose size is unknown until it loads, such as a
  video before its first frame, takes the largest box, so nothing shrinks
  when it arrives. A click opens it in the [media viewer](#the-media-viewer).
- **A video** never plays in place. Its thumbnail is its first frame, sized
  as an image's, in a row or in a message, with a play mark over its middle. A click opens it in the
  [media viewer](#the-media-viewer), scaled down to fit and never enlarged, where it
  plays at once in the player built into the user's browser, whose controls
  and full-screen control it keeps; closing the viewer stops it. A video's
  reference carries its pixel size where the backend reads it from the
  container's header (MP4 and QuickTime, WebM), and a thumbnail of unknown
  size is 16:9 until its first frame arrives.
- **On a phone**, the preview fits the conversation's width. An image opened
  large fills the screen: a tap toggles actual size, at actual size a drag
  moves the picture, a pinch zooms as anywhere on the page, and the close
  control closes it, as does a tap on the empty space around the picture,
  since the viewer covers the screen and leaves nothing else to tap. A video opened from its thumbnail plays in the player of
  the user's browser, which an iPhone shows full screen.
- **A medium that is gone.** A result can hold, in a medium's place, a part
  that says its bytes could not be stored ([Media](../agent/transcript.md#media)).
  The page shows one line under the row, with the store's reason, such
  as *Video not stored: the object store refused the write*. The model reads
  the same fact in a text of its own. A medium that was stored stays.
- **A medium that cannot be shown.** When the page cannot show a medium,
  because its blob is missing, the request failed or the user's browser cannot
  decode it, *Could not show this image.* (or *video*) shows in its place, at
  the same height, and a reload of the page tries again. A blob that a block
  references is never deleted ([Media](../agent/transcript.md#media)), so a
  missing one is a fault; a request that replays the medium then carries
  `[missing image blob <ref>]` in its place.
- **The model's picture.** The page loads the blob the result references,
  the same bytes the model receives, so it shows exactly the picture the
  model saw: the image as it was fitted when it entered the transcript
  ([Images in the transcript](../agent/transcript.md#images-in-the-transcript)),
  and a blob is named by the hash of its bytes, and nothing changes them. The
  original bytes stay in the command's whole output, which the model reads
  with `demi shell output`
  (`crates/agent-server/src/server/shell_output.rs`).

### A picture's preview on hover

The user rests the pointer on a screenshot in a call's row. After 300
milliseconds a card rises over it with the picture large and a line under
it, as a link preview does, so a glance needs no window:

```text
        ┌──────────────────────────┐
        │                          │
        │      the screenshot      │   at its own size, up to 70% of the page
        │                          │
        ├──────────────────────────┤
        │ Interacted with page     │   what the picture belongs to
        └──────────────────────────┘
 ▣ Click Sign In ›  [▭][▭] +3          the row; the pointer on the first
```

- **Which pictures.** Every picture the page shows smaller than this card
  would: a call's, a notice's and a run's in their rows, a picture in the agent's
  message, and an image or video capsule in a user's sent message. Every
  picture in the agent's message is a tile smaller than the card, so each
  has one.
- **The card.** The picture at its own size on the screen, its pixels shown
  one to one: its pixel size divided by the screen's pixel ratio, as a
  browser shows an image at 100%. A picture larger than 70% of the page's
  width or 70% of its height is scaled down to fit them, so the card never
  covers the page. For example, a 2000 × 295 screenshot taken on a Retina
  screen shows at 1000 × 148 on one; a 2560 × 1600 one shows at 1280 × 800 on
  a large enough window and scales down on a smaller one. It is never
  enlarged past that size,
  over the checkerboard where it is transparent; a video shows its first
  frame and does not play. Under it, one line: the call's or notice's
  title for a step's picture, the alt text for a message's, a capsule's
  file name, cut at its end, then the pixel size in the faint text. The
  card's box is known before the bytes arrive, from the medium's pixel
  size, so it never grows as it loads.
- **Where.** Above the picture, centered on it, moved to stay within the
  page and below it when the page has no room above. When neither side has
  room for it, it goes on the side with more room and is scaled down to fit
  there, so it never covers the picture it shows. It covers what is
  under it and takes no pointer, so the pointer moving off the picture
  closes it at once, and moving onto another picture shows that one's
  without the delay, as tooltips after the first do.
- **What it does not change.** A click on the picture still opens the
  [media viewer](#the-media-viewer), which closes the card. A phone
  has no hover and shows no card.

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
  320 × 240, whether the user drags it smaller or the picture is tiny, and
  no larger than the page, and its title bar never leaves the page.
- **Its size follows the picture.** Every picture opens the window at the
  picture's actual size, the size at which one of its pixels covers one
  pixel of the screen: a 2880 × 1800 screenshot on a display of two device
  pixels per page pixel opens 1440 × 900 page pixels large, plus the title
  bar. A picture whose window would be wider than 60% of the page or, title
  bar included, taller than 70% of it opens scaled down, keeping its proportions, to the largest size within
  both; a picture smaller than 320 × 240 sits centred in a window of that
  size. Every picture opens the window at its own size centred on the page,
  the first one and each picture shown after it, by a click or a step. A size the user dragged lasts while that picture
  shows; a video opens at its frame's size by the same rule. A picture
  whose size is not known yet opens the window at the largest size and
  takes its own size centred on the page once it loads, unless the user has
  moved the window by then.
- **One viewer.** Opening another picture or video shows it in the same
  window, in place of the one it showed.
- **Previous and next.** The arrow keys, left and up for the previous and
  right and down for the next, while the focus is in the viewer, show the picture or video before or after this one in the
  transcript it was opened from, the conversation's or a subagent's: the
  images messages show, the user's attached images and the media tools
  returned, in the order they stand, from the oldest to the newest. At the
  first or the last nothing moves. A message's Host image is among them
  by its file, whose version the viewer follows as the message does, so an
  open viewer shows the file as it is now; one whose file turns out to be
  missing shows *Could not show this image.* when stepped to. The title bar counts them, *3 of 12*. A
  picture the transcript has not loaded yet, above the part the page holds,
  is not among them. While the focus is on a video, the arrow keys are the
  player's, which seeks with them. On a phone, where the viewer takes the
  whole screen, a swipe left or right steps as the keys do.
- **What it shows.** An image opens fitted to the window, scaled down and
  never enlarged, over a checkerboard where it is transparent; its pixel
  size and the zoom show in the title bar, *1280 × 720 · 50%*, where 100% is
  the actual size above.
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
  session's windows share ([Recovering an unfinished turn](turns.md#recovering-an-unfinished-turn)); and so does
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
([Uploads and media](web-api-hosts.md#uploads-and-media)): Safari plays a video only
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

In Commits mode, a file with a preview shows its committed version beside
its working-tree version. An added file has only the working-tree side, a
deleted file only the committed side, and a renamed file takes its committed
side from the old path. The two columns stack when the panel is narrow. A
file without a preview shows a card per side, each with Download. A committed
version over 8 MiB, which the committed side's route cannot serve, shows a card
in its column saying it is too large to show, without Download; the
working-tree side is unaffected.

Conversation mode has no media previews: a call's retained edits are text
only, and a binary edit keeps no contents
([Edit tracking](../execution/edit-tracking.md#rationale)). Such an edit
says so in words a user reads without the design, *Demi keeps the text of
the changes a conversation makes, not pictures or other binary files.*,
with **Open File**, which shows the file as it is now.

## Getting the bytes

Text, including Markdown and SVG source, comes from the text route: at most
8 MiB of text, a byte that is no UTF-8 shown as U+FFFD, and a file that is
binary by the one rule (`crates/command-package-file/src/files.rs`)
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
| The runner | Moves file contents through pipes (`crates/runner-host/src/files.rs`). |
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
