<script setup lang="ts">
import { RightSmallLine } from '@mingcute/vue/right-small'

defineProps<{
  isOpen: boolean
}>()
</script>

<template>
  <span
    class="flex cursor-pointer items-center gap-0.5 rounded-md pl-2 pr-0.5 py-1 text-[12px] transition-colors"
    :class="isOpen ? 'bg-overlay/5 text-fg-muted' : 'text-fg-subtle hover:bg-hover hover:text-fg-muted'"
  >
    <slot />
    <RightSmallLine :size="16" class="rotate-90" />
  </span>
</template>
