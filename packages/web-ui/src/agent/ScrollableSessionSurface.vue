<script setup lang="ts">
import { nextTick, onMounted, ref, watch } from 'vue'
import { COMPOSER_CLEARANCE_PX } from '@demicodes/web-ui/agent/composer-clearance'
import SessionSurface from '@demicodes/web-ui/agent/SessionSurface.vue'
import { isNearBottom } from '@demicodes/web-ui/composables/scroll-bottom'

withDefaults(
  defineProps<{
    label?: string
    fill?: boolean
  }>(),
  {
    fill: false,
  },
)

const scrollRef = ref<HTMLDivElement>()
const surfaceRef = ref<{ dockHeight: number }>()
const isAtBottom = ref(true)

function updateAtBottom(): void {
  const scroller = scrollRef.value
  isAtBottom.value = scroller ? isNearBottom(scroller) : true
}

function scrollToEnd(): void {
  nextTick(() => {
    const scroller = scrollRef.value
    if (!scroller) {
      return
    }
    scroller.scrollTop = scroller.scrollHeight
    isAtBottom.value = true
  })
}

onMounted(scrollToEnd)

watch(
  () => surfaceRef.value?.dockHeight,
  () => {
    nextTick(updateAtBottom)
  },
)

defineExpose({
  scrollToEnd,
  isAtBottom,
})
</script>

<template>
  <section :class="fill ? 'flex min-h-0 flex-1 flex-col' : 'flex flex-col'">
    <div
      v-if="label"
      class="mb-1.5 text-[13px] text-fg-muted"
    >
      {{ label }}
    </div>
    <div
      class="relative overflow-hidden bg-surface"
      :class="fill ? 'min-h-0 flex-1' : 'h-[20rem] rounded-lg border border-border'"
    >
      <SessionSurface ref="surfaceRef">
        <div
          ref="scrollRef"
          class="h-full overflow-y-auto pt-4"
          :style="{
            paddingBottom: `${(surfaceRef?.dockHeight ?? 0) + COMPOSER_CLEARANCE_PX}px`,
            scrollbarGutter: 'stable',
          }"
          @scroll="updateAtBottom"
        >
          <slot />
        </div>
        <template #dock>
          <slot
            name="dock"
            :show-scroll-to-bottom="!isAtBottom"
            :scroll-to-end="scrollToEnd"
          />
        </template>
        <template #overDock>
          <slot name="panel" />
        </template>
      </SessionSurface>
    </div>
  </section>
</template>
