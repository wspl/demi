---
status: proposed
---

# A reply shows a widget

## Problem

Alex asks an agent how the shop's orders changed this quarter. The agent can
write a table, or draw a chart into an HTML file and attach it, which the
reply shows as a file card: Alex opens it in another view, and on his phone
he leaves the conversation to see it. A chart, a diagram or a small
interactive page belongs in the reply, where the words that explain it are.

## Decision

**What Alex sees.** The reply shows the page in place, as wide as the text,
between the sentences around it:

```text
  Orders rose in each week of the quarter:

  ORDERS · Q3
  12,480
  ╭──────────────────────────────────────────╮
  │                              ▁▂▃▅▆▇█●    │
  │  ▁▁▂▂▃▃▃▄▄▄▅▅▅                           │
  ╰──────────────────────────────────────────╯
  W27          W31          W35          W39

  The jump in September follows the new checkout.
```

The reply draws a card around the page, a line with round corners, so a
chart or a screen's mockup stands apart from the text alike. The page fills
the card and keeps its own room inside it, so a picture or a mockup's
background can reach the edges. It draws on the reply's background in Alex's
theme, tone and accent, and draws again when he changes them. Its scripts
run; a link opens in a new tab. Content taller than the frame scrolls inside
it. A button in the card's corner, always shown, opens the widget in a tab of
the work panel, as an attachment's card opens its file there: the page fills
the panel, beside the conversation, and a second click selects the same tab.
A shared conversation, which has no work panel, and an archived one, whose
panel takes no change, open the page in a new tab of the web browser.

**What the agent does.** A widget is an HTML page a reply shows in place, a
thing of its own: an attachment is a file the user receives. The agent writes
the page into `demi widget create` on its standard input, a file with `<`:

```console
$ demi widget create <<'EOF'
<!doctype html>
<html><head><meta name="demi-frame" content="height: 330px">...</head>
<body>...</body></html>
EOF
w1  height 330px
```

and places it in the reply on a line of its own:

```html
<widget id="w1" />
```

The command shows the agent the widget as the reply will draw it: the Host's
browser draws the page at the reply's width, in the default tone and accent,
once light and once dark, and the command hands the two screenshots to its
job, as `demi file view` hands media. A page whose content is taller than the
height it declares is named, with both heights, since its screenshot shows
only a page that ends too soon. A Host without the browser still gets
the widget, and the command says the screenshots were skipped and why; so
does a command an attached Host runs, since the conversation's primary Host
draws them.

A widget keeps its page as it was written; a changed page is created again,
as `w2`. Widgets are numbered across the user's conversations. A widget
belongs to the conversation it was created in and to each conversation
forked from one it belongs to; when the last of them is deleted, the widget
goes. A widget's name, which a download of it takes, is its page's
`<title>`, else `widget-w1.html`. A file meant for Alex to download, such as
a site the agent built, stays an attachment.

**The frame's size.** A page declares its frame in its head, in CSS's words
and meaning, and `demi widget create` prints what it read:

```html
<meta name="demi-frame" content="height: 360px">
<meta name="demi-frame" content="aspect-ratio: 16 / 9; min-height: 240px; max-height: 480px">
```

Only `height`, `aspect-ratio`, `min-height` and `max-height` count, lengths in
`px`, and a height counts the page's room at its edges. The frame has that size
from its first moment; content that turns out taller, as its text wraps in a
narrower reply, grows it to hold it, at most the window's height, which moves
the reply below it but never cuts the content. Its width is the reply's, which
follows the reader's window, from about 320px on a phone to the 798px the
screenshots are drawn at, so a page lays itself out for any width. A page that
declares nothing starts 320px high and follows its content's height as it
reports it, at most the window's: a page sized by the viewport would otherwise
grow without end.

**The default theme.** Every widget's page gets, ahead of its own head:

