<script setup lang="ts">
import { computed } from 'vue'

/** A key combination as separate caps: "⌘⇧P" or "ctrl+k" both render one cap per key. */
const props = defineProps<{
  keys: string
}>()

const caps = computed(() => {
  if (props.keys.includes('+')) return props.keys.split('+').map((key) => key.trim()).filter(Boolean)
  return Array.from(props.keys)
})
</script>

<template>
  <span class="inline-flex select-none items-center gap-0.5">
    <kbd
      v-for="(cap, index) in caps"
      :key="index"
      class="inline-flex h-5 min-w-5 items-center justify-center rounded-[4px] bg-surface-raised px-1 font-sans text-[11px] text-fg-muted ring-1 ring-line"
    >
      {{ cap }}
    </kbd>
  </span>
</template>
