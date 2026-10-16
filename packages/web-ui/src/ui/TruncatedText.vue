<script setup lang="ts">
import Tooltip from './Tooltip.vue'
import { isKeptStartCut, isTextCut } from './truncation'

/**
 * One line of text, cut with an ellipsis at its end where it does not fit
 * (with `wrap`, lines of text, cut only at a word wider than the line);
 * while it is cut, its tooltip shows it whole. The Writing page's rule for
 * long text says which text may be cut and where. The slot dresses the
 * text, such as with marked matches; the tooltip still shows it plain.
 *
 * With `keepsStart` it is never cut shorter than its first few characters
 * (3em, or the whole text when that is shorter): with less room than that it
 * shows nothing, as a crowded tab in Chrome keeps only its icon, rather than
 * a letter and an ellipsis no one can read. It is for a label beside a mark
 * that still names the thing, such as a pinned tab's title; the tooltip then
 * shows the text, as it does while the text is cut. A host whose own tooltip
 * covers more than the text, as a tab's covers the tab, turns this one off
 * (`noTooltip`) and asks `isKeptStartCut` when to show its own.
 */
defineProps<{
  text: string
  /** Wraps at its words, and cuts only a word wider than the line: for a title that must be read whole where it can. */
  wrap?: boolean
  /** Keeps its first few characters however narrow its row; one line only. */
  keepsStart?: boolean
  /** Shows no tooltip of its own, for a host whose tooltip covers more than the text. */
  noTooltip?: boolean
}>()
</script>

<template>
  <!-- The first box sets the widest it grows to: the text whole, though it would break between any two
       characters, so it asks for no least width. The second is one line that clips what wraps off it: the
       text's box starts at its least width, the text up to 3em, and grows to the line's; once the line is
       narrower than that, the box wraps off it, out of sight, away from the empty box before it, which
       holds the line's height, as a line's first box never wraps. In the text's box the text shows in
       the width the box has, contributing none of its own. -->
  <Tooltip v-if="keepsStart" tag="span" class="grid min-w-0" :content="text" :disabled="noTooltip" :show-if="isKeptStartCut">
    <span aria-hidden="true" class="invisible col-start-1 row-start-1 h-0 overflow-hidden break-all">{{ text }}</span>
    <span class="col-start-1 row-start-1 flex h-[1lh] min-w-0 flex-wrap overflow-hidden">
      <span aria-hidden="true" class="h-[1lh] w-0" />
      <span class="grid grow">
        <span aria-hidden="true" class="invisible col-start-1 row-start-1 h-0 max-w-[3em] overflow-clip whitespace-nowrap">{{ text }}</span>
        <span class="col-start-1 row-start-1 w-0 min-w-full truncate"><slot>{{ text }}</slot></span>
      </span>
    </span>
  </Tooltip>
  <Tooltip v-else tag="span" class="block min-w-0"
    :class="wrap ? 'overflow-hidden text-ellipsis' : 'truncate'" :content="text" :disabled="noTooltip" :show-if="isTextCut">
    <slot>{{ text }}</slot>
  </Tooltip>
</template>