* a `<base>` that opens every link in a new tab;
* Tailwind's browser build from its CDN (`@tailwindcss/browser@4`);
* a `@theme` with the color and font names of the product's own Tailwind
  theme (`bg-surface`, `text-fg-muted`, `border-line`, `bg-series-1`), the
  same names as the root's properties for a script, and the product's theme
  values, which follow the root's `data-theme`, `data-tone` and
  `data-accent` as the product's own pages do;
* a base sheet: no background, the reply's font, text size and color, and
  the theme's `color-scheme`, without which a dark page's frame paints white.

A page overrides any of it with its own `@theme` or CSS, which comes after.
It loads anything else it needs from a CDN, such as a chart library.

**The guide.** `demi widget guide` prints how to make a widget: the theme's
names, the frame, the style, and that the reply does not describe what its
widget shows. `demi widget guide <kind>` prints a page to copy for each
kind: `line`, `bar` and `scatter` with Chart.js 4, `heatmap`, `share`,
`table` and `flow` with Tailwind and SVG. The style:

* No title bar, card, border or background of the page's own, and 24px of
  room inside the card's edges for all but what is meant to reach them.
* A header of a small capitalized label, one large number and a change pill
  or a quiet note.
* One subject in the accent Alex chose (`accent-fill`), the rest the same
  color faded or gray; only categories that must be told apart take the
  data colors (`series-1` onward). A chart redraws in a new accent as the
  rest of the page does.
* Smooth lines with a fading area under the subject and a halo on its newest
  point; capsule bars; cells and nodes with large corners and no border.
* No axis lines, value ticks on the right, three or four of them, faint
  dotted grid lines.
* A neutral fill is the text color faded to about 8%, never a surface color,
  which on a light theme is the background itself.

The frame cuts off anything drawn outside it, so nothing is: no floating
tooltip, and a highlight's ring is drawn inside its element. Hovering a
chart shows the hovered point in the header, its number and note, and marks
it in place (a rule and a point, a lit bar, a ringed cell); leaving the chart
shows the totals again.

**When the agent makes one.** The system prompt says it in the one line it
gives every command group:

```text
demi widget: Charts, diagrams and interactive pages in your reply: data with a shape, such as a trend, a comparison or a distribution, is shown, not told.
```

Three familiar kinds name what a widget is, each a whole class: data,
structure and something to use. *Data with a shape* names when to make one
by what the data is, and its three examples are the classes a chart draws;
*shown, not told* says what the agent does with it. A few numbers, a list,
code or a fix have no shape, and stay words.

**A shared conversation** shows the widgets its replies name, and no other.

**The switch.** Widgets are a plugin, Widgets in Settings › Plugins, on by
default. Off, the agents have no `demi widget` and its line leaves the
prompt, and a widget already in a reply says in its place that widgets are
off.

**The design.**

* The plugin `widget` declares the group `demi widget`, two `rpc` leaves:
  `create`, whose page is its standard input (at most 1 MiB, as every
  command body), and `guide`.
* `create` keeps the page as a blob of the user's and the widget as the
  plugin's value `w<n>`, which names the blob and so keeps it; the value
  `count` numbers them. It then asks the browser package to draw the page,
  twice, reads each picture as a Host file and hands it to its job.

  ```rust
  pub struct Widget {
      pub number: u32,
      pub name: String,
      pub frame: FrameSize,
      pub page: BlobRef,
      /// The conversations whose replies can name it.
      pub conversations: Vec<String>,
  }

  pub struct FrameSize {
      pub height: Option<u32>,
      pub aspect_ratio: Option<AspectRatio>,
      pub min_height: Option<u32>,
      pub max_height: Option<u32>,
  }
  ```

* A reply's data carries each widget it names, its frame and its name, as it
  carries the attachments it names, so a reply keeps a widget's room from its
  first moment, before its page loads, and the page asks for nothing but the
  widget's document.
* The page and its frame are read from the page as a browser would read it:
  comments and the content of scripts, styles and other raw-text elements
  are not markup, and a tag ends past its quoted attributes.
