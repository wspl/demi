<script setup lang="ts">
import { dismissToast, holdToasts, releaseToasts, toasts, type Toast as ShownToast } from '../infra/toast'
import Toast from './Toast.vue'

/** Runs the toast's action, which closes it. */
function act(toast: ShownToast) {
  dismissToast(toast.id)
  toast.action?.run()
}

const overlayMotion = {
  enterActiveClass: 'transition-[opacity,transform] duration-150 ease-out',
  leaveActiveClass: 'absolute transition-[opacity,transform] duration-150 ease-out',
  enterFromClass: 'translate-y-1 opacity-0',
  leaveToClass: 'translate-y-1 opacity-0',
  moveClass: 'transition-transform duration-150 ease-out',
}

/** Pin the leaving toast so the stack can reflow; moveClass then eases the rest down. */
function pinLeavingToast(el: Element) {
  const node = el as HTMLElement
  const parent = node.parentElement
  if (!parent)
    return
  const parentRect = parent.getBoundingClientRect()
  const rect = node.getBoundingClientRect()
  node.style.left = `${rect.left - parentRect.left}px`
  node.style.top = `${rect.top - parentRect.top}px`
  node.style.width = `${rect.width}px`
}
</script>

<template>
  <Teleport to="body">
    <TransitionGroup
      v-bind="overlayMotion"
      tag="div"
      class="pointer-events-none fixed inset-0 z-[55] flex flex-col items-end justify-end gap-2 p-4"
      @before-leave="pinLeavingToast"
    >
      <div
        v-for="toast in toasts"
        :key="toast.id"
        class="pointer-events-auto w-80 max-w-[calc(100vw-2rem)]"
        @pointerenter="holdToasts"
        @pointerleave="releaseToasts"
      >
        <Toast
          :title="toast.title"
          :message="toast.message"
          :tone="toast.tone"
          :action="toast.action?.label"
          @dismiss="dismissToast(toast.id)"
          @act="act(toast)"
        />
      </div>
    </TransitionGroup>
  </Teleport>
</template>
