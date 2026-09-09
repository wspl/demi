<script setup lang="ts">
import { ref, type ComponentPublicInstance } from 'vue'
import {
  TAB_TRANSITION,
  afterEnterTab,
  afterLeaveTab,
  beforeEnterTab,
  beforeLeaveTab,
  enterTab,
  leaveTab,
} from './tab-strip'

const el = ref<HTMLElement | null>(null)

function bindEl(instance: Element | ComponentPublicInstance | null): void {
  el.value =
    instance instanceof HTMLElement
      ? instance
      : instance && '$el' in instance
        ? instance.$el
        : null
}

defineExpose({ el })
</script>

<template>
  <TransitionGroup
    :ref="bindEl"
    :name="TAB_TRANSITION"
    tag="div"
    @before-enter="beforeEnterTab"
    @enter="enterTab"
    @after-enter="afterEnterTab"
    @enter-cancelled="afterEnterTab"
    @before-leave="beforeLeaveTab"
    @leave="leaveTab"
    @after-leave="afterLeaveTab"
    @leave-cancelled="afterLeaveTab"
  >
    <slot />
  </TransitionGroup>
</template>

<style>
.tabs-enter-active,
.tabs-leave-active {
  transition:
    width 200ms ease-out,
    margin 200ms ease-out,
    opacity 150ms ease;
}
.tabs-leave-active {
  pointer-events: none;
}
.tabs-enter-from,
.tabs-leave-to {
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .tabs-enter-active,
  .tabs-leave-active {
    transition: none;
  }
}
</style>