* Mechanisms the plugin needs join the contracts for every plugin:
  * **An `rpc` command hands its job media.** The rpc port gains `medium`, a
    medium the job keeps and shows the model as it keeps what
    `demi file view` hands it. The relay sends its facts as `rpc_medium` and
    its bytes in a pipe of their own, since bulk bytes never travel in a
    message; the runner reads them before the call's next message, so the
    medium stays before its exit.
  * **An `rpc` leaf calls a package's operation.** A manifest's commands
    name the operations their `rpc` leaves call through the port (`calls`),
    which bind the tree as a native leaf's do: a tree whose operation the
    catalog does not serve is left out whole.
  * **A plugin's package call carries an input.** A plugin hands an
    operation bulk content, such as a document, as the call's input stream
    (`package_call_reading`), not as an argument: an invocation's arguments
    are metadata, which the command protocol bounds at 256 KiB.
  * **The browser package draws a document.** `demi.browser` gains `draw`:
    an HTML document, which it reads as its input, drawn by a Chrome of its
    own, with a profile of its own and no tab of any conversation's, at a
    width and a height or its own height, as a PNG at twice the CSS pixels in
    a file its caller reads as a Host file. The page is told the reader
    prefers reduced motion. Each drawing removes the pictures older than ten
    minutes; a cancelled one answers at once and cleans up after.
  * **A plugin learns of a conversation's deletion and fork.** A manifest
    says it keeps something of a conversation (`conversations`), and the
    plugin is told, on or off, when the user deletes one or forks one into
    another (`conversation_deleted`, `conversation_forked`). A deletion's
    pending record goes only once every such plugin answered, so one that
    failed is told again at the next start. A fork tells them while it
    copies the source's history, before the destination's history commits,
    so one that failed fails the fork, and a destination a start publishes
    was told already; an attempt never retried leaves its destination among
    a widget's conversations, as it leaves its copies. The user's forks and
    deletions take turns, a fork's from reading its source until its history
    commits, so a deletion of the source never reaches a plugin between a
    fork's reading and its writing. A plugin answers either idempotently,
    and a widget's conversations change by a write that reads the widget
    again when another came first.
  * **A plugin's page draws blocks in replies.** A manifest's page names the
    tags of its reply blocks and whether each has an HTML document
    (`blocks`), and the page gives each tag its component:

    ```ts
    interface ReplyBlock {
      /** A lowercase word no HTML element and no other page's block has. */
      tag: string
      /**
       * Drawn with the page's context, the line's `id`, its `facts` and
       * `open`, which opens the block whole in the work panel.
       */
      component: Component
    }
    ```

    `open` asks for the intent `block { tag, id, title }`, which only a
    panel kind of the page that draws the tag opens; the widget page's
    `widget` kind keys its tabs by the widget's id. A share has no panel, so
    its blocks get no `open`.

    The reply's renderer reads a line `<tag id="..." />` of a tag a page
    owns as it reads an attachment's line, and leaves the place of the
    page's component, which the reply's Markdown fills as it fills an
    attachment's card. The page host lists the product's pages, whichever
    plugins are on, and the backend reads a reply's text by the same tags,
    which its start registers from the plugins' pages: any other tag's line
    is HTML to both.
  * **A reply's data carries its blocks' facts.** What a block's component
    needs before anything loads, a widget's frame and name, travels as an
    attachment's description does: a transcript's page, its socket's frames
    and a share's page carry the facts of the blocks their texts name, read
    when they are sent and never stored, and a frame that completes a tag in
    pieces carries its block's. The backend asks the block's plugin, on or
    off, since a block keeps its room while its plugin is off
    (`ReplyBlocks { tag, ids }`), and the plugin answers each id's facts, or
    none for a block the user does not have. A block's facts never change,
    as an attachment's do not: a changed widget is another widget. The
    component reads them from its `facts`, and the page's plugin says
    whether it is on (`enabled`). A block named before it was made, or read
    while its plugin failed, has none, and a reply that names it again, or
    a later read, brings them; a copy the page keeps holds none of it. A page
    that shows a reply naming a block it holds no facts of, as a copy shows
    the replies it kept without a read, asks for them once, with the others
    it lacks then (`POST /api/reply-blocks`).
  * **A block's document is a page of its own.**
    `GET /api/plugins/:plugin/blocks/:tag/:id` serves the document the
    plugin answers while it is on (`ReplyBlockDocument { tag, id }`),
    sandboxed into an opaque origin as the File view's pages are, and the
    component frames it by its URL, never as a `srcdoc`: a framed `srcdoc`
    takes the policy of the page around it, and a shared page's allows no
    inline script.
  * **A share answers a plugin's reply blocks.** Its page carries the facts
    of the blocks its texts name by the attachment blocks' rule, every
    block's tag marked at once so adjacent lines read as the renderer reads
    them. `GET /api/shares/:token/plugins/:plugin/blocks/:tag/:id`, for a
    block a text of the share names and nothing else, sends the frame
    on to `GET /api/blocks/:link`, a link of that block alone, good for a
    minute, which answers by the same rule: a document's scripts read the
    address they load from, and the share's token there would hand them the
    whole share. The shared page's policy lets a frame load the share's
    routes and the links, and its host lists the pages that draw reply
    blocks (`BLOCK_PAGES`, generated as the registry is).
