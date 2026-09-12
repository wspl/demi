<script setup lang="ts">
import { ref } from 'vue'
import ScrollableSessionSurface from '@demicodes/web-ui/agent/ScrollableSessionSurface.vue'
import type { MessageListBlock } from '@demicodes/web-ui/agent/pending-steers'
defineProps<{
  label?: string
  fill?: boolean
  blocks?: readonly MessageListBlock[]
}>()
const surface = ref<InstanceType<typeof ScrollableSessionSurface>>()
defineExpose({
  scrollToEnd: () => surface.value?.scrollToEnd(),
  get isAtBottom() {
    return surface.value?.isAtBottom ?? true
  },
})
</script>
<template>
  <ScrollableSessionSurface
    ref="surface"
    :label="label"
    :fill="fill"
    :blocks="blocks"
  >
    <slot />
    <template #dock="state"
      ><slot
        name="dock"
        v-bind="state"
    /></template>
    <template #panel><slot name="panel" /></template>
  </ScrollableSessionSurface>
</template>
