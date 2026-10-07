<script setup lang="ts">
import { dismissToast, holdToasts, releaseToasts, toasts, type Toast as ShownToast } from '../infra/toast'
import Toast from './Toast.vue'
import { APP_BAR_PX } from './app-bar'

/**
 * The toasts stand in the top-right corner, below the bar atop the page, as
 * macOS places its notifications: never over the composer or the actions at
 * the bottom of a pane. The newest stands nearest the corner.
 */
const props = withDefaults(defineProps<{
  /** How far down the page's top bars reach, in px: the toasts stand below them. */
  below?: number
}>(), { below: APP_BAR_PX })

/** Runs the toast's action, which closes it. */
function act(toast: ShownToast) {
  dismissToast(toast.id)
  toast.action?.run()
}

const overlayMotion = {
  enterActiveClass: 'transition-[opacity,transform] duration-150 ease-out',
  leaveActiveClass: 'absolute transition-[opacity,transform] duration-150 ease-out',
  enterFromClass: '-translate-y-1 opacity-0',
  leaveToClass: '-translate-y-1 opacity-0',
  moveClass: 'transition-transform duration-150 ease-out',
}

/** Pin the leaving toast so the stack can reflow; moveClass then eases the rest up. */
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
      class="pointer-events-none fixed right-3 z-[55] flex flex-col-reverse items-end gap-2"
      :style="{ top: `${props.below + 8}px` }"
      @before-leave="pinLeavingToast"
    >
      <div
        v-for="toast in toasts"
        :key="toast.id"
        class="pointer-events-auto w-80 max-w-[calc(100vw-1.5rem)]"
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