* The page's component frames the widget's document by its URL. The frame's
  name hands the bootstrap the reply's appearance: the root's appearance
  attributes, its font and its text size. A change of them draws the page
  again, since a chart a script drew keeps the colors it read. A plugin
  turned off, while the block shows or before, shows no widget, in the room
  the widget had, and one turned on again shows it again.
* The bootstrap is the plugin's: it takes the product's theme names and
  values from the product's own stylesheets as they are built in, so a
  widget never has colors of its own.
* An older page shows a widget's line as its text until it reloads.
* The gallery's reply shows three of the guide's pages, light and dark, as
  the plugin draws them (`xtask contracts`).

## Spike

* **Tailwind's browser build in an opaque origin.** A sandboxed frame
  (`allow-scripts`, no `allow-same-origin`) that loaded
  `@tailwindcss/browser@4` applied a `@theme` set before the script:
  `bg-surface` drew `#123456`, `p-4` was 16px. A second `@theme` after the
  script replaced `--color-surface` and kept the first one's other names, so
  a page overrides a default name by name.
* **The style.** Pages of each kind of the guide, drawn in a reply of the
  gallery in both themes, settled the style above. A dark page without
  `color-scheme` painted its frame white; a node filled with the raised
  surface color vanished on the light theme; a ring drawn outside the last
  column of a heatmap was cut off by the frame; a canvas gradient from a
  color to `transparent` turned gray, where one between two opacities of the
  color keeps its hue.
* **Heavy pages.** Nine pages agents made on request (a dashboard of four
  charts and a sortable table, 5,000 points with filters, a 150-node force
  layout, a province map, a 20-task Gantt chart, a calculator with a linked
  chart, a three.js scene, a three-screen mockup and a 300-row table)
  loaded in 1.2 to 2.3 s and drew without errors. A reply of 23 of them,
  17,000px tall, scrolled with 95% of frames within 17 ms. A page of 1 MiB
  drew its screenshots in 4 s once `draw` read it as input; as an argument,
  a page over about 200 KB had none.
* **A page that never yields.** In Chrome a widget spinning forever, or
  allocating without end, runs in a process of its own and the reply stays
  responsive; in WebKit it stops the whole page.
* **The entry.** Six prompts, three with data a chart shows (a trend, a
  comparison, a distribution), one either way and two without, put to a
  conversation for each wording of the entry: one that named data with a
  shape made a widget for half of the first three over two rounds, and
  "when it explains better than text" for none. No wording made one for
  the other three.
* **The whole path.** An agent's `demi widget create` of the guide's line
  chart kept it as `w2`, the Host's Chrome drew it light and dark in 3 s,
  1600×660 each, and the reply drew it in its 330px frame. A share of the
  conversation showed it, and answered not found for a widget of another
  conversation. A fork kept it after its source was deleted, and deleting
  the fork removed it.
