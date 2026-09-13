# Shared control layout

`web-ui/ui/TextInput.vue` owns input sizing and padding. Its outer tooltip wrapper
receives the caller's layout classes and styles (`flex-1`, fixed widths and maximum
widths). The frame fills that wrapper; native input attributes and keyboard/focus
listeners go to the input. Bare fields stretch in their row and omit the normal
frame padding. `CommitTextInput` and `TokenInput` inherit these rules.

Size families are 24, 28 and 36 pixels for `sm`, `md` and `lg`. Ordinary horizontal
padding is 8, 10 and 12 pixels respectively. A prefix or suffix uses 6 pixels of
spacing beside the input text. Tailwind utilities must appear as complete literal
class names in shared source; constructing class fragments at runtime can leave
styles out of the production bundle.

`web-ui/ui/CopyCode.vue` vertically centers command text and the copy control,
including when a long command wraps. `Dialog` and `ScrollArea` own height and scroll
constraints; content with a fixed header opts out of the dialog's whole-content
scroller and supplies its own body scroller.

## Production-layout checks

The Gallery's `/control-layout` page uses the actual shared components. It includes
36 combinations: three input sizes, six variants (plain, prefix, suffix, password,
disabled and bare), and two container widths (240 and 400 pixels). Fixed-width
inputs and single-line/wrapped commands are included separately.

On 2026-09-10, browser measurements against built Gallery CSS verified all 36
combinations: expected padding and height, frame/wrapper width agreement, full
allocation of the row beside its action, and no horizontal overflow. Fixed inputs
measured 128 pixels. All four command examples had matching text/control vertical
centers and no horizontal overflow.

Product browser checks also verified the Claude setup-token dialog, account name,
settings search, change-password fields and manual model editing (including
context/output token inputs). The token input has 10 pixels of left
padding and expands into the space beside Continue. Its command text and copy
control have matching vertical centers. No credentials were submitted and no model
requests were made during these checks.

This verifies the listed surfaces and dimensions, not every screen, browser or
viewport. To repeat the check, build `packages/web` and `packages/web-gallery`, open
the production Gallery preview at `/control-layout`, and inspect the computed
padding, bounding rectangles and horizontal overflow of the labeled cases. Compare
against the same controls in account, provider and model dialogs in the product.

## Filtering surfaces

Filter dialogs use `web-ui/ui/FilterDialog.vue`: a stable 36rem preferred height,
capped by `Dialog` to the viewport minus its outer margins. Typing, clearing the
query and displaying zero results do not resize or recenter the dialog. The title
and search stay fixed; only the results scroll. Add provider uses this component.
Gallery's provider page uses the same dialog.

All visible text that matched a filter must use the shared `HighlightText`
component, which performs case-insensitive literal matching and uses the shared
highlight color tokens. Render matched text as text, never injected HTML. If an
alternate field caused a match (for example a vendor ID rather than its name),
show that field and highlight it so the result is understandable. Empty queries
have no highlights. Keep the empty-result message in the same results region.
The generic `Menu` already follows the label-highlighting rule.

## Segmented selection

`web-ui/ui/Segmented.vue` blends its dark selected thumb with the actual parent
surface, using a 20% overlay fill and the shared button outline. It must remain
visibly distinct on base, regular, raised and floating surfaces. Light mode uses
the button fill. The Gallery `/control-layout` page shows both sizes and the
disabled state on all four surfaces; selecting an option moves the shared thumb.

## Model selector placement

`web-ui/agent/ModelSelector.vue` aligns its dropdown to the trigger's right edge,
so the main dropdown extends to the left. The Gallery Session page uses the same
model selector.

## Controls above scrolling content

`IconButton` uses `variant="solid"` when content can scroll behind the control.
Its fill is the opaque `surface-raised` token; hover and pressed use the opaque
`surface-float` token. It retains the shared button outline and shadow. Default
buttons use the theme's regular button fill, which can be translucent in dark mode;
that fill is not suitable for controls over scrolling content. The session's
Scroll to bottom button uses the solid variant. Gallery `/control-layout` shows
it on all four surfaces, and Session States exercises it over a transcript.

## Product appearance

