<script setup lang="ts">
import { ref } from 'vue'
import Tooltip from './Tooltip.vue'

/**
 * One line of text, cut with an ellipsis where it does not fit; while it is
 * cut, its tooltip shows it whole.
 */
defineProps<{
  text: string
}>()

const cut = ref(false)

// Measured as the pointer arrives, before the tooltip's delay has run.
function measure(event: MouseEvent): void {
  if (event.currentTarget instanceof HTMLElement)
    cut.value = event.currentTarget.scrollWidth > event.currentTarget.clientWidth
}
</script>

<template>
  <Tooltip tag="span" class="block min-w-0 truncate" :content="text" :disabled="!cut" @mouseenter="measure">
    {{ text }}
  </Tooltip>
</template>
