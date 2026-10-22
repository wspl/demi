<script setup lang="ts">
/**
 * One section of a card, parted from the one above by the card's dividing
 * line and the room around it: 4 px under the line, 8 px under the section's
 * last row. It is the one owner of that rule, for the card's sections and for
 * the parts of one, such as the Cost section's profiles and cache hit rate,
 * whose line is subtler. The first section of a card has no line above it.
 */
withDefaults(defineProps<{
  /** The line above the section, which the card's first has not. */
  divided?: boolean
  /** A part of a section, parted by a subtler line than the card's sections are. */
  subtle?: boolean
}>(), {
  divided: true,
  subtle: false,
})
</script>

<template>
  <div class="pb-2" :class="divided ? ['border-t pt-1', subtle ? 'border-line-subtle' : 'border-line'] : 'pt-1'">
    <slot />
  </div>
</template>
