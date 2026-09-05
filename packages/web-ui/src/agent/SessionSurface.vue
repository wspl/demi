<script setup lang="ts">
import { ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import SessionBottomVeil from './SessionBottomVeil.vue'

const bottomAreaRef = ref<HTMLDivElement>()
// Border-box: the scroller pads by the dock's full footprint, its own padding included.
const { height: dockHeight } = useElementSize(bottomAreaRef, { width: 0, height: 0 }, { box: 'border-box' })

defineExpose({ dockHeight })
</script>

<template>
  <div class="relative h-full flex-1 overflow-hidden bg-surface">
    <slot />
    <div ref="bottomAreaRef" class="absolute bottom-0 left-0 right-0 z-10 px-3 pb-3">
      <SessionBottomVeil />
      <div class="relative z-10">
        <slot name="dock" />
      </div>
    </div>
  </div>
</template>
