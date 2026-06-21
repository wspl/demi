<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useAttrs, watch } from 'vue'
import { useFloating, offset as offsetMiddleware, flip, shift, autoUpdate } from '@floating-ui/vue'
import type { Placement } from '@floating-ui/vue'

defineOptions({
  inheritAttrs: false,
})

const props = withDefaults(defineProps<{
  content: string | undefined
  placement?: Placement
  offset?: number
  disabled?: boolean
  openDelayMs?: number
  closeDelayMs?: number
  tag?: 'span' | 'div'
}>(), {
  placement: 'top',
  offset: 8,
  disabled: false,
  openDelayMs: 120,
  closeDelayMs: 0,
  tag: 'span',
})

const triggerRef = ref<HTMLElement | null>(null)
const floatingRef = ref<HTMLElement | null>(null)
const isOpen = ref(false)
const attrs = useAttrs()
const hasContent = computed(() => !!props.content?.trim())
const canShow = computed(() => hasContent.value && !props.disabled)

const { floatingStyles } = useFloating(triggerRef, floatingRef, {
  placement: computed(() => props.placement),
  middleware: computed(() => [
    offsetMiddleware(props.offset),
    flip(),
    shift({ padding: 8 }),
  ]),
  whileElementsMounted: autoUpdate,
  transform: false,
})

let openTimer: ReturnType<typeof setTimeout> | null = null
let closeTimer: ReturnType<typeof setTimeout> | null = null

function clearOpenTimer() {
  if (!openTimer) return
  clearTimeout(openTimer)
  openTimer = null
}

function clearCloseTimer() {
  if (!closeTimer) return
  clearTimeout(closeTimer)
  closeTimer = null
}

function clearTimers() {
  clearOpenTimer()
  clearCloseTimer()
}

function openNow() {
  if (!canShow.value) return
  isOpen.value = true
}

function closeNow() {
  isOpen.value = false
}

function scheduleOpen() {
  clearCloseTimer()
  clearOpenTimer()
  if (!canShow.value) return
  if (props.openDelayMs <= 0) {
    openNow()
    return
  }
  openTimer = setTimeout(() => {
    openTimer = null
    openNow()
  }, props.openDelayMs)
}

function scheduleClose() {
  clearOpenTimer()
  clearCloseTimer()
  if (props.closeDelayMs <= 0) {
    closeNow()
    return
  }
  closeTimer = setTimeout(() => {
    closeTimer = null
    closeNow()
  }, props.closeDelayMs)
}

watch(canShow, (nextCanShow) => {
  if (nextCanShow) return
  clearTimers()
  closeNow()
})

onBeforeUnmount(() => {
  clearTimers()
})
</script>

<template>
  <component
    :is="props.tag"
    ref="triggerRef"
    v-bind="attrs"
    @mouseenter="scheduleOpen"
    @mouseleave="scheduleClose"
    @focusin="scheduleOpen"
    @focusout="scheduleClose"
  >
    <slot />
  </component>

  <Teleport to="body">
    <Transition
      enter-active-class="transition-[opacity,transform] duration-120 ease-out"
      leave-active-class="transition-[opacity,transform] duration-100 ease-out"
      enter-from-class="opacity-0 scale-95"
      leave-to-class="opacity-0 scale-95"
    >
      <div
        v-if="isOpen && canShow"
        ref="floatingRef"
        class="pointer-events-none z-80 line-clamp-2 max-w-sm rounded-md bg-surface px-2 py-1 text-[11px] leading-4 text-fg ring-1 ring-line-subtle shadow-lg"
        :style="floatingStyles"
        role="tooltip"
      >
        {{ props.content }}
      </div>
    </Transition>
  </Teleport>
</template>
