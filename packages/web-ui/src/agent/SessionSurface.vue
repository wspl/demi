<script setup lang="ts">
import { computed, provide, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import SessionBottomVeil from './SessionBottomVeil.vue'
import { sessionCoverKey } from './session-cover'

const bottomAreaRef = ref<HTMLDivElement>()
// Border-box: the dock's full footprint, its own padding included.
const { height: dockFootprint } = useElementSize(
  bottomAreaRef,
  { width: 0, height: 0 },
  { box: 'border-box' }
)
/** What stands over the dock's top, such as a permission card; the dock sets it. */
const coverHeight = ref(0)
provide(sessionCoverKey, coverHeight)
/**
 * What the transcript keeps clear at its foot: the dock and what stands over
 * it. A panel the dock opened stands on the dock alone, so the card covers
 * its lower part and it never moves when a card comes or goes.
 */
const dockHeight = computed(() => dockFootprint.value + coverHeight.value)

defineExpose({ dockHeight })
</script>

<template>
  <!-- The conversation reads in one column at most 50rem wide, its transcript
       and its dock alike; the space either side, --session-gutter, is padding
       inside the full-width scroller, so the scrollbar stays at the edge. -->
  <div
    class="relative h-full flex-1 overflow-hidden bg-surface [container-type:inline-size]"
    style="--session-gutter: max(0px, calc((100cqw - 50rem) / 2))"
  >
    <slot />
    <div
      v-if="$slots.overDock"
      class="pointer-events-none absolute inset-x-[calc(var(--session-gutter)+0.75rem)] z-20 min-h-0 pb-2"
      :style="{ bottom: `${dockFootprint}px`, height: '50%' }"
    >
      <!-- A panel here is in the column already: a transcript inside it takes no gutter again. -->
      <div class="relative h-full min-h-0 [--session-gutter:0px]">
        <slot name="overDock" />
      </div>
    </div>
    <!-- No stacking context of its own: the veil stays under a panel the dock opened, and the
         dock, with what stands over its top, above it. -->
    <div
      ref="bottomAreaRef"
      class="absolute bottom-0 left-0 right-0 px-[calc(var(--session-gutter)+0.75rem)] pb-3"
    >
      <SessionBottomVeil />
      <div class="relative z-30">
        <slot name="dock" />
      </div>
    </div>
  </div>
</template>
