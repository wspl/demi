# Widgets

A widget is an HTML page a reply shows in place, as wide as the reply's text, in
a card on the reply's background, in the reader's theme. Make one when a chart,
a diagram or something to try explains better than text; a few numbers, a list,
code or a fix stay text.

1. Write one whole page and create it: `demi widget create <<'EOF' ... EOF`.
   It prints the widget's ID, such as `w1`, and shows you two screenshots,
   light and dark. Fix what they show wrong and create it again.
2. Put `<widget id="w1" />` on a line of its own in the reply, where the
   widget belongs among its sentences, and do not describe what it shows.

`demi widget guide <kind>` prints a page to copy for each kind: `line`,
`bar`, `scatter` (Chart.js), `heatmap`, `share`, `table` and `flow` (Tailwind
and SVG). Copy the nearest and change its data and words; keep how it draws.

## The frame

Declare its size in the head, in CSS's words; the frame keeps it from the
first moment, so nothing in the reply moves:

    <meta name="demi-frame" content="height: 320px">
    <meta name="demi-frame" content="aspect-ratio: 16 / 9; min-height: 200px; max-height: 420px">

A chart takes a height; a drawing that scales as a whole takes an aspect ratio
with bounds. Only `height`, `aspect-ratio`, `min-height` and `max-height`, in
`px`, count, and a height counts the page's room at its edges. A height is where
the frame starts: content taller than it grows the frame once the page loads,
which moves the reply below it, so declare what holds the content at the
screenshots' width. Without one the frame takes the page's height after it
loads, which moves the reply below it. Never size the page by the viewport
(`vh`, `h-screen`) unless it declares an aspect ratio.

The frame is as wide as the reply, which follows the reader's window: from
about 320px on a phone to 798px, the width the screenshots are drawn at. Lay
the page out for any width in between, with no fixed width: what grows
taller as it narrows grows the frame. What it draws fills that
width: a chart sizes itself to its box, as the pages' Chart.js does, and an
SVG takes the whole width with the height its aspect ratio gives, never a
fixed height, which leaves its sides empty.

The frame cuts off whatever is drawn outside it: no floating tooltip, no
shadow or ring outside an element (`ring-inset`), and leave the edges'
contents room.

## The theme

Tailwind (v4, from its CDN) and the product's own names are there before your
head, which may override them:

- Surfaces: `surface`, `surface-raised`, `surface-float`, `sunken`.
- Text: `fg-emphasis`, `fg`, `fg-body`, `fg-muted`, `fg-subtle`, `fg-faint`.
- Lines: `line-subtle`, `line`, `line-strong`.
- Accent, the color the reader chose: `accent-fill` for shapes and fills,
  `on-accent` for text.
- Status: `on-success`, `on-danger`, `on-warning`, `on-info`.
- Data: `series-1` to `series-7` for categories told apart, and
  `series-reserved` for what is set aside.
- Shapes: `rounded-sm` to `rounded-3xl`, `font-sans`, `font-mono`.

Use them as classes (`text-fg-muted`, `bg-series-1`) or, in scripts and SVG,
as properties (`var(--color-accent-fill)`). Never write a fixed color: the page
draws again in the reader's theme. Load anything else from a CDN.

## The style

- The reply draws the card: a line with round corners around the frame. The
  page draws no background, border or title bar around itself, and fills the
  card; what it shows inside, a phone's outline or a mockup's own cards, it
  draws as it would anywhere.
- Keep 24px of room inside the card's edges (`<body class="p-6">`); only what
  is meant to reach them, such as a picture, a map or a mockup's own
  background, goes to the edge.
- A header: a small capitalized label (`text-[11px] font-semibold uppercase
  tracking-wider text-fg-subtle`), one large number (`text-[34px] font-bold
  tracking-tight text-fg-emphasis tabular-nums`) and a change pill or a quiet
  note.
- One subject in the accent (`accent-fill`); the rest the same color faded
  (`color-mix(in oklch, var(--color-accent-fill) 30%, transparent)`) or gray.
  Categories that must be told apart take `series-1` onward instead.
- Smooth lines with a fading area under the subject and a halo on its newest
  point; capsule bars; cells and nodes with large corners and no border.
- No axis lines, value ticks on the right, three or four of them, faint
  dotted grid lines.
- A neutral fill is the text color faded to about 8%, never a surface color,
  which on a light theme is the background itself.

A chart animates in only when the reader allows motion: the screenshots are
drawn with `prefers-reduced-motion: reduce`, and a page that ignores it shows
its chart mid-animation there.

Hovering a chart shows the hovered point in the header, its number and note,
and marks it in place: a rule and a point, a lit bar, a ringed cell. Leaving
the chart shows the totals again.
