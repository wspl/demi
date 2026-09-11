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
