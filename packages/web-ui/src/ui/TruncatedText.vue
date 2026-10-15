<script setup lang="ts">
import Tooltip from './Tooltip.vue'
import { isTextCut } from './truncation'

/**
 * One line of text, cut with an ellipsis at its end where it does not fit
 * (with `wrap`, lines of text, cut only at a word wider than the line);
 * while it is cut, its tooltip shows it whole. The Writing page's rule for
 * long text says which text may be cut and where. The slot dresses the
 * text, such as with marked matches; the tooltip still shows it plain.
 */
defineProps<{
  text: string
  /** Wraps at its words, and cuts only a word wider than the line: for a title that must be read whole where it can. */
  wrap?: boolean
}>()
</script>

<template>
  <Tooltip tag="span" class="block min-w-0"
    :class="wrap ? 'overflow-hidden text-ellipsis' : 'truncate'" :content="text" :show-if="isTextCut">
    <slot>{{ text }}</slot>
  </Tooltip>
</template>
