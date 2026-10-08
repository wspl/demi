<script setup lang="ts">
/**
 * A change's line counts, as every changed file shows them: added in green,
 * removed in red, a zero left out, so a change of no lines shows nothing.
 * `compact` sets them 2px apart, as the strip's Change badge does. `zeros`
 * shows a zero too, as a change summed over files does on GitHub: +10 −0.
 */
defineProps<{ added: number; removed: number; compact?: boolean; zeros?: boolean }>()
</script>

<template>
  <span
    v-if="zeros || added > 0 || removed > 0"
    class="inline-flex shrink-0 font-mono text-[11px] tabular-nums"
    :class="compact ? 'gap-0.5' : 'gap-1'"
  >
    <span v-if="zeros || added > 0" class="text-on-success">+{{ added }}</span>
    <span v-if="zeros || removed > 0" class="text-on-danger">−{{ removed }}</span>
  </span>
</template>