The product uses Regular density, Medium radius and Hairline shadows; those are
fixed. Tone (Ink or Warm), accent, the theme and the transcript text size are the
user's to choose on the General settings page.
`web-ui/theme/productAppearance.ts` defines the fixed axes and the catalogs of
tones and accents; `web-ui/styles/product-appearance.css` owns every token they
need in light and dark. The web composition root applies the axes and the gallery
exposes the same Demi preset, plus further tones for its own paradigms.

## Scroll regions and outlines

A scroll region clips both axes: `overflow-y: auto` makes `overflow-x` non-visible
too, so anything a child draws outside its box, such as a `ring` (a box-shadow), a
focus ring, or a corner badge, is cut off when the child sits flush with the
region's edge. Rules:

- Every scroll region carries horizontal padding of at least the widest outline
  drawn inside it. Where the layout wants the content flush, pair the padding with
  a matching negative margin (`-mx-1 px-1`) so the region grows instead of the
  content shrinking.
- Never fix a clipped outline on the child; fix the region.
- General product scroll regions use `ScrollArea`: the native bar is hidden and a thumb is
  drawn over the content at the right edge, so the bar takes no room and the
  content keeps symmetric padding whether or not it overflows. Padding for rings
  goes on the viewport (`viewportClass`); the region itself is a `min-h-0` flex
  item. The thumb shows on hover or while scrolling and can be dragged. The sidebar
  uses a native region with symmetric stable gutters; see
  [Sidebar layout](demi-next/sidebar-order.md#layout-motion-and-scrolling).
- The gallery audits this: `demiAuditClipping()` in the browser console, and
  automatically after each gallery navigation in development, lists every
  outlined element that a scroll region would clip.

## Control size families

Controls use aligned height families; each component exposes its applicable sizes: 36px
(`Button` lg, `TextInput` lg), 28px (`Button` md, `IconButton` md, `TextInput`
md, `Segmented` md, `Dropdown` md), 24px (`sm`) and 20px (`xs` buttons). A
surface picks one family per kind of control and keeps to it: all of its text
inputs one height, all of its buttons another. Isolated page forms (sign-in)
use 36px for both the fields and the submit button. In settings cards text
inputs are 28px, since a line of text wants that room, and buttons, icon
buttons, segmented and dropdown controls are 24px, since a bordered 28px icon
button reads heavy in a row. Two buttons of different heights in one card, or
two inputs, is a mistake. Chrome outside the cards (a dialog's search, the
narrow back row) stays at 28px, and a rail caption's action or a hover action
inside a 32px list row uses 20px. A bare input (no frame in any state) is the
exception: its hit area stretches to the row's content box, 28px in a compact
row and 32px in a regular one, so the value is easy to click into.

The gallery audits this: `demiAuditControlSizes()` in the browser console, and
automatically after each gallery navigation in development, lists every settings
card whose inputs or buttons mix families.

## Color transitions and `transparent`

Color transitions should retain their target hue while fading. For native
scrollbar thumbs, write the hidden color as the thumb's own hue at zero alpha,
`rgb(from <color> r g b / 0)`. Do not use the `transparent` keyword or a zero-percent
color mix for that hidden thumb, since interpolation from transparent black can
produce an unwanted dark midpoint. Verify new animated properties in the gallery
in light and dark modes rather than assuming the same interpolation behavior.

## Menus and typography

`MenuGroup` owns section headings; `MenuItem` owns labels, right-aligned metadata
in its `value` field, and status indicators. Metadata uses subtle foreground and
16px separation from the label. Model, host, project, and gallery menus share
these contracts.

Floating menus size to content with a 160px minimum, or 192px with search, and a
384px maximum capped by the viewport minus 32px. Labels truncate at the cap.
Shortcut space is reserved only for actual shortcuts. Conversation history uses
320px to fit titles and timestamps; embedded file and target lists fill their
panel. Other menus do not assign arbitrary fixed widths.

Interface text uses normal weight and macOS grayscale antialiasing. The shared
sign-in page places 36px fields and its submit button on the base surface, beside
a wide introduction area on the session surface. That area hides below tablet
width. Product theme options are defined above; gallery-only appearance controls
do not add settings to the product.