* **Pages that are not tidy.** A page whose comment held `<head>` and a
  `demi-frame` of 9000px, whose script held another, and whose head had a
  quoted `>` read as `height: 200px`; a page whose content was all
  positioned out of the flow measured its content, 160px, not 1px.

## Prior art

* [t3code](https://github.com/pingdotgg/t3code) gives the agent an
  `html_render` tool: a whole page in the tool's input, checked first by
  `html_preview` with a screenshot from a headless Chrome it downloads, shown
  above the final reply in a sandboxed frame without `allow-same-origin`,
  with theme variables injected. Its frame takes the height the page reports;
  the server measures nine widths ahead to start from. Demi places a widget
  anywhere in the reply and takes the size the page declares; its check is
  the same, inside `demi widget create`.
* Claude's `visualize` tool streams an HTML or SVG fragment into a frame
  inside a frame on its own domain as the model writes it, styled by a CSS
  file of its design system's names, with a Chart.js template in the guide
  the model reads first (`read_me`); the frame grows with the content. Its
  tool's name, `show_widget`, is where Demi takes the word. Demi shows a page
  once it is complete, and keeps the guide's reading on demand.
* Embeds on the web declare their size: CodePen, Observable and StackBlitz
  embeds a height, a YouTube embed an aspect ratio. An embed that takes its
  content's height after it loads, as a Twitter embed does, moves what is
  below it.

## Alternatives

* **An attachment with a `preview` mark:** the agent and the reader would
  meet one name for two things, a file to receive and a page to see.
* **The word embed** (`<embed id="e1" />`): it is an element of HTML, and a
  model writes `<embed src="...">` by habit, a line the page shows as text.
  The other words free in Demi either are elements too (`figure`, `frame`,
  `canvas`), already name something of Demi's (`view`, `panel`, `card`,
  `page`) or say the wrong thing (`artifact`, a lasting work in Claude).
* **Showing the page while the model writes it** (Claude): a page half
  written shows half a layout and no script, and the agent cannot check it
  before Alex sees it.
* **Charts as components or a library Demi serves:** the guide and the
  theme keep charts alike, and an agent already knows the libraries on a CDN.
* **Floating tooltips:** the frame cuts off one near its edge.
* **Full screen:** it hides the conversation the widget belongs to and
  stretches a page laid out for a reply's width across the screen; the work
  panel shows it at a size the user sets, beside the reply. A button shown
  only while the card is hovered never showed, since the frame takes the
  pointer's hover.
* **No card, the page on the reply's background:** a mockup with its own
  edges and room ran into the reply's text, and a page that drew its own
  card made one unlike the next.
* **Room in the card itself:** a page could not reach its edges, so a
  picture or a mockup's background would sit inside a ring of blank.
* **Widgets in the core, beside attachments:** a switch would be one more
  setting of its own, where a plugin's is the one every plugin has.
* **Drawing the check in the conversation's browser:** its tab would appear
  in the work panel, and the browser would start for a screenshot.
* **Checking with `demi browser` on a copy of the page:** the copy lacks the
  bootstrap, so the screenshot shows neither the theme nor the base sheet
  the reader sees, and the agent has to remember to take it.
* **Measuring the content and storing its height:** a height depends on the
  viewer's width, fonts and zoom; it is not the widget's to store.
* **Numbering widgets per conversation:** a fork would have to copy them, and
  their numbers would no longer be the ones its replies name.
* **An entry that leaves the choice to the model** ("when it explains better
  than text"): a model reads it as permission, not as a reason, and answers
  a series of numbers in text; one asked for a picture made a widget, so
  the entry, not the command, was what it lacked.
* **Pictures and media in messages:** a message is at most 4 MiB and holds
  up the connection while it moves; a picture is bulk bytes, which go in a
  pipe or a Host file.

## Open questions

* WebKit draws a sandboxed frame in its page's process, so a widget's script
  that never yields stops the whole page in Safari. No setting of the frame
  avoids it; the agent's screenshots, drawn in Chrome, do not show it.
