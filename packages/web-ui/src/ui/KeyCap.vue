<script setup lang="ts">
import { computed } from 'vue'
import { shortcutCaps } from './shortcut'

/** A shortcut of the page's set as separate caps, one per key. */
const props = defineProps<{
  keys: string
}>()

const caps = computed(() => shortcutCaps(props.keys))
</script>

<template>
  <span class="inline-flex select-none items-center gap-1">
    <kbd
      v-for="(cap, index) in caps"
      :key="index"
      class="inline-flex h-5 min-w-5 items-center justify-center rounded-[4px] bg-surface-raised px-1 font-sans text-[11px] text-fg-muted ring-1 ring-line"
    >
      {{ cap }}
    </kbd>
  </span>
</template>
