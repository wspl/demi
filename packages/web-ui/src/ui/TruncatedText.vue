<script setup lang="ts">
import Tooltip from './Tooltip.vue'
import { isTextCut } from './truncation'

/**
 * One line of text, cut with an ellipsis at its end where it does not fit;
 * while it is cut, its tooltip shows it whole. The Writing page's rule for
 * long text says which text may be cut and where.
 */
defineProps<{
  text: string
}>()
</script>

<template>
  <Tooltip tag="span" class="block min-w-0 truncate" :content="text" :show-if="isTextCut">
    {{ text }}
  </Tooltip>
</template>
